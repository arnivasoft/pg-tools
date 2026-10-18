import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tm } from "../../app/strings-maintenance";
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { MaintenanceProgress } from "../../ipc/bindings/MaintenanceProgress";
import type { MaintenanceTarget } from "../../ipc/bindings/MaintenanceTarget";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Risk } from "../../ipc/bindings/Risk";
import { createMaintenanceController, type MaintenanceControllerDeps } from "./maintenance-controller";

const OBJ: ObjectRef = { kind: "table", oid: 42 };
const ddlTarget = (p: Partial<DdlTarget> = {}): DdlTarget =>
  ({ serverId: "S1", databaseOid: 5, object: OBJ, create: null, focus: null, refreshNodeId: null, ...p });
const target = (p: Partial<MaintenanceTarget> = {}): MaintenanceTarget => ({
  kind: "table", schema: "s", name: "t", table: null, partitioned: false, system: false, clusterIndex: null,
  indexes: ["i1"], editable: true, unsupported: [], ...p,
});
const view = (hash: string, risk: Risk = "safe", p: Partial<PlanView> = {}): PlanView => ({
  statements: [{ sql: "VACUUM s.t", risk, transactional: false, note: null, redacted: false }],
  warnings: [], errors: [], hash, risk, empty: false, ...p,
});
const applied = (p: Partial<Extract<ApplyOutcome, { outcome: "applied" }>> = {}): ApplyOutcome =>
  ({ outcome: "applied", statements: 1, elapsedMs: 1234, created: null, ...p });
