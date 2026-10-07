import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { LookupRequest } from "../../ipc/bindings/LookupRequest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { CreateContext } from "../../ipc/bindings/CreateContext";
import type { TableDef } from "../../ipc/bindings/TableDef";
import type { Risk } from "../../ipc/bindings/Risk";
import { createDesignerController, planRequest, type AppliedInfo, type DesignerControllerDeps } from "./designer-controller";
import { failedText } from "./errors";

const schema = (p: Partial<{ name: string; owner: string | null; comment: string | null }> = {}): ObjectDef =>
  ({ kind: "schema", name: "app", owner: "alice", comment: null, ...p });
const view = (p: Partial<PlanView> = {}): PlanView => ({
  statements: [{ sql: "ALTER SCHEMA app RENAME TO x", risk: p.risk ?? "safe", transactional: true, note: null }],
  warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...p,
});
const risky = (risk: Risk, hash = "hr") => view({ risk, hash });
const applied = (created: ObjectRef | null = null, statements = 1): ApplyOutcome => ({ outcome: "applied", statements, elapsedMs: 3, created });
const err = (code: string, message = `m:${code}`, detail: string | null = null) => ({ code, message, detail, hint: null });
const OBJ: ObjectRef = { kind: "schema", oid: 100 };

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function setup(extra: Partial<DesignerControllerDeps> = {}) {
  let ids = 0;
  let clock = 1_000_000;
  const servers: Record<string, { name: string; locked: boolean; production: boolean }> = {
    S1: { name: "prod-1", locked: false, production: false },
    S2: { name: "other", locked: false, production: false },
  };
  const api = {
    ddlDefaults: vi.fn(async (_s: string, _db: number, _k: ObjectKind, _c: CreateContext): Promise<ObjectDef> => schema({ name: "", owner: null })),
    ddlLoad: vi.fn(async (_s: string, _db: number, _o: ObjectRef): Promise<ObjectDef> => schema()),
    ddlLookup: vi.fn(async (_s: string, _db: number, _r: LookupRequest): Promise<LookupItem[]> => [{ value: "alice", detail: null }]),
    ddlPreview: vi.fn(async (_s: string, _db: number, _r: PlanRequest): Promise<PlanView> => view()),
    ddlApply: vi.fn(async (_s: string, _db: number, _r: PlanRequest, _o: ApplyOptions): Promise<ApplyOutcome> => applied()),
    ddlCancel: vi.fn(async (_id: string) => null),
  };
  const notify = vi.fn();
  const refreshNode = vi.fn();
  const deps: DesignerControllerDeps = {
    api: api as unknown as DesignerControllerDeps["api"],
    notify,
    serverInfo: (id) => servers[id] ?? null,
    refreshNode,
    newId: () => `A${++ids}`,
    now: () => clock,
    ...extra,
  };
  const ctl = createDesignerController(deps);
  return { ctl, api, notify, refreshNode, servers, tick: (ms: number) => { clock += ms; } };
}
type S = ReturnType<typeof setup>;

/** Bekleyen sözleri ve sıfır gecikmeli işleri boşalt. */
const flush = () => vi.advanceTimersByTimeAsync(0);
/** Önizleme gecikmesi (150 ms) + yanıt. */
const settle = () => vi.advanceTimersByTimeAsync(150);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

async function openAlter(s: S, tabId = "T1", serverId = "S1", object: ObjectRef = OBJ) {
  await s.ctl.openAlter({ tabId, serverId, databaseOid: 5, object, refreshNodeId: "N-parent" });
  return tabId;
}
async function openCreate(s: S, tabId = "C1") {
  await s.ctl.openCreate({
    tabId, serverId: "S1", databaseOid: 5, create: { kind: "schema", context: { schemaOid: null, tableOid: null } }, refreshNodeId: "N-coll",
  });
  return tabId;
}
const rename = (name: string) => (d: ObjectDef) => {
  if (d.kind === "schema") d.name = name; // yerinde değişiklik: denetleyici kopya verir
};
/** Alter sekmesi + düzenleme + önizleme gelmiş (kapı hazır). */
async function readyToApply(s: S, v: PlanView = view()) {
  const id = await openAlter(s);
  s.api.ddlPreview.mockResolvedValue(v);
  s.ctl.edit(id, rename("yeni"));
  await settle();
  return id;
}

