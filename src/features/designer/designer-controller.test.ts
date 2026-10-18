import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { AclDef } from "../../ipc/bindings/AclDef";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { DefaultAclScope } from "../../ipc/bindings/DefaultAclScope";
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
import type { CreatePreset } from "../../ipc/bindings/CreatePreset";
import type { TableDef } from "../../ipc/bindings/TableDef";
import type { RoleDef } from "../../ipc/bindings/RoleDef";
import type { Risk } from "../../ipc/bindings/Risk";
import { createDesignerController, planRequest, type AppliedInfo, type DesignerControllerDeps } from "./designer-controller";
import { failedText } from "./errors";

const schema = (p: Partial<{ name: string; owner: string | null; comment: string | null }> = {}): ObjectDef =>
  ({ kind: "schema", name: "app", owner: "alice", comment: null, ...p });
const view = (p: Partial<PlanView> = {}): PlanView => ({
  statements: [{ sql: "ALTER SCHEMA app RENAME TO x", risk: p.risk ?? "safe", transactional: true, note: null, redacted: false }],
  warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...p,
});
const risky = (risk: Risk, hash = "hr") => view({ risk, hash });
const applied = (created: ObjectRef | null = null, statements = 1): ApplyOutcome => ({ outcome: "applied", statements, elapsedMs: 3, created });
const err = (code: string, message = `m:${code}`, detail: string | null = null) => ({ code, message, detail, hint: null });
const OBJ: ObjectRef = { kind: "schema", oid: 100 };
const aclDef = (p: Partial<AclDef> = {}): AclDef => ({
  kind: "table", schema: "public", name: "orders", signature: null, owner: "alice", viewer: "alice", explicit: true,
  grants: [{ grantee: null, privileges: [{ privilege: "select", grantOption: false }] }], foreign: [], columns: [],
  editable: true, unsupported: [], available: ["select", "insert"], ...p,
});
const defAclDef = (p: Partial<DefaultAclDef> = {}): DefaultAclDef => ({
  role: "alice", schema: null, viewer: "alice",
  sections: [{ objectType: "tables", grants: [], baseline: [], available: ["select"] }], editable: true, unsupported: [], ...p,
});

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
    ddlDefaults: vi.fn(async (_s: string, _db: number | null, _k: ObjectKind, _c: CreateContext, _p: CreatePreset | null = null): Promise<ObjectDef> => schema({ name: "", owner: null })),
    ddlLoad: vi.fn(async (_s: string, _db: number | null, _o: ObjectRef): Promise<ObjectDef> => schema()),
    ddlLookup: vi.fn(async (_s: string, _db: number | null, _r: LookupRequest): Promise<LookupItem[]> => [{ value: "alice", detail: null }]),
    ddlPreview: vi.fn(async (_s: string, _db: number | null, _r: PlanRequest): Promise<PlanView> => view()),
    ddlApply: vi.fn(async (_s: string, _db: number | null, _r: PlanRequest, _o: ApplyOptions): Promise<ApplyOutcome> => applied()),
    ddlCancel: vi.fn(async (_id: string) => null),
    ddlLoadAcl: vi.fn(async (_s: string, _db: number | null, _o: ObjectRef): Promise<AclDef> => aclDef()),
    ddlLoadDefaultAcl: vi.fn(async (_s: string, _db: number, sc: DefaultAclScope): Promise<DefaultAclDef> => defAclDef({ role: sc.role ?? "alice" })),
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
    tabId, serverId: "S1", databaseOid: 5, create: { kind: "schema", context: { schemaOid: null, tableOid: null }, preset: null }, refreshNodeId: "N-coll",
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
    expect(s.api.ddlDefaults).toHaveBeenCalledWith("S1", 5, "schema", { schemaOid: null, tableOid: null }, null);
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
      applyId: "A1", lockTimeoutMs: 2000, expectedHash: "hd", confirmed: true, confirmedProduction: false, password: null,
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
    expect(m).toMatchObject({ gateError: tdes.gate.partialChanged, needsReload: true, view: null });
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
    await s.ctl.openCreate({ tabId: "I", serverId: "S1", databaseOid: 5, create: { kind: "index", context: { schemaOid: 1, tableOid: 2 }, preset: null }, refreshNodeId: null });
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
    expect(req).toEqual({ op: "drop", object: OBJ, def: schema(), cascade: true, force: false });
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

  it("sunucu kapsamlı (rol) uygulama/kısmi create/silme sunucunun tüm veritabanlarının lookup önbelleğini düşürür", async () => {
    const s = setup();
    await s.ctl.openAlter({ tabId: "D42", serverId: "S1", databaseOid: 42, object: OBJ, refreshNodeId: "N" });
    const lookupRoles = async () => {
      await s.ctl.lookup("D42", { kind: "roles" });
      return s.api.ddlLookup.mock.calls.length;
    };
    expect(await lookupRoles()).toBe(1);
    expect(await lookupRoles()).toBe(1); // önbellekten
    // rol create uygulandı
    const id = await openRoleCreate(s);
    s.api.ddlPreview.mockResolvedValue(view({ hash: "hc" }));
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.name = "yeni_rol";
    });
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied(ROLE));
    s.api.ddlLoad.mockResolvedValueOnce(role({ name: "yeni_rol" }));
    await s.ctl.apply(id);
    await flush();
    expect(await lookupRoles()).toBe(2);
    // rol silindi
    expect(await lookupRoles()).toBe(2);
    s.ctl.objectDropped("S1", null, ROLE);
    expect(await lookupRoles()).toBe(3);
    // kısmi rol create
    const p = await openRoleCreate(s, "R2");
    s.ctl.edit(p, (d) => {
      if (d.kind === "role") d.name = "baska_rol";
    });
    await settle();
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "42501", position: null, error: err("query", "denied"), created: ROLE,
    });
    await s.ctl.apply(p);
    await flush();
    expect(await lookupRoles()).toBe(4);
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

  it("m-2: süresi dolmuş kısa ömürlü girdiler yeni eklemede atılır (yazılan önekler birikmez); oturum boyu girdiler kalır", async () => {
    const s = setup();
    const id = await openAlter(s);
    await s.ctl.lookup(id, { kind: "roles" });
    for (const q of ["i", "in", "int"]) await s.ctl.lookup(id, { kind: "types", query: q } as LookupRequest);
    await s.ctl.lookup(id, { kind: "table_columns", schema: "public", table: "t" });
    expect(s.ctl.lookupCacheSize()).toBe(5);
    s.tick(29_999);
    await s.ctl.lookup(id, { kind: "types", query: "inte" } as LookupRequest);
    expect(s.ctl.lookupCacheSize()).toBe(6); // henüz süresi dolan yok
    s.tick(1);
    await s.ctl.lookup(id, { kind: "types", query: "integ" } as LookupRequest);
    // i/in/int/table_columns düştü; roles (oturum), inte (taze) ve integ kaldı.
    expect(s.ctl.lookupCacheSize()).toBe(3);
    s.tick(3_600_000);
    await s.ctl.lookup(id, { kind: "roles" }); // önbellekten: ekleme yok, temizlik yok
    expect(s.ctl.lookupCacheSize()).toBe(3);
    await s.ctl.lookup(id, { kind: "tables", schema: "public" });
    expect(s.ctl.lookupCacheSize()).toBe(2); // roles + tables
  });
});