const failed = (sqlstate: string, message = "hata", hint: string | null = null): ApplyOutcome => ({
  outcome: "failed", failedIndex: 0, committed: 0, sqlstate, position: null,
  error: { code: "query", message, detail: null, hint },
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function setup(info: { versionNum: number | null; locked: boolean } | null = { versionNum: 170000, locked: false }) {
  let ids = 0;
  let hashN = 0;
  const api = {
    ddlTargetOf: vi.fn(async (_n: string): Promise<DdlTarget> => ddlTarget()),
    ddlLoadMaintenanceTarget: vi.fn(async (_s: string, _db: number, _o: ObjectRef): Promise<MaintenanceTarget> => target()),
    ddlPreview: vi.fn(async (_s: string, _db: number | null, r: PlanRequest): Promise<PlanView> => {
      const op = r.op === "maintenance" ? r.operation : null;
      const destructive = op?.op === "vacuum" && op.full;
      return view(`h${++hashN}`, destructive ? "destructive" : "safe");
    }),
    ddlApply: vi.fn(async (_s: string, _db: number | null, _r: PlanRequest, _o: ApplyOptions): Promise<ApplyOutcome> => applied()),
    ddlCancel: vi.fn(async (_id: string) => null),
    ddlMaintenanceProgress: vi.fn(async (_s: string, _db: number, _id: string): Promise<MaintenanceProgress | null> => null),
  };
  const notify = vi.fn();
  const onClosed = vi.fn();
  const deps: MaintenanceControllerDeps = {
    api: api as unknown as MaintenanceControllerDeps["api"],
    serverInfo: () => info,
    notify,
    onClosed,
    newId: () => `A${++ids}`,
    now: () => Date.now(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  const ctl = createMaintenanceController(deps);
  return { ctl, api, notify, onClosed, setInfo: (i: typeof info) => { info = i; } };
}
type S = ReturnType<typeof setup>;

const flush = () => vi.advanceTimersByTimeAsync(0);
/** Önizleme gecikmesi (debounce) dahil bekler. */
const settle = () => vi.advanceTimersByTimeAsync(200);
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

async function ready(s: S) {
  s.ctl.open("N1");
  await settle();
  return s.ctl.state()!;
}

describe("maintenance-controller yükleme", () => {
  it("ddlTargetOf → ddlLoadMaintenanceTarget → varsayılan işlem ve önizleme", async () => {
    const s = setup();
    const st = await ready(s);
    expect(s.api.ddlTargetOf).toHaveBeenCalledWith("N1");
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledWith("S1", 5, OBJ);
    expect(st.phase).toBe("ready");
    expect(st.operation?.op).toBe("vacuum");
    const req = s.api.ddlPreview.mock.calls[0]![2];
    expect(req).toEqual({ op: "maintenance", object: OBJ, target: target(), operation: st.operation });
    expect(st.view?.hash).toBe("h1");
    expect(s.ctl.canApply()).toBe(true);
  });

  it("veritabanı düğümü: databaseOid object.oid'dir", async () => {
    const s = setup();
    const db: ObjectRef = { kind: "database", oid: 77 };
    s.api.ddlTargetOf.mockResolvedValueOnce(ddlTarget({ databaseOid: null, object: db }));
    s.api.ddlLoadMaintenanceTarget.mockResolvedValueOnce(target({ kind: "database", schema: null, indexes: [] }));
    await ready(s);
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledWith("S1", 77, db);
  });

  it("yükleme hatası → ileti ve yeniden dene", async () => {
    const s = setup();
    s.api.ddlLoadMaintenanceTarget.mockRejectedValueOnce({ code: "query", message: "olmadı", detail: null, hint: null });
    s.ctl.open("N1");
    await settle();
    expect(s.ctl.state()!.phase).toBe("loadError");
    expect(s.ctl.state()!.loadError).toContain("olmadı");
    expect(s.ctl.canApply()).toBe(false);
    s.ctl.retry();
    await settle();
    expect(s.ctl.state()!.phase).toBe("ready");
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledTimes(2);
  });

  it("sürüm bilinmiyorsa (null) istemci kapısı yok: SKIP_LOCKED etkin", async () => {
    const s = setup({ versionNum: null, locked: false });
    await ready(s);
    expect(s.ctl.optionStates().skipLocked!.enabled).toBe(true);
    const s10 = setup({ versionNum: 100000, locked: false });
    await ready(s10);
    expect(s10.ctl.optionStates().skipLocked!.enabled).toBe(false);
  });
});

describe("maintenance-controller önizleme ve kapı", () => {
  it("seçenek değişince önizleme yenilenir; eski hash kullanılmaz", async () => {
    const s = setup();
    await ready(s);
    s.ctl.setOptions({ verbose: true });
    expect(s.ctl.canApply()).toBe(false); // bayat önizleme
    await s.ctl.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    await settle();
    expect(s.ctl.state()!.view?.hash).toBe("h2");
    await s.ctl.apply();
    expect(s.api.ddlApply.mock.calls[0]![3].expectedHash).toBe("h2");
    const req = s.api.ddlApply.mock.calls[0]![2];
    expect(req.op === "maintenance" && req.operation.op === "vacuum" && req.operation.verbose).toBe(true);
  });

  it("Destructive: onaysız uygulama kapalı; onayla confirmed=true", async () => {
    const s = setup();
    await ready(s);
    s.ctl.setOptions({ full: true });
    await settle();
    expect(s.ctl.state()!.view?.risk).toBe("destructive");
    expect(s.ctl.canApply()).toBe(false);
    await s.ctl.apply();
    expect(s.api.ddlApply).not.toHaveBeenCalled();
    s.ctl.setConfirmed(true);
    expect(s.ctl.canApply()).toBe(true);
    await s.ctl.apply();
    expect(s.api.ddlApply.mock.calls[0]![3]).toMatchObject({ confirmed: true, confirmedProduction: false, password: null, lockTimeoutMs: 5000 });
  });

  it("kilit canlıdır: kilitlenince kapı kapanır, açılınca kendiliğinden kalkar", async () => {
    const s = setup({ versionNum: 170000, locked: true });
    await ready(s);
    expect(s.ctl.locked()).toBe(true);
    expect(s.ctl.canApply()).toBe(false);
    expect(s.ctl.gate()).toEqual({ ok: false, reason: "locked" });
    s.setInfo({ versionNum: 170000, locked: false });
    expect(s.ctl.locked()).toBe(false);
    expect(s.ctl.canApply()).toBe(true);
    s.setInfo({ versionNum: 170000, locked: true });
    expect(s.ctl.canApply()).toBe(false);
  });

  it("locked reddi: ayrı bildirim yok, kilit canlı değerden; önizleme yenilenir", async () => {
    const s = setup();
    await ready(s);
    s.api.ddlApply.mockRejectedValueOnce({ code: "locked", message: "x", detail: null, hint: null });
    s.setInfo({ versionNum: 170000, locked: true });
    await s.ctl.apply();
    await settle();
    const st = s.ctl.state()!;
    expect(st.phase).toBe("ready");
    expect(st.notice).toBeNull();
    expect(st.result).toBeNull();
    expect(s.ctl.locked()).toBe(true);
    s.setInfo({ versionNum: 170000, locked: false });
    expect(s.ctl.canApply()).toBe(true);
  });

  it("işlem değişimi varsayılan seçenekleri kurar ve önizler", async () => {
    const s = setup();
    await ready(s);
    s.ctl.selectOp("cluster");
    await settle();
    const op = s.ctl.state()!.operation!;
    expect(op).toEqual({ op: "cluster", verbose: false, usingIndex: "i1" });
  });
});

describe("maintenance-controller uygulama", () => {
  it("uygulama sırasında aynı applyId ile yoklama başlar, sonuçta durur", async () => {
    const s = setup();
    await ready(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    void s.ctl.apply();
    await flush();
    expect(s.ctl.state()!.phase).toBe("applying");
    const id = s.api.ddlApply.mock.calls[0]![3].applyId;
    s.api.ddlMaintenanceProgress.mockResolvedValueOnce({ phase: "scanning heap", done: 5, total: 10, unit: "blocks" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(s.api.ddlMaintenanceProgress).toHaveBeenCalledWith("S1", 5, id);
    expect(s.ctl.state()!.progress?.done).toBe(5);
    d.resolve(applied());
    await flush();
    const calls = s.api.ddlMaintenanceProgress.mock.calls.length;
    await vi.advanceTimersByTimeAsync(5000);
    expect(s.api.ddlMaintenanceProgress.mock.calls.length).toBe(calls);
    expect(s.ctl.state()!.result?.kind).toBe("applied");
  });

  it("iptal → ddlCancel(applyId) ve 'İptal edildi'", async () => {
    const s = setup();
    await ready(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    void s.ctl.apply();
    await flush();
    const id = s.api.ddlApply.mock.calls[0]![3].applyId;
    s.ctl.cancelApply();
    expect(s.api.ddlCancel).toHaveBeenCalledWith(id);
    expect(s.ctl.state()!.cancelRequested).toBe(true);
    d.resolve(failed("57014", "canceling statement due to user request"));
    await flush();
    const st = s.ctl.state()!;
    expect(st.result?.kind).toBe("cancelled");
    expect(st.phase).toBe("ready");
    expect(tm.status[st.result!.kind]).toBe("İptal edildi");
  });

  it("REINDEX CONCURRENTLY iptali *_ccnew uyarısı taşır", async () => {
    const s = setup();
    await ready(s);
    s.ctl.selectOp("reindex");
    await settle();
    expect(s.ctl.state()!.operation).toMatchObject({ op: "reindex", concurrently: true });
    s.api.ddlApply.mockResolvedValueOnce(failed("57014"));
    await s.ctl.apply();
    expect(s.ctl.state()!.result).toMatchObject({ kind: "cancelled", ccnew: true });
  });

  it("başarısızlık: hata + ipucu; hedef yeniden yüklenmez", async () => {
    const s = setup();
    await ready(s);
    s.api.ddlApply.mockResolvedValueOnce(failed("55P03", "lock timeout", "kilitli"));
    await s.ctl.apply();
    expect(s.ctl.state()!.result).toMatchObject({ kind: "failed", error: "lock timeout", hint: "kilitli" });
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledTimes(1);
  });

  it("başarıdan sonra hedef yeniden yüklenir ve yeni hedefle önizlenir", async () => {
    const s = setup();
    await ready(s);
    s.ctl.selectOp("cluster");
    await settle();
    const fresh = target({ clusterIndex: "i1" });
    s.api.ddlLoadMaintenanceTarget.mockResolvedValueOnce(fresh);
    await s.ctl.apply();
    await settle();
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledTimes(2);
    expect(s.ctl.state()!.target).toEqual(fresh);
    const last = s.api.ddlPreview.mock.calls.at(-1)![2];
    expect(last.op === "maintenance" && last.target).toEqual(fresh);
    expect(s.ctl.state()!.operation?.op).toBe("cluster");
  });

  it("plan_changed reddi → hedef yeniden yüklenir ve bildirilir", async () => {
    const s = setup();
    await ready(s);
    s.api.ddlApply.mockRejectedValueOnce({ code: "plan_changed", message: "x", detail: null, hint: null });
    await s.ctl.apply();
    await settle();
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledTimes(2);
    expect(s.ctl.state()!.notice).toBe(tm.planChanged);
    expect(s.ctl.state()!.phase).toBe("ready");
  });

  it("WARNING özeti doğru", async () => {
    const s = setup();
    await ready(s);
    s.api.ddlApply.mockResolvedValueOnce(applied({
      messages: [
        { severity: "INFO", code: null, message: "a", detail: null, hint: null },
        { severity: "WARNING", code: "42501", message: "skipping", detail: null, hint: null },
        { severity: "WARNING", code: "42501", message: "skipping 2", detail: null, hint: null },
      ],
    }));
    await s.ctl.apply();
    expect(s.ctl.state()!.result).toMatchObject({ kind: "applied", warnings: 2, elapsedMs: 1234 });
    expect(s.ctl.state()!.result!.messages).toHaveLength(3);
  });
});

describe("maintenance-controller kapatma", () => {
  it("uygulama sürerken open() yok sayılır (uçuştaki işlem iptal edilmez)", async () => {
    const s = setup();
    await ready(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    void s.ctl.apply();
    await flush();
    s.ctl.open("N2");
    expect(s.api.ddlCancel).not.toHaveBeenCalled();
    expect(s.ctl.state()!.nodeId).toBe("N1");
    expect(s.ctl.state()!.phase).toBe("applying");
    d.resolve(applied());
    await flush();
    expect(s.ctl.state()!.result?.kind).toBe("applied");
  });

  it("çalışırken kapatma onay ister; Hayır açık bırakır, Evet iptal edip sonucu bekler", async () => {
    const s = setup();
    await ready(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    void s.ctl.apply();
    await flush();
    s.ctl.requestClose();
    expect(s.ctl.state()!.closeAsk).toBe(true);
    expect(s.onClosed).not.toHaveBeenCalled();
    s.ctl.answerClose(false);
    expect(s.ctl.state()!.closeAsk).toBe(false);
    expect(s.api.ddlCancel).not.toHaveBeenCalled();
    s.ctl.requestClose();
    s.ctl.answerClose(true);
    expect(s.api.ddlCancel).toHaveBeenCalledTimes(1);
    expect(s.ctl.state()!.closing).toBe(true);
    d.resolve(failed("57014"));
    await flush();
    expect(s.ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
  });

  it("boştayken kapatma hemen kapatır", async () => {
    const s = setup();
    await ready(s);
    s.ctl.requestClose();
    expect(s.ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
  });

  it("kapalıyken gelen sonuç yok sayılır; yoklama durur", async () => {
    const s = setup();
    await ready(s);
    const d = deferred<ApplyOutcome>();
    s.api.ddlApply.mockReturnValueOnce(d.promise);
    void s.ctl.apply();
    await flush();
    s.ctl.close();
    expect(s.api.ddlCancel).toHaveBeenCalledTimes(1);
    const calls = s.api.ddlMaintenanceProgress.mock.calls.length;
    await vi.advanceTimersByTimeAsync(3000);
    expect(s.api.ddlMaintenanceProgress.mock.calls.length).toBe(calls);
    d.resolve(applied());
    await flush();
    expect(s.ctl.state()).toBeNull();
    expect(s.api.ddlLoadMaintenanceTarget).toHaveBeenCalledTimes(1);
    // Yeni açılış eski sonuçtan etkilenmez.
    s.ctl.open("N1");
    await settle();
    expect(s.ctl.state()!.result).toBeNull();
  });
});
