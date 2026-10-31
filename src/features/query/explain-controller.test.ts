import { describe, expect, it, vi } from "vitest";
import pg16 from "./fixtures/plan-pg16.json?raw";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ExplainResult } from "../../ipc/bindings/ExplainResult";
import { createExplainController, type ExplainControllerDeps } from "./explain-controller";
import { DEFAULT_EXPLAIN } from "./explain-model";
import type { TabModel } from "./query-controller";
import { initialRun } from "./run-model";
import { tplan } from "./strings-plan";

const err = (code: string, message = code): ErrorPayload => ({ code, message, detail: null, hint: null }) as ErrorPayload;
const ok = (over: Partial<ExplainResult> = {}): ExplainResult => ({
  json: pg16,
  notices: [],
  elapsedMs: 7,
  rolledBack: false,
  sql: "EXPLAIN (FORMAT JSON, SUMMARY) select 1",
  note: null,
  ...over,
});

function setup(over: Partial<ExplainControllerDeps> = {}) {
  const models = new Map<string, TabModel>();
  const tab = (id: string, p: Partial<TabModel> = {}) =>
    models.set(id, {
      serverId: "S1",
      serverName: "srv",
      databaseOid: 1,
      database: "db",
      relation: null,
      options: { mode: "autocommit", statementTimeoutMs: 0 },
      session: "sess",
      run: { ...initialRun(), status: "idle" },
      serverGone: false,
      notes: [],
      op: null,
      txSince: null,
      last: null,
      selectedStmt: null,
      historyKey: 0,
      checkOff: false,
      ...p,
    } as TabModel);
  tab("a");
  const ops: (string | null)[] = [];
  const update = (id: string, f: (m: TabModel) => TabModel) => {
    const m = models.get(id);
    if (!m) return;
    const next = f(m);
    if (next.op !== m.op) ops.push(next.op);
    models.set(id, next);
  };
  const panes: string[] = [];
  const api = {
    qtExplain: vi.fn(async () => ok()),
    qtAnalyze: vi.fn(async () => []),
    qtHistoryPushMany: vi.fn(async () => 1),
  };
  const deps: ExplainControllerDeps = {
    api,
    ctl: {
      model: (id: string) => models.get(id),
      update,
      dispatch: vi.fn(),
      patch: (id: string, p: Partial<TabModel>) => update(id, (m) => ({ ...m, ...p })),
      ensureSession: vi.fn(async (id: string) => models.get(id)?.session ?? null),
      sessionLost: vi.fn(),
      refreshTx: vi.fn(async () => {}),
    } as unknown as ExplainControllerDeps["ctl"],
    snapshot: () => ({ text: "select 1", scope: { type: "all" } as never }),
    confirm: vi.fn(async () => true),
    production: () => false,
    version: () => 160000,
    showPane: (id, p) => panes.push(`${id}:${p}`),
    route: (e) => (e.code === "busy" ? "toast" : e.code === "not_found" ? "lost" : "message"),
    hint: () => null,
    notifyBusy: vi.fn(),
    payload: (e) => e as ErrorPayload,
    newId: () => "H1",
    now: () => 42,
    ...over,
  };
  const ex = createExplainController(deps);
  return { ex, deps, api, models, tab, ops, panes };
}