// ---- Plan 15 Task 2: sunucu kapsamı, parola haritası ve kapısı
const SECRET = "Gizli-1";
const role = (p: Partial<RoleDef> = {}): ObjectDef => ({
  kind: "role", name: "app_user", login: true, superuser: false, createdb: false, createrole: false, inherit: true,
  replication: false, bypassrls: false, connectionLimit: -1, validUntil: null, password: "keep", memberOf: [], settings: [],
  comment: null, unsupported: [], ...p,
});
const ROLE: ObjectRef = { kind: "role", oid: 16384 };
const redacted = (p: Partial<PlanView> = {}): PlanView => view({
  statements: [
    { sql: "CREATE ROLE app_user LOGIN", risk: "safe", transactional: true, note: null, redacted: false },
    { sql: "ALTER ROLE app_user PASSWORD ********", risk: "safe", transactional: true, note: null, redacted: true },
  ],
  hash: "hp",
  ...p,
});
const setPassword = (v: RoleDef["password"]) => (d: ObjectDef) => {
  if (d.kind === "role") d.password = v;
};
async function openRoleCreate(s: S, tabId = "R1") {
  s.api.ddlDefaults.mockResolvedValue(role({ name: "" }));
  await s.ctl.openCreate({
    tabId, serverId: "S1", databaseOid: null, create: { kind: "role", context: { schemaOid: null, tableOid: null }, preset: null }, refreshNodeId: "roles",
  });
  return tabId;
}
async function openRoleAlter(s: S, tabId = "R1") {
  s.api.ddlLoad.mockResolvedValue(role());
  await s.ctl.openAlter({ tabId, serverId: "S1", databaseOid: null, object: ROLE, refreshNodeId: "roles" });
  return tabId;
}
/** Rol taslağı password="set" + gizlenmiş önizleme gelmiş. */
async function roleWithPasswordPlan(s: S, v: PlanView = redacted(), alter = false) {
  const id = alter ? await openRoleAlter(s) : await openRoleCreate(s);
  s.api.ddlPreview.mockResolvedValue(v);
  s.ctl.edit(id, (d) => {
    if (d.kind === "role") {
      d.name = "app_user";
      d.password = "set";
    }
  });
  await settle();
  return id;
}