describe("designer-controller açılış", () => {
  it("openCreate: defaults yüklenir, dirty=false, önizleme zamanlanmaz", async () => {
    const s = setup();
    const id = await openCreate(s);
    expect(s.api.ddlDefaults).toHaveBeenCalledWith("S1", 5, "schema", { schemaOid: null, tableOid: null });
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ mode: "create", kind: "schema", status: "ready", original: null, object: null, dirty: false, pending: false });
    expect(m.draft).toEqual(schema({ name: "", owner: null }));
    expect(m.serverName).toBe("prod-1");
    await vi.advanceTimersByTimeAsync(5000);
    expect(s.api.ddlPreview).not.toHaveBeenCalled();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
    // Elle önizleme: create isteği (concurrently yalnızca index'te)
    s.ctl.previewNow(id);
    await flush();
    expect(s.api.ddlPreview).toHaveBeenCalledExactlyOnceWith("S1", 5, { op: "create", def: schema({ name: "", owner: null }), concurrently: false });
    // Varsayılana dönen düzenleme kirli değil
    s.ctl.edit(id, rename("x"));
    expect(s.ctl.isDirty(id)).toBe(true);
    s.ctl.edit(id, rename(""));
    expect(s.ctl.isDirty(id)).toBe(false);
  });

  it("openAlter: original/draft ayrı derin kopyalar; old her zaman original'in kopyası (düzenlemeler original'e sızmaz)", async () => {
    const s = setup();
    const loaded = schema({ comment: "ilk" });
    s.api.ddlLoad.mockResolvedValueOnce(loaded);
    const id = await openAlter(s);
    expect(s.api.ddlLoad).toHaveBeenCalledWith("S1", 5, OBJ);
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ mode: "alter", status: "ready", object: OBJ, dirty: false });
    expect(m.original).toEqual(loaded);
    expect(m.draft).toEqual(loaded);
    expect(m.draft).not.toBe(m.original);
    const original = m.original!;
    s.ctl.edit(id, rename("yeni"));
    s.ctl.edit(id, (d) => ({ ...d, comment: "ikinci" }));
    const m2 = s.ctl.model(id)!;
    expect(m2.original).toBe(original);
    expect(original).toEqual(schema({ comment: "ilk" }));
    expect(loaded).toEqual(schema({ comment: "ilk" }));
    expect(m2.draft).toEqual(schema({ name: "yeni", comment: "ikinci" }));
    expect(m2.dirty).toBe(true);
    await settle();
    const req = s.api.ddlPreview.mock.calls[0]![2];
    expect(req).toEqual({ op: "alter", object: OBJ, old: schema({ comment: "ilk" }), new: schema({ name: "yeni", comment: "ikinci" }) });
    if (req.op !== "alter") throw new Error("alter bekleniyordu");
    expect(req.old).not.toBe(m2.draft);
    // İstek nesnesini değiştirmek original'i etkilemez
    (req.old as { name: string }).name = "bozuk";
    expect(s.ctl.model(id)!.original).toEqual(schema({ comment: "ilk" }));
    // Apply isteği de object + old = original taşır
    s.ctl.setConfirm(id, { checked: true });
    await s.ctl.apply(id);
    const areq = s.api.ddlApply.mock.calls[0]![2];
    expect(areq).toEqual({ op: "alter", object: OBJ, old: schema({ comment: "ilk" }), new: schema({ name: "yeni", comment: "ikinci" }) });
  });

  it("openAlter yükleme hataları: not_found/unknown_server → gone, bağlantı → disconnected, diğer → error", async () => {
    const s = setup();
    s.api.ddlLoad.mockRejectedValueOnce(err("not_found"));
    await openAlter(s, "A");
    expect(s.ctl.model("A")).toMatchObject({ status: "gone", error: tdes.status.gone, draft: null });
    s.api.ddlLoad.mockRejectedValueOnce(err("unknown_server"));
    await openAlter(s, "B");
    expect(s.ctl.model("B")).toMatchObject({ status: "gone", error: tdes.status.serverGone });
    s.api.ddlLoad.mockRejectedValueOnce(err("connection_lost"));
    await openAlter(s, "C");
    expect(s.ctl.model("C")).toMatchObject({ status: "disconnected", error: "m:connection_lost" });
    s.api.ddlLoad.mockRejectedValueOnce(err("query", "izin yok", "permission denied"));
    await openAlter(s, "D");
    expect(s.ctl.model("D")).toMatchObject({ status: "error", error: "izin yok — permission denied", draft: null });
    expect(s.ctl.peekApplyGate("D")).toEqual({ kind: "blocked", reason: "loading" });
    expect(s.ctl.peekApplyGate("C")).toEqual({ kind: "blocked", reason: "disconnected" });
    // Hata durumunda düzenleme/önizleme yok
    s.ctl.edit("D", rename("x"));
    s.ctl.previewNow("D");
    await settle();
    expect(s.api.ddlPreview).not.toHaveBeenCalled();
    // Yeniden dene
    await s.ctl.reload("D");
    expect(s.ctl.model("D")).toMatchObject({ status: "ready", error: null });
    // gone sekme yeniden yüklenmez
    const calls = s.api.ddlLoad.mock.calls.length;
    await s.ctl.reload("A");
    expect(s.api.ddlLoad.mock.calls.length).toBe(calls);
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("aynı sekme kimliğiyle ikinci açılış yok sayılır", async () => {
    const s = setup();
    await openAlter(s);
    await openAlter(s);
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(1);
  });
});

describe("designer-controller önizleme", () => {
  it("edit: version++, dirty, 150 ms sonra tek ddlPreview; hızlı üç edit → tek istek", async () => {
    const s = setup();
    const id = await openAlter(s);
    const v0 = s.ctl.model(id)!.version;
    s.ctl.edit(id, rename("a"));
    s.ctl.edit(id, rename("ab"));
    s.ctl.edit(id, rename("abc"));
    const m = s.ctl.model(id)!;
    expect(m.version).toBe(v0 + 3);
    expect(m.pending).toBe(true);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "pending" });
    await vi.advanceTimersByTimeAsync(149);
    expect(s.api.ddlPreview).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    expect((s.api.ddlPreview.mock.calls[0]![2] as { new: { name: string } }).new.name).toBe("abc");
    const m2 = s.ctl.model(id)!;
    expect(m2).toMatchObject({ pending: false, viewFor: v0 + 3, previewError: null });
    expect(m2.view).toEqual(view());
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    // Original'e dönüş: kirli değil
    s.ctl.edit(id, rename("app"));
    expect(s.ctl.isDirty(id)).toBe(false);
  });

  it("bayat önizleme: yanıt geldiğinde version ilerlediyse viewIsCurrent=false, apply kapısı 'stale'", async () => {
    const s = setup();
    const id = await openAlter(s);
    s.ctl.edit(id, rename("a"));
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    // Yeni sürümün önizlemesi uçuşta; eski görünüm v1'e ait kalır
    const d = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(d.promise);
    s.ctl.edit(id, rename("b"));
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "pending" });
    d.reject(err("io", "geçici"));
    await flush();
    const m = s.ctl.model(id)!;
    expect(m.viewFor).toBeLessThan(m.version);
    expect(m).toMatchObject({ pending: false, previewError: "geçici" });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    // Uçuştaki eski önizleme, yeni düzenleme sonrası gelirse yok sayılır (en son kazanır)
    const old = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(old.promise);
    s.ctl.edit(id, rename("c"));
    await settle();
    s.api.ddlPreview.mockResolvedValueOnce(view({ hash: "yeni" }));
    s.ctl.edit(id, rename("d"));
    await settle();
    old.resolve(view({ hash: "eski" }));
    await flush();
    expect(s.ctl.model(id)!.view?.hash).toBe("yeni");
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
  });

  it("önizleme bağlantı hatası → disconnected; not_found → gone", async () => {
    const s = setup();
    const id = await openAlter(s);
    s.api.ddlPreview.mockRejectedValueOnce(err("not_connected"));
    s.ctl.edit(id, rename("x"));
    await settle();
    expect(s.ctl.model(id)).toMatchObject({ status: "disconnected", pending: false });
    // Salt okunur: düzenleme ve önizleme yok
    const v = s.ctl.model(id)!.version;
    s.ctl.edit(id, rename("y"));
    s.ctl.previewNow(id);
    await settle();
    expect(s.ctl.model(id)!.version).toBe(v);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    const id2 = await openAlter(s, "T2");
    s.api.ddlPreview.mockRejectedValueOnce(err("not_found"));
    s.ctl.edit(id2, rename("x"));
    await settle();
    expect(s.ctl.model(id2)).toMatchObject({ status: "gone", error: tdes.status.gone });
  });

  it("yeni hash'li görünüm onayı sıfırlar; aynı hash korur", async () => {
    const s = setup();
    const id = await readyToApply(s, risky("destructive", "h1"));
    s.ctl.setConfirm(id, { checked: true });
    s.ctl.edit(id, rename("yeni2"));
    s.ctl.edit(id, rename("yeni")); // aynı plan → aynı hash
    await settle();
    expect(s.ctl.model(id)!.confirm.checked).toBe(true);
    s.api.ddlPreview.mockResolvedValueOnce(risky("destructive", "h2"));
    s.ctl.edit(id, rename("baska"));
    await settle();
    expect(s.ctl.model(id)!.confirm).toEqual({ checked: false, typedName: "" });
  });
});

