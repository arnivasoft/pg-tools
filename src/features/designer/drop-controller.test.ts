import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { CreateContext } from "../../ipc/bindings/CreateContext";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { LookupRequest } from "../../ipc/bindings/LookupRequest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Risk } from "../../ipc/bindings/Risk";
import { createDesignerController, type DesignerControllerDeps } from "./designer-controller";
import type { DropInput } from "./drop-controller";

const schema = (p: Partial<{ name: string; owner: string | null; comment: string | null }> = {}): ObjectDef =>
  ({ kind: "schema", name: "app", owner: "alice", comment: null, ...p });
const view = (risk: Risk = "destructive", hash = "h1", p: Partial<PlanView> = {}): PlanView => ({
  statements: [{ sql: risk === "data_loss" ? "DROP SCHEMA app CASCADE" : "DROP SCHEMA app", risk, transactional: true, note: null, redacted: false }],
  warnings: [], errors: [], hash, risk, empty: false, ...p,
});
const applied = (): ApplyOutcome => ({ outcome: "applied", statements: 1, elapsedMs: 2, created: null });
const err = (code: string, message = `m:${code}`, detail: string | null = null) => ({ code, message, detail, hint: null });
const OBJ: ObjectRef = { kind: "schema", oid: 100 };
const INPUT: DropInput = { serverId: "S1", databaseOid: 5, object: OBJ, refreshNodeId: "N-coll", name: "app" };

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function setup() {
  let ids = 0;
  const servers: Record<string, { name: string; locked: boolean; production: boolean }> = {
    S1: { name: "prod-1", locked: false, production: false },
  };
  const api = {
    ddlDefaults: vi.fn(async (_s: string, _db: number | null, _k: ObjectKind, _c: CreateContext): Promise<ObjectDef> => schema()),
    ddlLoad: vi.fn(async (_s: string, _db: number | null, _o: ObjectRef): Promise<ObjectDef> => schema()),
    ddlLookup: vi.fn(async (_s: string, _db: number | null, _r: LookupRequest): Promise<LookupItem[]> => []),
    ddlPreview: vi.fn(async (_s: string, _db: number | null, r: PlanRequest): Promise<PlanView> =>
      view(r.op === "drop" && r.cascade ? "data_loss" : "destructive", r.op === "drop" && r.cascade ? "hc" : "h1")),
    ddlApply: vi.fn(async (_s: string, _db: number | null, _r: PlanRequest, _o: ApplyOptions): Promise<ApplyOutcome> => applied()),
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
  };
  const ctl = createDesignerController(deps);
  return { ctl, drop: ctl.drop, api, notify, refreshNode, servers };
}
type S = ReturnType<typeof setup>;

const flush = () => vi.advanceTimersByTimeAsync(0);
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

/** Diyalog açık, tanım yüklendi, önizleme geldi. */
async function ready(s: S, input: DropInput = INPUT) {
  s.drop.open(input);
  await flush();
  return s.drop.state()!;
}