describe("designer-controller Plan 15: parola", () => {
  it("parola yalnızca ddlApply options.password'da gider; ddlPreview isteğinde ve modelde yok", async () => {
    const s = setup();
    const id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.ctl.previewNow(id);
    await flush();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(2);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    expect(JSON.stringify(s.ctl.model(id))).not.toContain(SECRET);
    s.api.ddlApply.mockResolvedValueOnce(applied(ROLE));
    await s.ctl.apply(id);
    await flush();
    expect(JSON.stringify(s.api.ddlPreview.mock.calls)).not.toContain(SECRET);
    expect(s.api.ddlApply).toHaveBeenCalledOnce();
    expect(s.api.ddlApply.mock.calls[0]![3].password).toBe(SECRET);
    expect(JSON.stringify(s.api.ddlApply.mock.calls[0]![2])).not.toContain(SECRET);
    expect(JSON.stringify(s.ctl.model(id))).not.toContain(SECRET);
    expect(JSON.stringify(s.notify.mock.calls)).not.toContain(SECRET);
  });

  it("gizlenmemiş planda (parola niyeti yok) verilen parola gönderilmez", async () => {
    const s = setup();
    const id = await openRoleAlter(s);
    s.api.ddlPreview.mockResolvedValue(view());
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.comment = "x";
    });
    await settle();
    s.ctl.setSecret(id, SECRET); // savunma: harita boşaltılmamış olsa bile
    await s.ctl.apply(id);
    expect(s.api.ddlApply.mock.calls[0]![3].password).toBeNull();
  });

  it("başarılı uygulama, reload, close ve password≠set düzenlemesi parolayı siler; secretReady false olur", async () => {
    // başarılı uygulama
    let s = setup();
    let id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(true);
    s.api.ddlApply.mockResolvedValueOnce(applied(ROLE));
    await s.ctl.apply(id);
    await flush();
    expect(s.ctl.secretReady(id)).toBe(false);

    // başarısız uygulama (Failed, committed=0) parolayı korur: yeniden denenebilir
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 1, committed: 0, sqlstate: "42501", position: null, error: err("query", "denied"),
    });
    await s.ctl.apply(id);
    await flush();
    expect(s.ctl.secretReady(id)).toBe(true);

    // reload (force)
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    await s.ctl.reload(id, { force: true });
    expect(s.ctl.secretReady(id)).toBe(false);

    // close (aynı kimlikle yeniden açılan sekme parolayı görmez)
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.ctl.close(id);
    expect(s.ctl.secretReady(id)).toBe(false);
    id = await roleWithPasswordPlan(s);
    expect(s.ctl.secretReady(id)).toBe(false);

    // password ≠ set düzenlemesi
    for (const next of ["keep", "clear"] as const) {
      s = setup();
      id = await roleWithPasswordPlan(s);
      s.ctl.setSecret(id, SECRET);
      s.ctl.edit(id, (d) => {
        if (d.kind === "role") d.comment = "yorum"; // set kalır → silinmez
      });
      expect(s.ctl.secretReady(id)).toBe(true);
      s.ctl.edit(id, setPassword(next));
      expect(s.ctl.secretReady(id)).toBe(false);
    }

    // sunucu oturumları kapandı
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.ctl.secretReady(id)).toBe(false);

    // setSecret(null/boş) siler; bilinmeyen sekme saklanmaz
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.ctl.setSecret(id, "");
    expect(s.ctl.secretReady(id)).toBe(false);
    s.ctl.setSecret(id, SECRET);
    s.ctl.setSecret(id, null);
    expect(s.ctl.secretReady(id)).toBe(false);
    s.ctl.setSecret("YOK", SECRET);
    expect(s.ctl.secretReady("YOK")).toBe(false);
  });

  it("gizlenmiş planda parola yokken gate 'password'; parola girilince needsConfirm/ready; önizleme her durumda istenir", async () => {
    const s = setup();
    const id = await roleWithPasswordPlan(s, redacted({ risk: "destructive", hash: "hd" }), true);
    expect(s.api.ddlPreview).toHaveBeenCalledOnce(); // parola yokken de önizleme istendi
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "password" });
    s.ctl.setConfirm(id, { checked: true });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsConfirm" });
    s.ctl.setSecret(id, null);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "password" });
    // gizlenmemiş, güvenli plan: parola gerekmez
    s.api.ddlPreview.mockResolvedValue(view({ hash: "hs" }));
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.comment = "x";
    });
    await settle();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(2);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    // gizlenmiş + güvenli: parola girilince ready
    s.api.ddlPreview.mockResolvedValue(redacted({ hash: "hr2" }));
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.comment = "y";
    });
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "password" });
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
  });

  it("setSecret yalnızca rol taslağı password='set' ve status='ready' iken saklar; değer değişmedikçe registry dürtülmez", async () => {
    // password = keep (alter yüklemesi): saklanmaz
    let s = setup();
    let id = await openRoleAlter(s);
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(false);
    // rol dışı tür: saklanmaz
    s = setup();
    id = await openAlter(s);
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(false);
    // yükleniyor (ready değil): saklanmaz
    s = setup();
    s.api.ddlLoad.mockReturnValueOnce(new Promise(() => {}));
    void s.ctl.openAlter({ tabId: "R1", serverId: "S1", databaseOid: null, object: ROLE, refreshNodeId: null });
    s.ctl.setSecret("R1", SECRET);
    expect(s.ctl.secretReady("R1")).toBe(false);
    // bağlantı kesik: saklanmaz
    s = setup();
    id = await roleWithPasswordPlan(s);
    s.ctl.serverSessionsClosed("S1", false);
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(false);
    // set + ready: saklanır (dürtme sayısı designer-reactivity.test.ts'te); model nesnesi değişmez
    s = setup();
    id = await roleWithPasswordPlan(s);
    const before = s.ctl.model(id);
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(true);
    s.ctl.setSecret(id, null);
    expect(s.ctl.secretReady(id)).toBe(false);
    expect(s.ctl.model(id)).toBe(before);
  });

  it("connectedUser: sunucu kaydının kullanıcı adı (yoksa null); bilinmeyen sekme null", async () => {
    const s = setup({
      serverInfo: (id) => (id === "S1" ? { name: "prod-1", locked: false, production: false, username: "app_user" } : null),
    });
    const id = await openRoleAlter(s);
    expect(s.ctl.connectedUser(id)).toBe("app_user");
    expect(s.ctl.connectedUser("YOK")).toBeNull();
    const s2 = setup();
    const id2 = await openRoleAlter(s2);
    expect(s2.ctl.connectedUser(id2)).toBeNull();
  });

  it("role_password_missing hata kodu gateError metnine eşlenir ve uygulama yeniden denenebilir", async () => {
    const s = setup();
    const id = await roleWithPasswordPlan(s);
    s.ctl.setSecret(id, SECRET);
    s.api.ddlApply.mockRejectedValueOnce(err("role_password_missing"));
    await s.ctl.apply(id);
    await flush();
    const m = s.ctl.model(id)!;
    expect(m.gateError).toBe(tdes.gate.passwordMissing);
    expect(m.status).toBe("ready");
    expect(m.needsReload).toBe(false);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(2); // önizleme yeniden istendi
    s.api.ddlApply.mockResolvedValueOnce(applied(ROLE));
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(2);
    expect(s.api.ddlApply.mock.calls[1]![3].password).toBe(SECRET);
  });
});