describe("designer-controller uygulama kapısı", () => {
  it("apply: kapı ready değilken ddlApply çağrılmaz (invalid/empty/stale/pending/locked/disconnected)", async () => {
    const s = setup();
    // stale (önizleme yok)
    const id = await openAlter(s);
    s.ctl.edit(id, rename("x"));
    await s.ctl.apply(id); // pending
    s.ctl.model(id);
    await settle();
    // invalid
    s.api.ddlPreview.mockResolvedValueOnce(view({ errors: [{ path: "name", message: "boş" }], empty: true }));
    s.ctl.edit(id, rename(""));
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "invalid" });
    await s.ctl.apply(id);
    // empty
    s.api.ddlPreview.mockResolvedValueOnce(view({ statements: [], empty: true }));
    s.ctl.edit(id, rename("app"));
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "empty" });
    await s.ctl.apply(id);
    // locked: kayıt kilitli, serverChanged henüz gelmedi → apply anında yeniden hesaplanır
    s.ctl.edit(id, rename("z"));
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    s.servers.S1!.locked = true;
    await s.ctl.apply(id);
    s.ctl.serverChanged("S1");
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "locked" });
    await s.ctl.apply(id);
    // disconnected
    s.servers.S1!.locked = false;
    s.ctl.serverChanged("S1");
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "disconnected" });
    await s.ctl.apply(id);
    // sunucu kaydı yok (apply anında) → gönderilmez
    const id2 = await openAlter(s, "T2", "S2");
    s.ctl.edit(id2, rename("q"));
    await settle();
    delete s.servers.S2;
    await s.ctl.apply(id2);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    expect(s.notify).not.toHaveBeenCalled();
    expect(s.ctl.model(id)!.gateError).toBeNull();
  });

  it("apply: needsConfirm onay kutusu olmadan çağrılmaz; kutu + canSubmit ile confirmed=true gider", async () => {
    const s = setup();
    const id = await readyToApply(s, risky("destructive", "hd"));
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsConfirm" });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setConfirm(id, { checked: true });
    s.ctl.setLockTimeout(id, 2000);
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    expect(s.api.ddlApply.mock.calls[0]![3]).toEqual({
      applyId: "A1", lockTimeoutMs: 2000, expectedHash: "hd", confirmed: true, confirmedProduction: false,
    });
  });

  it("apply: production+data_loss ad yazılmadan çağrılmaz; doğru adla confirmedProduction=true", async () => {
    const s = setup();
    s.servers.S1!.production = true;
    const id = await readyToApply(s, risky("data_loss", "hx"));
    expect(s.ctl.model(id)!.production).toBe(true);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    s.ctl.setConfirm(id, { checked: true });
    await s.ctl.apply(id);
    s.ctl.setConfirm(id, { typedName: "Prod-1" });
    await s.ctl.apply(id);
    s.ctl.setConfirm(id, { checked: false, typedName: "prod-1" });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setConfirm(id, { checked: true, typedName: "  prod-1 " });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    expect(s.api.ddlApply.mock.calls[0]![3]).toMatchObject({ confirmed: true, confirmedProduction: true, expectedHash: "hx" });
  });

  it("apply anında kayıttaki üretim etiketi de dikkate alınır (serverChanged gelmeden)", async () => {
    const s = setup();
    const id = await readyToApply(s, risky("data_loss"));
    s.ctl.setConfirm(id, { checked: true });
    s.servers.S1!.production = true;
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
  });

  it("apply çift tıklama: uçuştayken ikinci apply ikinci istek üretmez; uygulama sırasında düzenleme/önizleme yok sayılır", async () => {
    const s = setup();
    const id = await readyToApply(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p1 = s.ctl.apply(id);
    const p2 = s.ctl.apply(id);
    expect(s.ctl.model(id)).toMatchObject({ applying: true, applyId: "A1" });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "busy" });
    const before = s.ctl.model(id)!;
    s.ctl.edit(id, rename("baska"));
    s.ctl.previewNow(id);
    s.ctl.setConcurrently(id, true);
    await settle();
    expect(s.ctl.model(id)!.version).toBe(before.version);
    expect(s.ctl.model(id)!.draft).toBe(before.draft);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    await p2;
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    d.resolve(applied());
    await p1;
    expect(s.ctl.model(id)!.applying).toBe(false);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
  });
});