describe("drop-controller açılış ve önizleme", () => {
  it("ddlLoad → def olduğu gibi (derin kopya) Drop isteğine gider; cascade varsayılan kapalı", async () => {
    const s = setup();
    const loaded = schema({ comment: "yorum" });
    s.api.ddlLoad.mockResolvedValueOnce(loaded);
    const st = await ready(s);
    expect(s.api.ddlLoad).toHaveBeenCalledWith("S1", 5, OBJ);
    expect(st).toMatchObject({ phase: "ready", cascade: false, pending: false, def: loaded });
    expect(st.def).not.toBe(loaded);
    expect(s.api.ddlPreview).toHaveBeenCalledExactlyOnceWith("S1", 5, { op: "drop", object: OBJ, def: loaded, cascade: false, force: false });
    const sent = s.api.ddlPreview.mock.calls[0]![2];
    if (sent.op !== "drop") throw new Error("drop bekleniyordu");
    expect(sent.def).not.toBe(st.def);
    expect(st.view?.hash).toBe("h1");
    expect(s.drop.gate()).toEqual({ kind: "needsConfirm" });
  });

  it("cascade değişince önizleme yeniden istenir; en son istek kazanır", async () => {
    const s = setup();
    await ready(s);
    const a = deferred<PlanView>();
    const b = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    s.drop.setCascade(true);
    s.drop.setCascade(false);
    expect(s.api.ddlPreview.mock.calls.map((c) => (c[2] as Extract<PlanRequest, { op: "drop" }>).cascade)).toEqual([false, true, false]);
    expect(s.drop.state()!.pending).toBe(true);
    expect(s.drop.gate()).toEqual({ kind: "blocked", reason: "pending" });
    b.resolve(view("destructive", "h-last"));
    await flush();
    a.resolve(view("data_loss", "h-old"));
    await flush();
    expect(s.drop.state()).toMatchObject({ cascade: false, pending: false });
    expect(s.drop.state()!.view?.hash).toBe("h-last");
  });

  it("kapatma yükleme/önizleme sırasında: geç yanıt durum yazmaz", async () => {
    const s = setup();
    const load = deferred<ObjectDef>();
    s.api.ddlLoad.mockReturnValueOnce(load.promise);
    s.drop.open(INPUT);
    expect(s.drop.state()!.phase).toBe("loading");
    s.drop.close();
    load.resolve(schema());
    await flush();
    expect(s.drop.state()).toBeNull();
    expect(s.api.ddlPreview).not.toHaveBeenCalled();

    const pv = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(pv.promise);
    s.drop.open(INPUT);
    await flush();
    s.drop.close();
    pv.resolve(view());
    await flush();
    expect(s.drop.state()).toBeNull();
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("yükleme hataları: bağlantı → kapanır + bildirim; not_found → stale; genel → yeniden yüklenebilir", async () => {
    const s = setup();
    s.api.ddlLoad.mockRejectedValueOnce(err("connection_lost", "koptu"));
    s.drop.open(INPUT);
    await flush();
    expect(s.drop.state()).toBeNull();
    expect(s.notify).toHaveBeenCalledWith("koptu", "error");

    s.api.ddlLoad.mockRejectedValueOnce(err("not_found"));
    s.drop.open(INPUT);
    await flush();
    expect(s.drop.state()).toMatchObject({ phase: "error", canReload: false, status: { kind: "stale", text: tdes.drop.stale } });
    expect(s.drop.gate()).toEqual({ kind: "blocked", reason: "loading" });

    s.api.ddlLoad.mockRejectedValueOnce(err("query", "izin yok", "d"));
    s.drop.open(INPUT);
    await flush();
    expect(s.drop.state()).toMatchObject({ phase: "error", canReload: true, status: { kind: "other", text: "izin yok — d" } });
    s.drop.reload();
    await flush();
    expect(s.drop.state()!.phase).toBe("ready");
  });
});

describe("drop-controller kapı ve uygulama", () => {
  it("onay kutusu olmadan gönderilmez; işaretliyken confirmed=true ile tek ddlApply", async () => {
    const s = setup();
    await ready(s);
    await s.drop.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.drop.setConfirm({ checked: true });
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const first = s.drop.apply();
    void s.drop.apply(); // çift tıklama
    expect(s.api.ddlApply).toHaveBeenCalledOnce();
    expect(s.api.ddlApply).toHaveBeenCalledWith("S1", 5, { op: "drop", object: OBJ, def: schema(), cascade: false, force: false },
      { applyId: "A1", lockTimeoutMs: 5000, expectedHash: "h1", confirmed: true, confirmedProduction: false, password: null });
    expect(s.drop.state()!.phase).toBe("applying");
    s.drop.setCascade(true); // uçuşta etkisiz
    expect(s.drop.state()!.cascade).toBe(false);
    d.resolve(applied());
    await first;
  });

  it("başarı: bildirim + refreshNode + aynı nesnenin alter sekmesi gone (bir kez); diyalog kapanır", async () => {
    const s = setup();
    await s.ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: OBJ, refreshNodeId: "N-coll" });
    await s.ctl.openAlter({ tabId: "T2", serverId: "S1", databaseOid: 5, object: { kind: "schema", oid: 101 }, refreshNodeId: null });
    await ready(s);
    s.drop.setConfirm({ checked: true });
    await s.drop.apply();
    expect(s.drop.state()).toBeNull();
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.drop.done("app"), "success");
    expect(s.refreshNode).toHaveBeenCalledExactlyOnceWith("N-coll");
    expect(s.ctl.model("T1")).toMatchObject({ status: "gone", error: tdes.status.dropped });
    expect(s.ctl.model("T2")!.status).toBe("ready");
  });

  it("üretim + cascade (data_loss): kutu yetmez, sunucu adı tam yazılmalı", async () => {
    const s = setup();
    s.servers.S1!.production = true;
    await ready(s);
    expect(s.drop.gate()).toEqual({ kind: "needsConfirm" }); // cascade kapalı: destructive
    s.drop.setCascade(true);
    await flush();
    expect(s.drop.gate()).toEqual({ kind: "needsProductionName", expected: "prod-1" });
    s.drop.setConfirm({ checked: true });
    await s.drop.apply();
    s.drop.setConfirm({ typedName: "PROD-1" });
    await s.drop.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.drop.setConfirm({ typedName: " prod-1 " });
    await s.drop.apply();
    expect(s.api.ddlApply).toHaveBeenCalledOnce();
    expect(s.api.ddlApply.mock.calls[0]![2]).toMatchObject({ cascade: true });
    expect(s.api.ddlApply.mock.calls[0]![3]).toMatchObject({ expectedHash: "hc", confirmed: true, confirmedProduction: true });
  });

  it("cascade değişimi onayı sıfırlar", async () => {
    const s = setup();
    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.drop.setCascade(true);
    expect(s.drop.state()!.confirm.checked).toBe(false);
  });

  it("kilitli sunucu: kapı kapalı, gönderim yok; kayıt değişince kilit/onay yenilenir", async () => {
    const s = setup();
    s.servers.S1!.locked = true;
    await ready(s);
    expect(s.drop.gate()).toEqual({ kind: "blocked", reason: "locked" });
    s.drop.setConfirm({ checked: true });
    await s.drop.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.servers.S1!.locked = false;
    s.ctl.serverChanged("S1");
    expect(s.drop.state()).toMatchObject({ locked: false, confirm: { checked: false } });
    // Kayıt uygulama anında kilitlendiyse (arayüz henüz bilmiyor) yine gönderilmez.
    s.drop.setConfirm({ checked: true });
    s.servers.S1!.locked = true;
    await s.drop.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
  });

  it("Failed 2BP01: bağımlı nesne şeridi + detail; diyalog açık kalır, yeni önizleme istenir; cascade ile yeniden denenir", async () => {
    const s = setup();
    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "2BP01", position: null,
      error: err("query", "cannot drop schema app because other objects depend on it", "table app.t depends on schema app"),
    });
    await s.drop.apply();
    await flush();
    const st = s.drop.state()!;
    expect(st.phase).toBe("ready");
    expect(st.status).toEqual({ kind: "dependents", text: tdes.drop.dependents, detail: "table app.t depends on schema app" });
    expect(st.confirm.checked).toBe(false);
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(2);
    expect(s.refreshNode).not.toHaveBeenCalled();
    s.drop.setCascade(true);
    expect(s.drop.state()!.status).toBeNull();
  });

  it("ddlApply reddi: plan_changed → stale (uç durum); bağlantı → kapanır; diğer → şerit + yeni önizleme", async () => {
    const s = setup();
    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockRejectedValueOnce(err("plan_changed"));
    await s.drop.apply();
    expect(s.drop.state()).toMatchObject({ phase: "error", status: { kind: "stale", text: tdes.drop.stale } });
    await s.drop.apply();
    expect(s.api.ddlApply).toHaveBeenCalledOnce();

    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockRejectedValueOnce(err("not_connected", "bağlı değil"));
    await s.drop.apply();
    expect(s.drop.state()).toBeNull();
    expect(s.notify).toHaveBeenCalledWith("bağlı değil", "error");

    await ready(s);
    s.drop.setConfirm({ checked: true });
    const before = s.api.ddlPreview.mock.calls.length;
    s.api.ddlApply.mockRejectedValueOnce(err("locked"));
    await s.drop.apply();
    expect(s.drop.state()).toMatchObject({ phase: "ready", locked: true, status: { kind: "locked" } });
    expect(s.api.ddlPreview).toHaveBeenCalledTimes(before);
  });

  it("uygulama sırasında kapatma: ddl_cancel; geç başarı diyalog durumu yazmaz ama Gezgin/sekme güncellenir ve bildirilir", async () => {
    const s = setup();
    await s.ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: OBJ, refreshNodeId: null });
    await ready(s);
    s.drop.setConfirm({ checked: true });
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.drop.apply();
    s.drop.close();
    expect(s.api.ddlCancel).toHaveBeenCalledWith("A1");
    d.resolve(applied());
    await p;
    expect(s.drop.state()).toBeNull();
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.drop.done("app"), "success");
    expect(s.refreshNode).toHaveBeenCalledExactlyOnceWith("N-coll");
    expect(s.ctl.model("T1")!.status).toBe("gone");

    // Geç hata da durum yazmaz.
    await ready(s);
    s.drop.setConfirm({ checked: true });
    const e = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(e.promise);
    const q = s.drop.apply();
    s.drop.close();
    e.reject(err("query", "x"));
    await q;
    expect(s.drop.state()).toBeNull();
  });

  it("geç başarı yeni açılan diyaloğu kapatmaz; yine bildirilir", async () => {
    const s = setup();
    await ready(s);
    s.drop.setConfirm({ checked: true });
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.drop.apply();
    s.drop.close();
    await ready(s, { ...INPUT, object: { kind: "schema", oid: 200 }, name: "diger" });
    d.resolve(applied());
    await p;
    expect(s.notify).toHaveBeenCalledExactlyOnceWith(tdes.drop.done("app"), "success");
    expect(s.drop.state()).toMatchObject({ phase: "ready", input: { name: "diger" } });
  });

  it("stale ve kısmi silme Gezgini yeniler: plan_changed, not_found (yükleme/önizleme), committed > 0", async () => {
    const s = setup();
    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockRejectedValueOnce(err("plan_changed"));
    await s.drop.apply();
    expect(s.refreshNode).toHaveBeenCalledTimes(1);

    s.api.ddlLoad.mockRejectedValueOnce(err("not_found"));
    s.drop.open(INPUT);
    await flush();
    expect(s.refreshNode).toHaveBeenCalledTimes(2);

    s.api.ddlPreview.mockRejectedValueOnce(err("not_found"));
    s.drop.open(INPUT);
    await flush();
    expect(s.drop.state()!.phase).toBe("error");
    expect(s.refreshNode).toHaveBeenCalledTimes(3);

    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockResolvedValueOnce({ outcome: "failed", failedIndex: 1, committed: 1, sqlstate: "XX000", position: null, error: err("query", "x") });
    await s.drop.apply();
    expect(s.drop.state()).toMatchObject({ phase: "error", status: { kind: "stale" } });
    expect(s.refreshNode).toHaveBeenCalledTimes(4);
    expect(s.refreshNode).toHaveBeenLastCalledWith("N-coll");

    // Genel hata ve 2BP01 yenilemez.
    await ready(s);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockRejectedValueOnce(err("query", "y"));
    await s.drop.apply();
    expect(s.refreshNode).toHaveBeenCalledTimes(4);
  });

  it("cancelApply yalnızca uçuşta ddl_cancel ister", async () => {
    const s = setup();
    await ready(s);
    s.drop.cancelApply();
    expect(s.api.ddlCancel).not.toHaveBeenCalled();
    s.drop.setConfirm({ checked: true });
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    const p = s.drop.apply();
    s.drop.cancelApply();
    expect(s.api.ddlCancel).toHaveBeenCalledExactlyOnceWith("A1");
    d.resolve({ outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "57014", position: null, error: err("query", "iptal") });
    await p;
    expect(s.drop.state()).toMatchObject({ phase: "ready", status: { kind: "other" } });
  });

  it("sunucu oturumları kapanınca diyalog kapanır", async () => {
    const s = setup();
    await ready(s);
    s.ctl.serverSessionsClosed("S2", false);
    expect(s.drop.state()).not.toBeNull();
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.drop.state()).toBeNull();
  });
});