describe("designer-controller Plan 15: sunucu kapsamı", () => {
  it("sunucu kapsamlı sekme: ddlLoad/ddlPreview/ddlApply/ddlLookup databaseOid=null ile çağrılır", async () => {
    const s = setup();
    const id = await openRoleAlter(s);
    expect(s.api.ddlLoad).toHaveBeenCalledWith("S1", null, ROLE);
    expect(s.ctl.model(id)!.databaseOid).toBeNull();
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.comment = "x";
    });
    await settle();
    expect(s.api.ddlPreview.mock.calls[0]![1]).toBeNull();
    await s.ctl.apply(id);
    await flush();
    expect(s.api.ddlApply.mock.calls[0]![1]).toBeNull();
    await s.ctl.lookup(id, { kind: "grantable_roles" });
    expect(s.api.ddlLookup).toHaveBeenCalledWith("S1", null, { kind: "grantable_roles" });
    expect(s.ctl.findAlter("S1", null, ROLE)).toBe(id);
    expect(s.ctl.findAlter("S1", 5, ROLE)).toBeNull();
    // create (Database) varsayılanları da null ile
    s.api.ddlDefaults.mockResolvedValueOnce({
      kind: "database", name: "", owner: null, template: null, encoding: null, localeProvider: null, lcCollate: null, lcCtype: null,
      providerLocale: null, tablespace: null, connectionLimit: -1, isTemplate: false, allowConnections: true, settings: [], comment: null,
    });
    await s.ctl.openCreate({ tabId: "D1", serverId: "S1", databaseOid: null, create: { kind: "database", context: { schemaOid: null, tableOid: null }, preset: null }, refreshNodeId: "dbs" });
    expect(s.api.ddlDefaults).toHaveBeenLastCalledWith("S1", null, "database", { schemaOid: null, tableOid: null }, null);
    // objectDropped null ile eşleşir
    s.ctl.objectDropped("S1", null, ROLE);
    expect(s.ctl.model(id)!.status).toBe("gone");
  });

  it("SESSION_LOOKUPS: grantable_roles/encodings/template_databases/locales/settings TTL sonrası yeniden istenmez", async () => {
    const s = setup();
    const id = await openRoleAlter(s);
    const reqs: LookupRequest[] = [
      { kind: "grantable_roles" }, { kind: "encodings" }, { kind: "template_databases" },
      { kind: "locales", provider: "libc" }, { kind: "locales", provider: "icu" }, { kind: "settings" },
    ];
    for (const r of reqs) await s.ctl.lookup(id, r);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(6); // locales{provider} ayrı anahtar
    s.tick(3_600_000);
    for (const r of reqs) await s.ctl.lookup(id, r);
    expect(s.api.ddlLookup).toHaveBeenCalledTimes(6);
  });
});