describe("designer-controller uygulama sonuçları", () => {
  it("Applied(create, created=ref): mode alter'a geçer, yeniden yüklenir, refreshNode(refreshNodeId) çağrılır, toast", async () => {
    const s = setup();
    const id = await openCreate(s);
    s.ctl.edit(id, rename("yeni"));
    await settle();
    const created: ObjectRef = { kind: "schema", oid: 77 };
    s.api.ddlApply.mockResolvedValueOnce(applied(created, 2));
    s.api.ddlLoad.mockResolvedValueOnce(schema({ name: "yeni", owner: "bob" }));
    await s.ctl.apply(id);
    await flush();
    expect(s.api.ddlApply.mock.calls[0]![2]).toEqual({ op: "create", def: schema({ name: "yeni", owner: null }), concurrently: false });
    expect(s.api.ddlLoad).toHaveBeenCalledExactlyOnceWith("S1", 5, created);
    expect(s.refreshNode).toHaveBeenCalledExactlyOnceWith("N-coll");
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.apply.applied(2), "success");
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ mode: "alter", object: created, status: "ready", dirty: false, applying: false, view: null });
    expect(m.original).toEqual(schema({ name: "yeni", owner: "bob" }));
    expect(m.draft).toEqual(m.original);
    expect(m.outcome).toEqual(applied(created, 2));
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
  });

  it("Applied(alter, created=yeni ref): indeks yeniden kuruldu → yeni oid ile yüklenir, sekme ready; created yoksa nesne korunur", async () => {
    const s = setup();
    const idx = (name: string): ObjectDef => ({
      kind: "index", schema: "public", name, table: "t", method: "btree", unique: false, columns: [], include: [],
      tablespace: null, fillfactor: null, whereClause: null, comment: null, constraint: null, valid: true,
    } as unknown as ObjectDef);
    const old: ObjectRef = { kind: "index", oid: 42 };
    s.api.ddlLoad.mockResolvedValueOnce(idx("i"));
    await s.ctl.openAlter({ tabId: "I", serverId: "S1", databaseOid: 5, object: old, refreshNodeId: "N-idx" });
    s.ctl.edit("I", (d) => ({ ...d, method: "hash" }) as ObjectDef);
    await settle();
    const rebuilt: ObjectRef = { kind: "index", oid: 99 };
    s.api.ddlApply.mockResolvedValueOnce(applied(rebuilt, 2));
    s.api.ddlLoad.mockResolvedValueOnce(idx("i"));
    await s.ctl.apply("I");
    await flush();
    expect(s.api.ddlLoad).toHaveBeenLastCalledWith("S1", 5, rebuilt);
    expect(s.ctl.model("I")).toMatchObject({ mode: "alter", object: rebuilt, status: "ready", needsReload: false });

    // created yok: aynı nesneyle yüklenir
    s.ctl.edit("I", (d) => ({ ...d, name: "j" }) as ObjectDef);
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied(null));
    s.api.ddlLoad.mockResolvedValueOnce(idx("j"));
    await s.ctl.apply("I");
    await flush();
    expect(s.api.ddlLoad).toHaveBeenLastCalledWith("S1", 5, rebuilt);
    expect(s.ctl.model("I")).toMatchObject({ object: rebuilt, status: "ready" });
  });

  it("Applied(create, created=null): mod create kalır, durum ready, outcome saklanır, yeniden yükleme yok", async () => {
    const s = setup();
    const id = await openCreate(s);
    s.ctl.edit(id, rename("yeni"));
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied(null));
    await s.ctl.apply(id);
    expect(s.api.ddlLoad).not.toHaveBeenCalled();
    expect(s.refreshNode).toHaveBeenCalledWith("N-coll");
    expect(s.ctl.model(id)).toMatchObject({ mode: "create", status: "ready", dirty: false, outcome: applied(null), view: null });
    // "Tamam" durumu: düzenleme, önizleme, uygulama yok (arayüz Kapat sunar)
    const before = s.ctl.model(id)!;
    s.ctl.edit(id, rename("ikinci"));
    s.ctl.previewNow(id);
    await settle();
    await s.ctl.apply(id);
    expect(s.ctl.model(id)!.draft).toBe(before.draft);
    expect(s.ctl.model(id)!.outcome).toEqual(applied(null));
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
  });

  it("Applied(alter) sonrası yeniden yükleme genel hatası: ready + error + needsReload; başarılı reload temizler", async () => {
    const s = setup();
    const id = await readyToApply(s);
    expect(s.ctl.model(id)!.needsReload).toBe(false);
    s.api.ddlLoad.mockRejectedValueOnce(err("query", "geçici"));
    await s.ctl.apply(id);
    await flush();
    expect(s.ctl.model(id)).toMatchObject({ status: "ready", error: "geçici", needsReload: true, dirty: false });
    expect(s.ctl.model(id)!.original).toEqual(schema());
    await s.ctl.reload(id);
    expect(s.ctl.model(id)).toMatchObject({ status: "ready", error: null, needsReload: false });
  });

  it("Applied(alter): reload + refreshNode; dirty=false; bayat önizleme düşer; lookup önbelleği temizlenir", async () => {
    const s = setup();
    const id = await readyToApply(s);
    await s.ctl.lookup(id, { kind: "schemas" });
    s.api.ddlLoad.mockResolvedValueOnce(schema({ name: "yeni" }));
    await s.ctl.apply(id);
    await flush();
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(2);
    expect(s.refreshNode).toHaveBeenCalledExactlyOnceWith("N-parent");
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ dirty: false, status: "ready", view: null, pending: false });
    expect(m.original).toEqual(schema({ name: "yeni" }));
    await s.ctl.lookup(id, { kind: "schemas" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(2);
  });

  it("refreshNode hatası yutulur", async () => {
    const s = setup();
    s.refreshNode.mockImplementation(() => Promise.reject(new Error("ağaç")));
    const id = await readyToApply(s);
    await s.ctl.apply(id);
    await flush();
    expect(s.ctl.model(id)!.status).toBe("ready");
  });

  it("Failed{failedIndex,committed>0}: outcome saklanır, taslak korunur, uyarı toast'ı; sqlstate 55P03 kullanıcı metni", async () => {
    const s = setup();
    const id = await readyToApply(s);
    const draft = s.ctl.model(id)!.draft;
    const failed: ApplyOutcome = {
      outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "55P03", position: null, error: err("query", "canceling statement"),
    };
    s.api.ddlApply.mockResolvedValueOnce(failed);
    await s.ctl.apply(id);
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ outcome: failed, applying: false, applyId: null, dirty: true });
    expect(m.draft).toBe(draft);
    expect(m.applyError).toBe(tdes.apply.failed(1, tdes.apply.lockTimeout));
    expect(m.applyError).toBe(failedText(failed as Extract<ApplyOutcome, { outcome: "failed" }>));
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.apply.committed(1), "error");
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(1);
    // Kısmen kalıcı: hemen yeniden yükleme ipucu; bayat temele karşı önizleme ve otomatik gönderim yok
    expect(m).toMatchObject({ gateError: tdes.gate.partial, needsReload: true, view: null });
    await settle();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
    // committed=0 → toast yok
    const id2 = await openAlter(s, "T2");
    s.ctl.edit(id2, rename("k"));
    await settle();
    s.api.ddlApply.mockResolvedValueOnce({ ...failed, committed: 0, sqlstate: "42P07", error: err("query", "zaten var", "d") });
    await s.ctl.apply(id2);
    expect(s.notify).toHaveBeenCalledTimes(1);
    expect(s.ctl.model(id2)).toMatchObject({ applyError: tdes.apply.failed(1, "zaten var — d"), gateError: null, needsReload: false });
    // committed=0: aynı hash'le gönderim yok, önizleme yenilenir
    await flush();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(3);
    // Düzenleme sonucu temizler
    s.ctl.edit(id2, rename("l"));
    expect(s.ctl.model(id2)).toMatchObject({ outcome: null, applyError: null });
  });

  it("Err plan_changed: gateError + otomatik reload YOK (dirty iken); reload yalnızca force ile (karar: hiçbir durumda otomatik değil)", async () => {
    const s = setup();
    const id = await readyToApply(s);
    s.api.ddlApply.mockRejectedValueOnce(err("plan_changed"));
    await s.ctl.apply(id);
    await settle();
    const m = s.ctl.model(id)!;
    expect(m).toMatchObject({ gateError: tdes.gate.planChanged, needsReload: true, dirty: true, applying: false, view: null, status: "ready" });
    expect((m.draft as { name: string }).name).toBe("yeni");
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(1);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1); // eski original'e karşı önizleme yeniden istenmez
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
    // dirty iken force'suz reload hiçbir şey yapmaz
    await s.ctl.reload(id);
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(1);
    s.api.ddlLoad.mockResolvedValueOnce(schema({ comment: "başkası" }));
    await s.ctl.reload(id, { force: true });
    expect(s.ctl.model(id)).toMatchObject({ dirty: false, gateError: null, draft: schema({ comment: "başkası" }) });
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("Err invalid_def → önizleme yeniden istenir; Err locked → locked=true; Err not_found → gone; bağlantı kodları → disconnected", async () => {
    const s = setup();
    let id = await readyToApply(s);
    s.api.ddlApply.mockRejectedValueOnce(err("invalid_def"));
    await s.ctl.apply(id);
    expect(s.ctl.model(id)!.gateError).toBe(tdes.gate.invalidDef);
    await flush();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(2);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });

    s.api.ddlApply.mockRejectedValueOnce(err("locked"));
    await s.ctl.apply(id);
    expect(s.ctl.model(id)!.locked).toBe(true);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "locked" });

    s.ctl.close(id);
    id = await readyToApply(s);
    s.api.ddlApply.mockRejectedValueOnce(err("not_found"));
    await s.ctl.apply(id);
    expect(s.ctl.model(id)).toMatchObject({ status: "gone", error: tdes.status.gone, applying: false });

    s.ctl.close(id);
    id = await readyToApply(s);
    s.api.ddlApply.mockRejectedValueOnce(err("connection_lost"));
    await s.ctl.apply(id);
    expect(s.ctl.model(id)).toMatchObject({ status: "disconnected", applying: false });

    s.ctl.close(id);
    id = await readyToApply(s);
    s.api.ddlApply.mockRejectedValueOnce(err("query", "olmadı", "ayrıntı"));
    await s.ctl.apply(id);
    expect(s.ctl.model(id)).toMatchObject({ status: "ready", gateError: "olmadı — ayrıntı" });
    expect(s.notify).not.toHaveBeenCalled();
    expect(s.refreshNode).not.toHaveBeenCalled();
  });
});