describe("explain denetleyicisi", () => {
  it("başarı: plan Plan sekmesine düşer, not/NOTICE Mesajlar'a, geçmişe EXPLAIN metni yazılır", async () => {
    const s = setup();
    s.api.qtExplain.mockResolvedValueOnce(
      ok({ note: "Geri alındı.", rolledBack: true, notices: [{ severity: "NOTICE", code: "00000", message: "n1", detail: null, hint: "h" }] }),
    );
    await s.ex.explain("a", "plain");
    const p = s.ex.plan("a");
    expect(p?.root.nodeType).toBe("Limit");
    expect(p).toMatchObject({ source: "explain", rolledBack: true, elapsedMs: 7 });
    expect(s.models.get("a")!.notes).toEqual([
      { level: "info", text: "Geri alındı." },
      { level: "info", text: "NOTICE: n1", hint: "h" },
    ]);
    expect(s.panes).toEqual(["a:plan"]);
    expect(s.ops).toEqual(["explain", null]); // meşgul işareti istek boyunca
    expect(s.api.qtHistoryPushMany).toHaveBeenCalledWith([
      { id: "H1", sql: "EXPLAIN (FORMAT JSON, SUMMARY) select 1", serverId: "S1", serverName: "srv", database: "db", atMs: 42, elapsedMs: 7, ok: true },
    ]);
    expect(s.deps.ctl.refreshTx).toHaveBeenCalledWith("a");
  });

  it("F7 ANALYZE'sız, Shift+F7 ANALYZE ile; sekme seçenekleri gönderilir", async () => {
    const s = setup();
    s.ex.setOption("a", "verbose", true);
    await s.ex.explain("a", "plain");
    await s.ex.explain("a", "analyze");
    const sent = s.api.qtExplain.mock.calls.map((c) => (c as unknown[])[3]);
    expect(sent[0]).toEqual({ ...DEFAULT_EXPLAIN, verbose: true });
    expect(sent[1]).toEqual({ ...DEFAULT_EXPLAIN, verbose: true, analyze: true });
  });

  it("onay: confirmation_required → diyalog → confirmed ile yeniden", async () => {
    const s = setup({ production: () => true });
    s.api.qtExplain.mockRejectedValueOnce(err("confirmation_required"));
    await s.ex.explain("a", "analyze");
    expect(s.deps.confirm).toHaveBeenCalledTimes(1);
    expect((s.deps.confirm as ReturnType<typeof vi.fn>).mock.calls[0][1]).toBe(true);
    expect(s.api.qtExplain.mock.calls.map((c) => (c as unknown[])[4])).toEqual([false, true]);
    expect(s.ex.plan("a")).not.toBeNull();
  });

  it("meşgul sekmede (çalışıyor ya da başka işlem) istek yapılmaz", async () => {
    const s = setup();
    s.tab("a", { op: "export" });
    await s.ex.explain("a", "plain");
    s.tab("a", { run: { ...initialRun(), status: "running" } as never });
    await s.ex.explain("a", "plain");
    s.tab("a", { serverGone: true });
    await s.ex.explain("a", "plain");
    expect(s.api.qtExplain).not.toHaveBeenCalled();
  });

  it("editör başka sekmedeyse (anlık görüntü yok) istek yapılmaz ve meşgul işareti konmaz", async () => {
    const s = setup({ snapshot: () => null });
    await s.ex.explain("a", "plain");
    expect(s.api.qtExplain).not.toHaveBeenCalled();
    expect(s.ops).toEqual([]);
  });

  it("hata Mesajlar'a gider, önceki plan korunur", async () => {
    const s = setup();
    await s.ex.explain("a", "plain");
    const before = s.ex.plan("a");
    s.api.qtExplain.mockRejectedValueOnce(err("explain_scope", "Plan için tek bir ifade seçin."));
    await s.ex.explain("a", "plain");
    expect(s.ex.plan("a")).toBe(before);
    expect(s.models.get("a")!.notes).toEqual([{ level: "error", text: tplan.explainFailed("Plan için tek bir ifade seçin."), hint: null }]);
    expect(s.panes.at(-1)).toBe("a:messages");
    expect(s.models.get("a")!.op).toBeNull();
  });

  it("busy hatası bildirim olarak gösterilir; oturum kaybı denetleyiciye iletilir", async () => {
    const s = setup();
    s.api.qtExplain.mockRejectedValueOnce(err("busy"));
    await s.ex.explain("a", "plain");
    expect(s.deps.notifyBusy).toHaveBeenCalled();
    s.api.qtExplain.mockRejectedValueOnce(err("not_found"));
    await s.ex.explain("a", "plain");
    expect(s.deps.ctl.sessionLost).toHaveBeenCalledWith("a");
    expect(s.ex.plan("a")).toBeNull();
  });

  it("onay reddedilirse not yok, plan değişmez, meşgul işareti kalkar", async () => {
    const s = setup({ confirm: vi.fn(async () => false) });
    s.api.qtExplain.mockRejectedValueOnce(err("confirmation_required"));
    await s.ex.explain("a", "analyze");
    expect(s.ex.plan("a")).toBeNull();
    expect(s.models.get("a")!.notes).toEqual([]);
    expect(s.models.get("a")!.op).toBeNull();
  });

  it("seçenekler sekme başına hatırlanır", () => {
    const s = setup();
    s.tab("b");
    s.ex.setOption("a", "analyze", true);
    expect(s.ex.options("a").analyze).toBe(true);
    expect(s.ex.options("b")).toEqual(DEFAULT_EXPLAIN);
    s.ex.prune(new Set(["b"]));
    expect(s.ex.options("a")).toEqual(DEFAULT_EXPLAIN);
  });

  it("ızgaradan açma: geçerli JSON Plan sekmesinde, geçersizde not ve önceki plan korunur", () => {
    const s = setup();
    expect(s.ex.openJson("a", pg16)).toBe(true);
    const p = s.ex.plan("a");
    expect(p).toMatchObject({ source: "grid", sql: null });
    expect(s.ex.openJson("a", "{\"x\":1}")).toBe(false);
    expect(s.ex.plan("a")).toBe(p);
    expect(s.models.get("a")!.notes).toEqual([{ level: "error", text: tplan.errNoPlan }]);
    expect(s.panes).toEqual(["a:plan", "a:messages"]);
  });

  it("sunucuya varan istek ızgaranın imlecini kapatır (\"Daha fazla\" kapanır); kilit öncesi retlerde kapanmaz", async () => {
    const s = setup();
    const dispatch = s.deps.ctl.dispatch as ReturnType<typeof vi.fn>;
    await s.ex.explain("a", "plain");
    expect(dispatch).toHaveBeenCalledWith("a", { type: "optionsChanged" });
    dispatch.mockClear();
    s.api.qtExplain.mockRejectedValueOnce(err("syntax", "sözdizimi"));
    await s.ex.explain("a", "plain");
    expect(dispatch).toHaveBeenCalledWith("a", { type: "optionsChanged" });
    dispatch.mockClear();
    for (const code of ["busy", "explain_already", "locked"]) {
      s.api.qtExplain.mockRejectedValueOnce(err(code));
      await s.ex.explain("a", "plain");
    }
    s.api.qtExplain.mockRejectedValueOnce(err("confirmation_required"));
    (s.deps.confirm as ReturnType<typeof vi.fn>).mockResolvedValueOnce(false);
    await s.ex.explain("a", "analyze");
    expect(dispatch).not.toHaveBeenCalled();
  });
});