describe("designer-controller Plan 15: uygulama sonrası akışlar", () => {
  const DB5: ObjectRef = { kind: "database", oid: 5 };
  const vdef = (p: Record<string, unknown> = {}): ObjectDef => ({
    kind: "view", schema: "public", name: "v", owner: null, query: "SELECT 1", checkOption: null, securityBarrier: false,
    securityInvoker: false, comment: null, recreate: false, columns: [], unsupported: [], ...p,
  }) as ObjectDef;

  it("database drop: o veritabanına ait (veritabanı kapsamlı) açık sekmeler gone; sunucu kapsamlı ve diğer veritabanı/sunucu sekmeleri etkilenmez", async () => {
    const s = setup();
    const a = await openAlter(s, "A5"); // S1 / db 5
    const c = await openCreate(s, "C5"); // S1 / db 5 (create)
    await s.ctl.openAlter({ tabId: "A6", serverId: "S1", databaseOid: 6, object: OBJ, refreshNodeId: null });
    await s.ctl.openAlter({ tabId: "B5", serverId: "S2", databaseOid: 5, object: OBJ, refreshNodeId: null });
    const r = await openRoleAlter(s, "R1");
    await s.ctl.openAlter({ tabId: "D5", serverId: "S1", databaseOid: null, object: DB5, refreshNodeId: "dbs" });
    await s.ctl.openAlter({ tabId: "D6", serverId: "S1", databaseOid: null, object: { kind: "database", oid: 6 }, refreshNodeId: "dbs" });
    // Silme diyaloğunun başarı yolu: objectDropped(sunucu, null, database ref)
    s.ctl.objectDropped("S1", null, DB5);
    expect(s.ctl.model(a)).toMatchObject({ status: "gone", error: tdes.status.gone });
    expect(s.ctl.model(c)).toMatchObject({ status: "gone", error: tdes.status.gone });
    expect(s.ctl.model("D5")).toMatchObject({ status: "gone", error: tdes.status.dropped });
    for (const id of ["A6", "B5", r, "D6"]) expect(s.ctl.model(id)!.status).toBe("ready");
    // sekmeler kapanmaz
    expect(s.ctl.ids()).toEqual(expect.arrayContaining([a, c, "D5"]));
    // doğrudan çağrı: sunucu kapsamlı (null) sekmeler hiçbir zaman eşleşmez
    s.ctl.databaseDropped("S1", 6);
    expect(s.ctl.model("A6")!.status).toBe("gone");
    expect(s.ctl.model(r)!.status).toBe("ready");
    expect(s.ctl.model("D6")!.status).toBe("ready");
  });

  it("view recreate planı DataLoss → üretimde ad yazılmadan gönderilmez (kapı regresyonu)", async () => {
    const s = setup();
    s.servers.S1!.production = true;
    s.api.ddlLoad.mockResolvedValue(vdef());
    const id = await openAlter(s, "V1", "S1", { kind: "view", oid: 900 });
    s.api.ddlPreview.mockResolvedValue(risky("data_loss", "hv"));
    s.ctl.edit(id, (d) => {
      if (d.kind === "view") { d.query = "SELECT 2"; d.recreate = true; }
    });
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    s.ctl.setConfirm(id, { checked: true });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setConfirm(id, { typedName: "prod-1" });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    expect(s.api.ddlApply.mock.calls[0]![2]).toMatchObject({ op: "alter", new: { recreate: true } });
  });

  it("applied.warnings sekmede ve toast'ta görünür, yeniden yüklemede korunur, sonraki uygulamada temizlenir", async () => {
    const s = setup();
    const id = await readyToApply(s);
    expect(s.ctl.model(id)!.postApplyWarnings).toEqual([]);
    s.api.ddlApply.mockResolvedValueOnce({ ...applied(null, 2), warnings: ["üyelik farkı: a", "üyelik farkı: b"] } as ApplyOutcome);
    await s.ctl.apply(id);
    await flush();
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.apply.appliedWarnings(2, 2), "info");
    expect(tdes.apply.appliedWarnings(2, 2)).toContain("2 uyarı");
    expect(s.ctl.model(id)!.postApplyWarnings).toEqual(["üyelik farkı: a", "üyelik farkı: b"]);
    // uygulama sonrası yükleme ve elle yeniden yükleme korur
    await s.ctl.reload(id);
    expect(s.ctl.model(id)).toMatchObject({ status: "ready", postApplyWarnings: ["üyelik farkı: a", "üyelik farkı: b"] });
    // sonraki uygulama temizler (uçuşta da görünmez)
    s.ctl.edit(id, rename("baska"));
    await settle();
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.ctl.apply(id);
    expect(s.ctl.model(id)!.postApplyWarnings).toEqual([]);
    d.resolve(applied());
    await p;
    await flush();
    expect(s.ctl.model(id)!.postApplyWarnings).toEqual([]);
    expect(s.notify).toHaveBeenLastCalledWith(tdes.apply.applied(1), "success");
    // kapanışta model (ve uyarılar) gider
    s.ctl.close(id);
    expect(s.ctl.model(id)).toBeUndefined();
  });

  it("failed.created: ağaç yenilenir, create yeniden gönderilemez (gate partial), 'Düzenle olarak aç' openAlter'ı created ile çağırır", async () => {
    const onApplied = vi.fn();
    const s = setup({ onApplied });
    const id = await openRoleCreate(s);
    s.api.ddlPreview.mockResolvedValue(view({ hash: "hc" }));
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") {
        d.name = "yeni_rol";
        d.password = "set";
      }
    });
    await settle();
    s.ctl.setSecret(id, SECRET);
    expect(s.ctl.secretReady(id)).toBe(true);
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "42501", position: null, error: err("query", "denied"), created: ROLE,
    });
    await s.ctl.apply(id);
    await flush();
    expect(s.refreshNode).toHaveBeenCalledExactlyOnceWith("roles");
    expect(s.ctl.model(id)).toMatchObject({ mode: "create", partialCreated: ROLE });
    // nesne oluştu: parola artık gönderilmez, silinir; bildirim "kısmi" metnini taşır
    expect(s.ctl.secretReady(id)).toBe(false);
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.gate.partial, "error");
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "partial" });
    // düzenleme + taze önizleme sonrası bile kapı kapalı; apply gönderilmez
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.comment = "x";
    });
    await settle();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "partial" });
    await s.ctl.apply(id);
    // yeniden yükleme (varsayılanlar) de kısmi durumu silmez
    await s.ctl.reload(id, { force: true });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "partial" });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(1);
    expect(onApplied).not.toHaveBeenCalled();
    // Düzenle olarak aç: aynı sekme alter'a döner, created yüklenir
    s.api.ddlLoad.mockResolvedValueOnce(role({ name: "yeni_rol" }));
    await s.ctl.openCreatedAsAlter(id);
    expect(s.api.ddlLoad).toHaveBeenCalledExactlyOnceWith("S1", null, ROLE);
    expect(s.ctl.ids()).toEqual([id]);
    expect(s.ctl.model(id)).toMatchObject({ mode: "alter", object: ROLE, kind: "role", status: "ready", partialCreated: null, databaseOid: null });
    expect(s.ctl.findAlter("S1", null, ROLE)).toBe(id);
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
    // başlık güncellemesi yüklenen tanımla
    expect(onApplied).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ tabId: id, kind: "role", mode: "create", object: ROLE, after: role({ name: "yeni_rol" }) }));
    // kısmi durum yoksa çağrı etkisiz
    await s.ctl.openCreatedAsAlter(id);
    expect(s.api.ddlLoad).toHaveBeenCalledTimes(1);
  });

  it("failed.created yokken eski davranış (yeniden denenebilir)", async () => {
    const s = setup();
    const id = await openRoleCreate(s);
    s.api.ddlPreview.mockResolvedValue(view({ hash: "hc" }));
    s.ctl.edit(id, (d) => {
      if (d.kind === "role") d.name = "yeni_rol";
    });
    await settle();
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "42501", position: null, error: err("query", "denied"),
    });
    await s.ctl.apply(id);
    await settle();
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.ctl.model(id)!.partialCreated).toBeNull();
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "ready" });
    // yeniden deneme gönderilir; committed > 0 ama created yok: eski kısmi yol (yeniden yükleme ipucu), partial kapısı yok
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "XX000", position: null, error: err("query", "x"),
    });
    await s.ctl.apply(id);
    expect(s.api.ddlApply).toHaveBeenCalledTimes(2);
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.ctl.model(id)).toMatchObject({ gateError: tdes.gate.partialChanged, needsReload: true, partialCreated: null });
    await s.ctl.openCreatedAsAlter(id);
    expect(s.api.ddlLoad).not.toHaveBeenCalled();
  });

  it("failed.created alter'da (ör. yeniden adlandırma sonrası hata): nesne kimliği güncellenir, yeniden yükleme ipucu", async () => {
    const s = setup();
    const id = await readyToApply(s);
    const moved: ObjectRef = { kind: "schema", oid: 101 };
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "42501", position: null, error: err("query", "denied"), created: moved,
    });
    await s.ctl.apply(id);
    expect(s.refreshNode).toHaveBeenCalledWith("N-parent");
    expect(s.ctl.model(id)).toMatchObject({ mode: "alter", object: moved, needsReload: true, gateError: tdes.gate.partialChanged, partialCreated: null });
    expect(s.ctl.peekApplyGate(id)).toEqual({ kind: "blocked", reason: "stale" });
  });
});