describe("designer-controller yaşam döngüsü", () => {
  it("serverSessionsClosed: sekmeler disconnected/gone, zamanlayıcı iptal, uçuştaki apply için ddlCancel", async () => {
    const s = setup();
    const a = await openAlter(s, "A");
    const b = await openAlter(s, "B");
    const other = await openAlter(s, "O", "S2");
    s.ctl.edit(a, rename("x")); // zamanlayıcıda
    s.ctl.edit(b, rename("y"));
    await settle();
    s.api.ddlCancel.mockRejectedValueOnce(err("not_found"));
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.ctl.apply(b);
    const previews = s.api.ddlPreview.mock.calls.length;
    s.ctl.edit(a, rename("x2"));
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.api.ddlCancel).toHaveBeenCalledExactlyOnceWith("A1");
    await settle();
    expect(s.api.ddlPreview.mock.calls.length).toBe(previews);
    expect(s.ctl.model(a)).toMatchObject({ status: "disconnected", pending: false });
    expect(s.ctl.model(b)).toMatchObject({ status: "disconnected", applying: false, applyId: null });
    expect(s.ctl.model(other)!.status).toBe("ready");
    // Geç gelen apply yanıtı yok sayılır
    d.resolve(applied());
    await p;
    await flush();
    expect(s.notify).not.toHaveBeenCalled();
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.ctl.model(b)!.outcome).toBeNull();
    // Silinen sunucu: gone
    s.ctl.serverSessionsClosed("S2", true);
    expect(s.ctl.model(other)).toMatchObject({ status: "gone", error: tdes.status.serverGone });
    // Yeniden bağlanınca reload ile hazır olur (kirli taslak force ister)
    await s.ctl.reload(a);
    expect(s.ctl.model(a)!.status).toBe("disconnected");
    await s.ctl.reload(a, { force: true });
    expect(s.ctl.model(a)!.status).toBe("ready");
  });

  it("close: uçuştaki önizleme/apply yanıtı durumu değiştirmez; ddlCancel çağrılır", async () => {
    const s = setup();
    // Önizleme uçuşta kapanış
    const p1 = await openAlter(s, "P");
    const pv = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(pv.promise);
    s.ctl.edit(p1, rename("x"));
    await settle();
    s.ctl.close(p1);
    pv.resolve(view());
    await flush();
    expect(s.ctl.model(p1)).toBeUndefined();
    // Apply uçuşta kapanış
    const id = await readyToApply(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.ctl.apply(id);
    s.ctl.close(id);
    expect(s.api.ddlCancel).toHaveBeenCalledExactlyOnceWith("A1");
    d.resolve(applied({ kind: "schema", oid: 9 }));
    await p;
    await flush();
    expect(s.ctl.model(id)).toBeUndefined();
    expect(s.notify).not.toHaveBeenCalled();
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(2); // yalnızca iki açılış
    // Yükleme uçuşta kapanış
    const ld = deferred<ObjectDef>();
    s.api.ddlLoad.mockReturnValueOnce(ld.promise);
    const op = openAlter(s, "L");
    s.ctl.close("L");
    ld.resolve(schema());
    await op;
    expect(s.ctl.model("L")).toBeUndefined();
    // Kapanmış sekmede işlemler zararsız
    s.ctl.edit("L", rename("x"));
    await s.ctl.apply("L");
    s.ctl.close("L");
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
  });

  it("cancelApply: uçuştaki apply için ddlCancel(applyId); hata yutulur; uçuşta değilse çağrı yok", async () => {
    const s = setup();
    const id = await readyToApply(s);
    await s.ctl.cancelApply(id);
    expect(s.api.ddlCancel).not.toHaveBeenCalled();
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.ctl.apply(id);
    s.api.ddlCancel.mockRejectedValueOnce(err("not_found"));
    await s.ctl.cancelApply(id);
    expect(s.api.ddlCancel).toHaveBeenCalledExactlyOnceWith("A1");
    d.resolve({ outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "57014", position: null, error: err("query", "iptal") });
    await p;
    expect(s.ctl.model(id)).toMatchObject({ applying: false, applyError: tdes.apply.failed(0, "iptal") });
  });

  it("reload yanıt sırası: eski yükleme yanıtı yenisinden sonra gelirse yok sayılır", async () => {
    const s = setup();
    const id = await openAlter(s);
    const d1 = deferred<ObjectDef>(), d2 = deferred<ObjectDef>();
    s.api.ddlLoad.mockReturnValueOnce(d1.promise).mockReturnValueOnce(d2.promise);
    const r1 = s.ctl.reload(id);
    const r2 = s.ctl.reload(id);
    d2.resolve(schema({ comment: "yeni" }));
    await r2;
    d1.resolve(schema({ comment: "eski" }));
    await r1;
    expect(s.ctl.model(id)!.original).toEqual(schema({ comment: "yeni" }));
  });

  it("serverChanged: locked/production yeniden hesaplanır; onay sıfırlanır", async () => {
    const s = setup();
    const id = await readyToApply(s, risky("data_loss"));
    s.ctl.setConfirm(id, { checked: true, typedName: "prod-1" });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsConfirm" });
    s.servers.S1!.production = true;
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)).toMatchObject({ production: true, confirm: { checked: false, typedName: "" } });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    s.ctl.setConfirm(id, { checked: true, typedName: "prod-1" });
    // Ad değişir: onay sıfırlanır, yeni ad beklenir
    s.servers.S1!.name = "prod-2";
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)).toMatchObject({ serverName: "prod-2", confirm: { checked: false, typedName: "" } });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsProductionName", expected: "prod-2" });
    // Kilit
    s.servers.S1!.locked = true;
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)!.locked).toBe(true);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "locked" });
    s.servers.S1!.locked = false;
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)!.locked).toBe(false);
    // Değişiklik yoksa onay korunur
    s.ctl.setConfirm(id, { checked: true });
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)!.confirm.checked).toBe(true);
    // Kayıt yoksa dokunulmaz
    delete s.servers.S1;
    s.ctl.serverChanged("S1");
    expect(s.ctl.model(id)!.serverName).toBe("prod-2");
  });

  it("objectDropped: eşleşen alter sekmeleri gone; diğerleri etkilenmez", async () => {
    const s = setup();
    const a = await openAlter(s, "A");
    const other = await openAlter(s, "B", "S1", { kind: "schema", oid: 101 });
    const otherDb = await openAlter(s, "C", "S2");
    const c = await openCreate(s);
    s.ctl.edit(a, rename("x"));
    s.ctl.objectDropped("S1", 5, { kind: "schema", oid: 100 });
    await settle();
    expect(s.ctl.model(a)).toMatchObject({ status: "gone", error: tdes.status.dropped, pending: false });
    expect(s.api.ddlPreview).not.toHaveBeenCalled();
    expect(s.ctl.model(other)!.status).toBe("ready");
    expect(s.ctl.model(otherDb)!.status).toBe("ready");
    expect(s.ctl.model(c)!.status).toBe("ready");
  });

  it("focus isteği: requestFocus her çağrıda yeni seq üretir", async () => {
    const s = setup();
    const id = await openAlter(s);
    expect(s.ctl.focusRequest(id)).toBeNull();
    s.ctl.requestFocus(id, "name");
    expect(s.ctl.focusRequest(id)).toEqual({ field: "name", seq: 1 });
    s.ctl.focusField(id, "name");
    expect(s.ctl.focusRequest(id)).toEqual({ field: "name", seq: 2 });
    expect(s.ctl.focusRequest("yok")).toBeNull();
    // Tüketim: eski seq yeni isteği silmez; aynı seq bir kez temizler (yeniden bağlanan form tekrar odaklamaz).
    s.ctl.consumeFocus(id, 1);
    expect(s.ctl.focusRequest(id)).toEqual({ field: "name", seq: 2 });
    s.ctl.consumeFocus(id, 2);
    expect(s.ctl.focusRequest(id)).toBeNull();
    s.ctl.consumeFocus(id, 2);
    expect(s.ctl.focusRequest(id)).toBeNull();
  });
});