const DB: ObjectRef = { kind: "database", oid: 5 };
const DB_INPUT: DropInput = { serverId: "S1", databaseOid: null, object: DB, refreshNodeId: "dbs", name: "app" };
const ROLE_INPUT: DropInput = { serverId: "S1", databaseOid: null, object: { kind: "role", oid: 16384 }, refreshNodeId: "roles", name: "u" };

describe("drop-controller Plan 15: FORCE ve sunucu kapsamı", () => {
  it("database: force varsayılan kapalı; setForce sürümü ilerletir, onayı sıfırlar, önizlemeyi force ile yeniden ister; en son istek kazanır", async () => {
    const s = setup();
    const st = await ready(s, DB_INPUT);
    expect(st.force).toBe(false);
    expect(s.api.ddlLoad).toHaveBeenCalledWith("S1", null, DB);
    expect(s.api.ddlPreview.mock.calls[0]![1]).toBeNull();
    const v0 = s.drop.state()!.version;
    s.drop.setConfirm({ checked: true });
    const a = deferred<PlanView>();
    const b = deferred<PlanView>();
    s.api.ddlPreview.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    s.drop.setForce(true);
    expect(s.drop.state()).toMatchObject({ force: true, version: v0 + 1, confirm: { checked: false } });
    s.drop.setForce(false);
    expect(s.api.ddlPreview.mock.calls.map((c) => (c[2] as Extract<PlanRequest, { op: "drop" }>).force)).toEqual([false, true, false]);
    expect(s.api.ddlPreview.mock.calls.every((c) => (c[2] as Extract<PlanRequest, { op: "drop" }>).cascade === false)).toBe(true);
    b.resolve(view("data_loss", "h-last"));
    await flush();
    a.resolve(view("data_loss", "h-old"));
    await flush();
    expect(s.drop.state()).toMatchObject({ force: false, pending: false });
    expect(s.drop.state()!.view?.hash).toBe("h-last");
    // aynı değer: etkisiz
    const calls = s.api.ddlPreview.mock.calls.length;
    s.drop.setForce(false);
    expect(s.api.ddlPreview.mock.calls.length).toBe(calls);
  });

  it("database: force=true uygulamada da gider; databaseOid null; başarıda objectDropped null ile", async () => {
    const s = setup();
    await s.ctl.openAlter({ tabId: "D1", serverId: "S1", databaseOid: null, object: DB, refreshNodeId: "dbs" });
    await ready(s, DB_INPUT);
    s.drop.setForce(true);
    await flush();
    s.drop.setConfirm({ checked: true });
    await s.drop.apply();
    expect(s.api.ddlApply).toHaveBeenCalledOnce();
    expect(s.api.ddlApply.mock.calls[0]![1]).toBeNull();
    expect(s.api.ddlApply.mock.calls[0]![2]).toMatchObject({ op: "drop", cascade: false, force: true });
    expect(s.api.ddlApply.mock.calls[0]![3].password).toBeNull();
    expect(s.ctl.model("D1")!.status).toBe("gone");
  });

  it("dropOptionsFor'da sunulmayan kutular denetleyicide de etkisiz (database cascade, role cascade/force, schema force)", async () => {
    let s = setup();
    expect(s.drop.options()).toEqual({ cascade: false, force: false }); // kapalı diyalog
    await ready(s, DB_INPUT);
    expect(s.drop.options()).toEqual({ cascade: false, force: true });
    s.drop.setCascade(true);
    expect(s.drop.state()!.cascade).toBe(false);
    s = setup();
    await ready(s, ROLE_INPUT);
    expect(s.drop.options()).toEqual({ cascade: false, force: false });
    s.drop.setCascade(true);
    s.drop.setForce(true);
    expect(s.drop.state()).toMatchObject({ cascade: false, force: false });
    expect(s.api.ddlPreview).toHaveBeenCalledOnce();
    s = setup();
    await ready(s);
    expect(s.drop.options()).toEqual({ cascade: true, force: false });
    s.drop.setForce(true);
    expect(s.drop.state()!.force).toBe(false);
  });

  it("role: Failed 2BP01 şeridi CASCADE önermez (REASSIGN/DROP OWNED)", async () => {
    const s = setup();
    await ready(s, ROLE_INPUT);
    s.drop.setConfirm({ checked: true });
    s.api.ddlApply.mockResolvedValueOnce({
      outcome: "failed", failedIndex: 0, committed: 0, sqlstate: "2BP01", position: null,
      error: err("query", "role u cannot be dropped because some objects depend on it", "owner of table app.t"),
    });
    await s.drop.apply();
    await flush();
    expect(s.api.ddlApply).toHaveBeenCalledOnce();
    expect(s.drop.state()!.status).toEqual({ kind: "dependents", text: tdes.drop.dependentsNoCascade, detail: "owner of table app.t" });
  });
});