describe("designer-controller Plan 19: preset ve editable/unsupported", () => {
  const fn = (p: Record<string, unknown> = {}): ObjectDef => ({
    kind: "function", schema: "public", name: "", owner: null, comment: null, language: "plpgsql", args: [],
    returns: { kind: "trigger" }, body: "BEGIN\n  RETURN NEW;\nEND;", volatility: "volatile", strict: false, securityDefiner: false,
    leakproof: false, parallel: "unsafe", cost: null, rows: null, settings: [], recreate: false, editable: true, unsupported: [], ...p,
  } as ObjectDef);

  it("create preset ddlDefaults'a aynen geçer (trigger_function); reload aynı preset'i gönderir", async () => {
    const s = setup();
    s.api.ddlDefaults.mockResolvedValue(fn());
    await s.ctl.openCreate({
      tabId: "F1", serverId: "S1", databaseOid: 5, create: { kind: "function", context: { schemaOid: 2200, tableOid: null }, preset: "trigger_function" },
      refreshNodeId: "coll-tf",
    });
    expect(s.api.ddlDefaults).toHaveBeenLastCalledWith("S1", 5, "function", { schemaOid: 2200, tableOid: null }, "trigger_function");
    await s.ctl.reload("F1", { force: true });
    expect(s.api.ddlDefaults).toHaveBeenCalledTimes(2);
    expect(s.api.ddlDefaults).toHaveBeenLastCalledWith("S1", 5, "function", { schemaOid: 2200, tableOid: null }, "trigger_function");
  });

  it("ddl_defaults'ın editable/unsupported alanları düzenleme sonrası PlanRequest'te korunur", async () => {
    const s = setup();
    s.api.ddlDefaults.mockResolvedValue(fn());
    await s.ctl.openCreate({
      tabId: "F2", serverId: "S1", databaseOid: 5, create: { kind: "function", context: { schemaOid: 2200, tableOid: null }, preset: null }, refreshNodeId: null,
    });
    s.ctl.edit("F2", (d) => {
      if (d.kind === "function") d.name = "f";
    });
    await settle();
    const req = s.api.ddlPreview.mock.calls.at(-1)![2];
    expect(req.op).toBe("create");
    const def = (req as Extract<PlanRequest, { op: "create" }>).def as Record<string, unknown>;
    expect(def).toMatchObject({ kind: "function", name: "f", editable: true, unsupported: [], recreate: false });
    // Varsayılana dönen düzenleme kirli değil (editable/unsupported karşılaştırmada korunur)
    s.ctl.edit("F2", (d) => {
      if (d.kind === "function") d.name = "";
    });
    expect(s.ctl.isDirty("F2")).toBe(false);
  });

  it("alter: yüklenen editable=false + unsupported old/new'de aynen gider", async () => {
    const s = setup();
    const loaded = fn({ name: "f", editable: false, unsupported: ["C dili"] });
    s.api.ddlLoad.mockResolvedValue(loaded);
    await s.ctl.openAlter({ tabId: "F3", serverId: "S1", databaseOid: 5, object: { kind: "function", oid: 9 }, refreshNodeId: null });
    s.ctl.edit("F3", (d) => {
      if (d.kind === "function") d.comment = "x";
    });
    await settle();
    const req = s.api.ddlPreview.mock.calls.at(-1)![2] as Extract<PlanRequest, { op: "alter" }>;
    expect(req.old).toMatchObject({ editable: false, unsupported: ["C dili"] });
    expect(req.new).toMatchObject({ editable: false, unsupported: ["C dili"], comment: "x" });
  });
});