describe("designer-controller ayarlar ve istekler", () => {
  it("setLockTimeout: geçersiz → varsayılan 5000; negatif → 5000; kesirli → aşağı", async () => {
    const s = setup();
    const id = await openAlter(s);
    expect(s.ctl.model(id)!.lockTimeoutMs).toBe(5000);
    s.ctl.setLockTimeout(id, 0);
    expect(s.ctl.model(id)!.lockTimeoutMs).toBe(0);
    s.ctl.setLockTimeout(id, Number.NaN);
    expect(s.ctl.model(id)!.lockTimeoutMs).toBe(5000);
    s.ctl.setLockTimeout(id, -1);
    expect(s.ctl.model(id)!.lockTimeoutMs).toBe(5000);
    s.ctl.setLockTimeout(id, 1500.7);
    expect(s.ctl.model(id)!.lockTimeoutMs).toBe(1500);
  });

  it("setConcurrently yalnızca index create'te önizlemeyi yeniler ve isteğe girer", async () => {
    const s = setup();
    const idx: ObjectDef = {
      kind: "index", schema: "public", name: "i", table: "t", method: "btree", unique: false, columns: [], include: [],
      tablespace: null, fillfactor: null, whereClause: null, comment: null, constraint: null, valid: true,
    } as unknown as ObjectDef;
    s.api.ddlDefaults.mockResolvedValueOnce(idx);
    await s.ctl.openCreate({ tabId: "I", serverId: "S1", databaseOid: 5, create: { kind: "index", context: { schemaOid: 1, tableOid: 2 } }, refreshNodeId: null });
    expect(s.ctl.isDirty("I")).toBe(false);
    s.ctl.setConcurrently("I", true);
    expect(s.ctl.isDirty("I")).toBe(true); // varsayılandan farklı (edit ile tutarlı)
    await settle();
    expect(s.api.ddlPreview.mock.calls[0]![2]).toMatchObject({ op: "create", concurrently: true });
    s.ctl.setConcurrently("I", false);
    expect(s.ctl.isDirty("I")).toBe(false);
    s.ctl.edit("I", (d) => ({ ...d, name: "j" }));
    s.ctl.setConcurrently("I", true);
    s.ctl.edit("I", (d) => ({ ...d, name: "i" })); // taslak varsayılana döndü ama CONCURRENTLY seçili
    expect(s.ctl.isDirty("I")).toBe(true);
    await settle();
    const n = s.api.ddlPreview.mock.calls.length;
    // Schema'da etkisiz
    const c = await openCreate(s);
    s.ctl.setConcurrently(c, true);
    await settle();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(n);
    expect(s.ctl.model(c)!.concurrently).toBe(false);
  });

  it("drop için cascade bayrağı request'e girer (drop request'i: object, def=original, cascade)", async () => {
    const s = setup();
    const id = await openAlter(s);
    s.ctl.edit(id, rename("taslak"));
    s.ctl.setCascade(id, true);
    const m = s.ctl.model(id)!;
    const req = planRequest(m, "drop");
    expect(req).toEqual({ op: "drop", object: OBJ, def: schema(), cascade: true });
    if (req?.op !== "drop") throw new Error("drop bekleniyordu");
    expect(req.def).not.toBe(m.original);
    s.ctl.setCascade(id, false);
    expect(planRequest(s.ctl.model(id)!, "drop")).toMatchObject({ cascade: false });
    // create sekmesinde drop isteği yok
    const c = await openCreate(s);
    expect(planRequest(s.ctl.model(c)!, "drop")).toBeNull();
  });
});

describe("designer-controller lookup önbelleği", () => {
  it("lookup önbelleği: aynı istek tek uçuşta, hata önbelleğe alınmaz, TTL (30 sn) sahte saatle", async () => {
    const s = setup();
    const a = await openAlter(s, "A");
    const b = await openAlter(s, "B");
    const d = deferred<LookupItem[]>();
    s.api.ddlLookup.mockReturnValueOnce(d.promise);
    const l1 = s.ctl.lookup(a, { kind: "tables", schema: "public" });
    const l2 = s.ctl.lookup(b, { schema: "public", kind: "tables" } as LookupRequest); // anahtar sırası farklı, aynı sunucu/db
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(1);
    expect(s.api.ddlLookup).toHaveBeenCalledWith("S1", 5, { kind: "tables", schema: "public" });
    d.resolve([{ value: "t", detail: null }]);
    expect(await l1).toEqual([{ value: "t", detail: null }]);
    expect(await l2).toEqual([{ value: "t", detail: null }]);
    // 30 sn içinde önbellekten
    s.tick(29_999);
    await s.ctl.lookup(a, { kind: "tables", schema: "public" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(1);
    s.tick(1);
    await s.ctl.lookup(a, { kind: "tables", schema: "public" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(2);
    // Oturum boyu türler TTL'siz
    await s.ctl.lookup(a, { kind: "roles" });
    s.tick(3_600_000);
    await s.ctl.lookup(b, { kind: "roles" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(3);
    // Hata önbelleğe alınmaz
    s.api.ddlLookup.mockRejectedValueOnce(err("query"));
    await expect(s.ctl.lookup(a, { kind: "schemas" })).rejects.toMatchObject({ code: "query" });
    expect(await s.ctl.lookup(a, { kind: "schemas" })).toEqual([{ value: "alice", detail: null }]);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(5);
    // Başka sunucu ayrı anahtar
    const o = await openAlter(s, "O", "S2");
    await s.ctl.lookup(o, { kind: "roles" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(6);
    // Oturumlar kapanınca o sunucunun önbelleği düşer
    s.ctl.serverSessionsClosed("S1", false);
    await s.ctl.lookup(a, { kind: "roles" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(7);
    // Bilinmeyen sekme: boş liste, istek yok
    expect(await s.ctl.lookup("yok", { kind: "roles" })).toEqual([]);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(7);
  });
});

describe("designer-controller yerel engel (sequence sayıları)", () => {
  const seqDef: ObjectDef = {
    kind: "sequence", schema: "app", name: "s", owner: null, dataType: "bigint", increment: "1", minValue: null,
    maxValue: null, start: "1", cache: "1", cycle: false, ownedBy: null, comment: null,
  };
  const SEQ: ObjectRef = { kind: "sequence", oid: 200 };
  const setNum = (k: "increment" | "start" | "cache" | "minValue" | "maxValue", v: string | null) => (d: ObjectDef) => {
    if (d.kind === "sequence") (d as Record<string, unknown>)[k] = v;
  };

  it("biçimsiz sayı: ddlPreview/ddlApply çağrılmaz, previewError yerel metin, kapı stale; düzelince metin kalkar", async () => {
    const s = setup();
    s.api.ddlLoad.mockResolvedValue(seqDef);
    const id = await openAlter(s, "Q1", "S1", SEQ);
    // Önce geçerli bir düzenleme ve önizleme (kapı hazır).
    s.ctl.edit(id, setNum("cache", "5"));
    await settle();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(1);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    let calls = 1;
    for (const [k, v] of [["increment", ""], ["start", "abc"], ["cache", "1e3"], ["increment", "-"], ["start", "9223372036854775808"], ["minValue", ""], ["maxValue", "x"]] as const) {
      s.ctl.edit(id, setNum(k, v));
      const m = s.ctl.model(id)!;
      expect(m).toMatchObject({ pending: false, previewError: tdes.sequence.fixNumbers });
      expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
      await settle();
      await s.ctl.apply(id);
      s.ctl.previewNow(id);
      await flush();
      expect(s.api.ddlPreview).toHaveBeenCalledTimes(calls);
      expect(s.api.ddlApply).not.toHaveBeenCalled();
      // Geri düzelt (bir sonraki durum için temiz taban).
      s.ctl.edit(id, setNum(k, k === "minValue" || k === "maxValue" ? null : "5"));
      expect(s.ctl.model(id)!).toMatchObject({ pending: true, previewError: null });
      await settle();
      calls++;
    }
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(8);
    expect(s.ctl.model(id)!.previewError).toBeNull();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
  });

  it("bekleyen önizleme biçimsiz düzenlemeyle iptal edilir (eski yanıt gelmez)", async () => {
    const s = setup();
    s.api.ddlLoad.mockResolvedValue(seqDef);
    const id = await openAlter(s, "Q2", "S1", SEQ);
    s.ctl.edit(id, setNum("cache", "5"));
    s.ctl.edit(id, setNum("cache", "abc"));
    await settle();
    expect(s.api.ddlPreview).not.toHaveBeenCalled();
    expect(s.ctl.model(id)!).toMatchObject({ pending: false, previewError: tdes.sequence.fixNumbers, view: null });
  });
});

const tbl = (p: Partial<TableDef> = {}): ObjectDef => ({
  kind: "table", schema: "public", name: "orders", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [],
  partitionBy: null, fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p,
});
const TOBJ: ObjectRef = { kind: "table", oid: 500 };

describe("designer-controller Task 4: findAlter, onApplied, Table", () => {
  it("findAlter: alter sekmesini bulur; create başarısı (created) sonrası aynı sekme; gone sekme bulunmaz", async () => {
    const s = setup();
    const id = await openAlter(s);
    expect(s.ctl.findAlter("S1", 5, OBJ)).toBe(id);
    expect(s.ctl.findAlter("S2", 5, OBJ)).toBeNull();
    expect(s.ctl.findAlter("S1", 6, OBJ)).toBeNull();
    expect(s.ctl.findAlter("S1", 5, { kind: "schema", oid: 101 })).toBeNull();
    const c = await openCreate(s);
    expect(s.ctl.findAlter("S1", 5, { kind: "schema", oid: 300 })).toBeNull();
    s.ctl.edit(c, rename("yeni"));
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied({ kind: "schema", oid: 300 }));
    s.ctl.setConfirm(c, { checked: true });
    await s.ctl.apply(c);
    await flush();
    expect(s.ctl.findAlter("S1", 5, { kind: "schema", oid: 300 })).toBe(c);
    s.ctl.objectDropped("S1", 5, { kind: "schema", oid: 300 });
    expect(s.ctl.findAlter("S1", 5, { kind: "schema", oid: 300 })).toBeNull();
  });

  it("onApplied: alter'da before=original kopyası, after=uygulanan taslak, object=created ?? object; create'te created null ise çağrılmaz", async () => {
    const calls: AppliedInfo[] = [];
    const s = setup({ onApplied: (i) => void calls.push(i) });
    const id = await readyToApply(s);
    await s.ctl.apply(id);
    await flush();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ tabId: id, kind: "schema", mode: "alter", before: schema(), after: schema({ name: "yeni" }), object: OBJ });
    // create, created null → çağrılmaz
    const c = await openCreate(s);
    s.ctl.edit(c, rename("x"));
    await settle();
    s.ctl.setConfirm(c, { checked: true });
    await s.ctl.apply(c);
    await flush();
    expect(calls).toHaveLength(1);
    // create, created dolu
    const c2 = await openCreate(s, "C2");
    s.ctl.edit(c2, rename("y"));
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied({ kind: "schema", oid: 9 }));
    s.ctl.setConfirm(c2, { checked: true });
    await s.ctl.apply(c2);
    await flush();
    expect(calls).toHaveLength(2);
    expect(calls[1]).toMatchObject({ tabId: "C2", mode: "create", before: null, after: schema({ name: "y", owner: null }), object: { kind: "schema", oid: 9 } });
  });

  it("onApplied hatası uygulamayı bozmaz (yeniden yükleme yine yapılır, toast tek)", async () => {
    const s = setup({ onApplied: () => { throw new Error("başlık"); } });
    const id = await readyToApply(s);
    await s.ctl.apply(id);
    await flush();
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(2);
    expect(s.ctl.model(id)).toMatchObject({ status: "ready", dirty: false });
    expect(s.notify.mock.calls.filter((c) => c[1] === "success")).toHaveLength(1);
  });

  it("Table alter kimliği aynı kalır (rename: created.oid = eski oid): yeniden yükleme aynı oid ile; model.object değişmez", async () => {
    const s = setup();
    s.api.ddlLoad.mockResolvedValue(tbl());
    await s.ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: TOBJ, refreshNodeId: "N" });
    s.ctl.edit("T1", (d) => ({ ...d, name: "orders2" }));
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied({ kind: "table", oid: 500 }));
    s.api.ddlLoad.mockResolvedValue(tbl({ name: "orders2" }));
    s.ctl.setConfirm("T1", { checked: true });
    await s.ctl.apply("T1");
    await flush();
    expect(s.api.ddlLoad).toHaveBeenLastCalledWith("S1", 5, { kind: "table", oid: 500 });
    expect(s.ctl.model("T1")!.object).toEqual(TOBJ);
    expect(s.ctl.findAlter("S1", 5, TOBJ)).toBe("T1");
  });

  it("Table DataLoss planı (DROP COLUMN) üretimde ad yazılmadan gönderilmez; Err confirmation_required → gateError + önizleme yeniden istenir", async () => {
    const s = setup();
    s.servers.S1!.production = true;
    const col = { name: "a", attnum: 1 };
    s.api.ddlLoad.mockResolvedValue(tbl({ columns: [col as never] }));
    await s.ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: TOBJ, refreshNodeId: "N" });
    s.api.ddlPreview.mockResolvedValue(risky("data_loss", "hd"));
    s.ctl.edit("T1", (d) => (d.kind === "table" ? { ...d, columns: [] } : d));
    await settle();
    expect(s.ctl.peekApplyGate("T1")).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    s.ctl.setConfirm("T1", { checked: true });
    await s.ctl.apply("T1");
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setConfirm("T1", { checked: true, typedName: "prod-1" });
    s.api.ddlApply.mockRejectedValueOnce(err("confirmation_required"));
    const before = s.api.ddlPreview.mock.calls.length;
    await s.ctl.apply("T1");
    await settle();
    expect(s.ctl.model("T1")!.gateError).toContain(tdes.gate.confirmationRequired);
    expect(s.api.ddlPreview.mock.calls.length).toBeGreaterThan(before);
  });

  it("findAlter: oid'i değiştiren alter (indeks yeniden kurma) sonrası yeni oid bulunur, eski oid bulunmaz", async () => {
    const s = setup();
    const id = await readyToApply(s);
    s.api.ddlApply.mockResolvedValueOnce(applied({ kind: "schema", oid: 777 }));
    await s.ctl.apply(id);
    await flush();
    expect(s.ctl.findAlter("S1", 5, { kind: "schema", oid: 777 })).toBe(id);
    expect(s.ctl.findAlter("S1", 5, OBJ)).toBeNull();
  });

  it("lookup önbelleği: collations oturum boyunca (TTL sonrası yeni istek yok); types{query} 30 sn", async () => {
    const s = setup();
    const id = await openAlter(s);
    await s.ctl.lookup(id, { kind: "collations" });
    await s.ctl.lookup(id, { kind: "types", query: "int" } as LookupRequest);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(2);
    s.tick(3_600_000);
    await s.ctl.lookup(id, { kind: "collations" });
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(2);
    await s.ctl.lookup(id, { kind: "types", query: "int" } as LookupRequest);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(3);
  });
});