describe("Plan 22: yetki varyantları (acl / default_acl)", () => {
  const TBL: ObjectRef = { kind: "table", oid: 42 };
  const CTX = { serverId: "S1", databaseOid: 5, refreshNodeId: "N-tbl" };
  const grantInsert = (d: AclDef) => {
    d.grants[0]!.privileges.push({ privilege: "insert", grantOption: false });
  };

  it("openAcl: ddlLoadAcl çağrılır; variant=acl, mode=alter, kind nesne türü; dirty=false", async () => {
    const s = setup();
    await s.ctl.openAcl("P1", CTX, TBL);
    expect(s.api.ddlLoadAcl).toHaveBeenCalledExactlyOnceWith("S1", 5, TBL);
    expect(s.api.ddlLoad).not.toHaveBeenCalled();
    const m = s.ctl.model("P1")!;
    expect(m).toMatchObject({ variant: "acl", mode: "alter", kind: "table", object: TBL, status: "ready", dirty: false, aclScope: null });
    expect(m.original).toEqual(aclDef());
    expect(m.draft).toEqual(aclDef());
    expect(m.draft).not.toBe(m.original);
  });

  it("acl: düzenleme → privileges isteği (cascade dahil); geri alınan düzenleme dirty=false", async () => {
    const s = setup();
    await s.ctl.openAcl("P1", CTX, TBL);
    s.ctl.setCascade("P1", true);
    s.ctl.editAcl("P1", grantInsert);
    expect(s.ctl.isDirty("P1")).toBe(true);
    await settle();
    const want = aclDef();
    want.grants[0]!.privileges.push({ privilege: "insert", grantOption: false });
    expect(s.api.ddlPreview).toHaveBeenLastCalledWith("S1", 5, { op: "privileges", object: TBL, old: aclDef(), new: want, cascade: true });
    expect(planRequest(s.ctl.model("P1")!)).toEqual({ op: "privileges", object: TBL, old: aclDef(), new: want, cascade: true });
    s.ctl.editAcl("P1", (d) => { d.grants[0]!.privileges.pop(); });
    expect(s.ctl.isDirty("P1")).toBe(false);
    // Nesne düzenlemesi yetki sekmesinde etkisizdir.
    s.ctl.edit("P1", (d) => ({ ...d, name: "x" }));
    expect(s.ctl.model("P1")!.draft).toEqual(aclDef());
  });

  it("acl: uygulama sonrası ddlLoadAcl ile yeniden yüklenir; onApplied/ağaç yenilemesi yok; uyarılar kalır", async () => {
    const onApplied = vi.fn();
    const s = setup({ onApplied });
    await s.ctl.openAcl("P1", CTX, TBL);
    s.ctl.editAcl("P1", grantInsert);
    await settle();
    s.api.ddlApply.mockResolvedValueOnce({ outcome: "applied", statements: 1, elapsedMs: 1, created: null, warnings: ["ACL uyarısı"] });
    await s.ctl.apply("P1");
    await flush();
    expect(s.api.ddlLoadAcl).toHaveBeenCalledTimes(2);
    expect(s.api.ddlLoad).not.toHaveBeenCalled();
    expect(onApplied).not.toHaveBeenCalled();
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.ctl.model("P1")).toMatchObject({ dirty: false, needsReload: false, postApplyWarnings: ["ACL uyarısı"] });
  });

  it("başarılı uygulama Inspector'ı yeniler (yetki ve nesne sekmesi); hatası sonucu bozmaz", async () => {
    const refreshInspector = vi.fn();
    const s = setup({ refreshInspector });
    await s.ctl.openAcl("P1", CTX, TBL);
    s.ctl.editAcl("P1", grantInsert);
    await settle();
    s.api.ddlApply.mockResolvedValueOnce(applied());
    await s.ctl.apply("P1");
    await flush();
    expect(refreshInspector).toHaveBeenCalledTimes(1);

    const t = setup({ refreshInspector: () => { throw new Error("x"); } });
    const id = await readyToApply(t);
    await t.ctl.apply(id);
    await flush();
    expect(t.ctl.model(id)).toMatchObject({ status: "ready", dirty: false });
  });

  it("başarısız uygulama Inspector'ı yenilemez", async () => {
    const refreshInspector = vi.fn();
    const s = setup({ refreshInspector });
    const id = await readyToApply(s);
    s.api.ddlApply.mockResolvedValueOnce({ outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "42501", position: null, error: { code: "query", message: "x", detail: null, hint: null } });
    await s.ctl.apply(id);
    await flush();
    expect(refreshInspector).not.toHaveBeenCalled();
  });

  it("acl: reload ddlLoadAcl kullanır", async () => {
    const s = setup();
    await s.ctl.openAcl("P1", CTX, TBL);
    await s.ctl.reload("P1");
    expect(s.api.ddlLoadAcl).toHaveBeenCalledTimes(2);
  });

  it("acl sekmesi findAlter ile nesne sekmesi sayılmaz; nesne silinince gone olur", async () => {
    const s = setup();
    await s.ctl.openAcl("P1", CTX, TBL);
    expect(s.ctl.findAlter("S1", 5, TBL)).toBeNull();
    s.ctl.objectDropped("S1", 5, TBL);
    expect(s.ctl.model("P1")!.status).toBe("gone");
  });

  it("openDefaultAcl: ddlLoadDefaultAcl (databaseOid zorunlu); default_privileges isteği", async () => {
    const s = setup();
    const scope: DefaultAclScope = { role: null, schemaOid: 2200 };
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, scope);
    expect(s.api.ddlLoadDefaultAcl).toHaveBeenCalledExactlyOnceWith("S1", 5, scope);
    const m = s.ctl.model("D1")!;
    expect(m).toMatchObject({ variant: "default_acl", mode: "alter", kind: "schema", aclScope: scope, databaseOid: 5, dirty: false });
    s.ctl.editDefaultAcl("D1", (d) => { d.sections[0]!.grants.push({ grantee: "bob", privileges: [{ privilege: "select", grantOption: false }] }); });
    expect(s.ctl.isDirty("D1")).toBe(true);
    await settle();
    const req = s.api.ddlPreview.mock.calls.at(-1)![2];
    expect(req).toMatchObject({ op: "default_privileges", scope, old: defAclDef() });
    expect(req.op === "default_privileges" && req.new.sections[0]!.grants).toHaveLength(1);
    // Veritabanı kapsamı: kind database
    await s.ctl.openDefaultAcl("D2", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: null });
    expect(s.ctl.model("D2")!.kind).toBe("database");
  });

  it("setAclRole: kapsamın rolü değişir ve yeniden yüklenir (kirli taslak atılır)", async () => {
    const s = setup();
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: null });
    s.ctl.editDefaultAcl("D1", (d) => { d.sections[0]!.grants.push({ grantee: "bob", privileges: [] }); });
    s.ctl.setAclRole("D1", "bob");
    await flush();
    expect(s.api.ddlLoadDefaultAcl).toHaveBeenLastCalledWith("S1", 5, { role: "bob", schemaOid: null });
    const m = s.ctl.model("D1")!;
    expect(m.aclScope).toEqual({ role: "bob", schemaOid: null });
    expect(m.draft).toEqual(defAclDef({ role: "bob" }));
    expect(m.dirty).toBe(false);
    // Nesne/acl sekmesinde etkisiz.
    await s.ctl.openAcl("P1", CTX, TBL);
    s.ctl.setAclRole("P1", "bob");
    await flush();
    expect(s.ctl.model("P1")!.aclScope).toBeNull();
  });

  it("acl: CASCADE değişimi sürümü ilerletir ve yeniden önizler; kapı yeni görünüme göre (üretim adı)", async () => {
    const s = setup();
    s.servers.S1!.production = true;
    await s.ctl.openAcl("P1", CTX, TBL);
    s.api.ddlPreview.mockResolvedValue(view({ risk: "destructive", hash: "h-rev" }));
    s.ctl.editAcl("P1", (d) => { d.grants = []; });
    await settle();
    expect(s.ctl.peekApplyGate("P1")).toEqual({ kind: "needsConfirm" });
    const v0 = s.ctl.model("P1")!.version;
    s.api.ddlPreview.mockResolvedValue(view({ risk: "data_loss", hash: "h-cascade" }));
    s.ctl.setCascade("P1", true);
    expect(s.ctl.model("P1")!.version).toBe(v0 + 1);
    expect(s.ctl.peekApplyGate("P1").kind).toBe("blocked");
    await settle();
    expect(s.api.ddlPreview).toHaveBeenLastCalledWith("S1", 5, expect.objectContaining({ op: "privileges", cascade: true }));
    expect(s.ctl.peekApplyGate("P1")).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    // Aynı değer: yeni önizleme yok.
    const calls = s.api.ddlPreview.mock.calls.length;
    s.ctl.setCascade("P1", true);
    await settle();
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(calls);
  });

  it("acl: yeni temelde (yeniden yükleme / uygulama sonrası) CASCADE false olur", async () => {
    const s = setup();
    await s.ctl.openAcl("P1", CTX, TBL);
    s.ctl.setCascade("P1", true);
    await s.ctl.reload("P1");
    expect(s.ctl.model("P1")!.cascade).toBe(false);
    s.ctl.editAcl("P1", grantInsert);
    s.ctl.setCascade("P1", true);
    await settle();
    await s.ctl.apply("P1");
    await flush();
    expect(s.api.ddlApply).toHaveBeenCalledWith("S1", 5, expect.objectContaining({ cascade: true }), expect.anything());
    expect(s.ctl.model("P1")).toMatchObject({ cascade: false, dirty: false });
  });

  it("setAclRole: yükleme başarısızsa kapsam eski role döner, eski tanım korunur, hata görünür", async () => {
    const s = setup();
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: null });
    const before = s.ctl.model("D1")!;
    s.api.ddlLoadDefaultAcl.mockRejectedValueOnce(err("role_not_found", "rol yok"));
    s.ctl.setAclRole("D1", "yok");
    await flush();
    const m = s.ctl.model("D1")!;
    expect(m.aclScope).toEqual({ role: null, schemaOid: null });
    expect(m.status).toBe("ready");
    expect(m.error).toBeTruthy();
    expect(m.original).toEqual(before.original);
    expect(m.draft).toEqual(before.draft);
    // Sonraki istek eski kapsamla gider.
    s.ctl.editDefaultAcl("D1", (d) => { d.sections[0]!.grants.push({ grantee: "bob", privileges: [{ privilege: "select", grantOption: false }] }); });
    await settle();
    expect(s.api.ddlPreview).toHaveBeenLastCalledWith("S1", 5, expect.objectContaining({ op: "default_privileges", scope: { role: null, schemaOid: null } }));
  });

  it("setAclRole: şema/veritabanı yok (`not_found`) → sekme gone (rol hatası değil)", async () => {
    const s = setup();
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: 7 });
    s.api.ddlLoadDefaultAcl.mockRejectedValueOnce(err("not_found", "şema yok"));
    s.ctl.setAclRole("D1", "bob");
    await flush();
    const m = s.ctl.model("D1")!;
    expect(m.status).toBe("gone");
    expect(m.aclRoleGone).toBeNull();
  });

  it("yeniden yükleme: seçili rol artık yok (`role_not_found`) → bağlı role (role: null) düşer, bilgi notu; sekme düşmez", async () => {
    const s = setup();
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: null });
    s.ctl.setAclRole("D1", "bob");
    await flush();
    expect(s.ctl.model("D1")!.aclScope).toEqual({ role: "bob", schemaOid: null });
    s.api.ddlLoadDefaultAcl.mockRejectedValueOnce(err("role_not_found", "rol yok"));
    await s.ctl.reload("D1");
    await flush();
    expect(s.api.ddlLoadDefaultAcl).toHaveBeenLastCalledWith("S1", 5, { role: null, schemaOid: null });
    const m = s.ctl.model("D1")!;
    expect(m).toMatchObject({ status: "ready", error: null, aclScope: { role: null, schemaOid: null }, aclRoleGone: "bob" });
    expect(m.draft).toEqual(defAclDef());
    // Sonraki rol değişimi notu temizler.
    s.ctl.setAclRole("D1", "carol");
    await flush();
    expect(s.ctl.model("D1")!.aclRoleGone).toBeNull();
  });

  it("yeniden yükleme: bağlı rol kapsamında (role: null) `role_not_found` ya da şema `not_found` → gone", async () => {
    const s = setup();
    await s.ctl.openDefaultAcl("D1", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: null, schemaOid: null });
    s.api.ddlLoadDefaultAcl.mockRejectedValueOnce(err("role_not_found"));
    await s.ctl.reload("D1");
    expect(s.ctl.model("D1")!.status).toBe("gone");
    await s.ctl.openDefaultAcl("D2", { serverId: "S1", databaseOid: 5, refreshNodeId: null }, { role: "bob", schemaOid: 7 });
    s.api.ddlLoadDefaultAcl.mockRejectedValueOnce(err("not_found"));
    await s.ctl.reload("D2");
    expect(s.ctl.model("D2")!).toMatchObject({ status: "gone", aclScope: { role: "bob", schemaOid: 7 } });
  });

  it("planRequest: varyant yoksa nesne isteği (geriye uyumlu); default_acl kapsamsız → null", () => {
    expect(planRequest({
      variant: "default_acl", aclScope: null, mode: "alter", kind: "database", object: null, original: defAclDef(), draft: defAclDef(),
      concurrently: false, cascade: false,
    })).toBeNull();
    expect(planRequest({
      variant: "acl", aclScope: null, mode: "alter", kind: "table", object: TBL, original: aclDef(), draft: aclDef(), concurrently: false, cascade: false,
    }, "drop")).toBeNull();
  });
});
