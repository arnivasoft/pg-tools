import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tdata } from "../../app/strings-data";
import type { DataColumn } from "../../ipc/bindings/DataColumn";
import type { DataOutcome } from "../../ipc/bindings/DataOutcome";
import type { DataSelectResult } from "../../ipc/bindings/DataSelectResult";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { IpcError } from "../../ipc/errors";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import { createTabsStore } from "../query/tabs-store";
import type { DataApi } from "./data-commands";
import { ROW_CAP, createDataController, type DataControllerDeps } from "./data-controller";
import { addRow, newRowSetCell, setCell, toggleDelete } from "./data-edit-model";
import { createDataOpener } from "./data-opener";

// Fikstürler yapay veridir. Ayırt edici işaretler: sızıntı testi bunları günlük/bildirim argümanlarında arar.
const MARK = { key: "KEYMARK", cell: "CELLMARK", ver: "VERMARK", sql: "SQLMARK", filter: "FILTERMARK" };
const SQL = `SELECT ${MARK.sql} FROM ONLY t`;
const obj: ObjectRef = { kind: "table", oid: 42 };

function col(attnum: number, name: string): DataColumn {
  return {
    attnum, name, typeOid: 25, typeDisplay: "text", notNull: false, hasDefault: false, identity: "none", generated: false,
    insertable: true, updatable: true,
  };
}

function source(over: Partial<DataSource> = {}): DataSource {
  return {
    object: obj, schema: "public", name: "t", relkind: "table", columns: [col(1, "id"), col(2, "name"), col(3, "note")],
    key: { source: "primary_key", name: "t_pkey", columns: ["id"] }, canSelect: true, canInsert: true, canUpdate: true,
    canDelete: true, locked: false, editable: true, readOnlyReason: null, fingerprint: "fp1", ...over,
  };
}

const row = (i: number): (string | null)[] => [`${MARK.key}${i}`, `${MARK.cell}${i}`, null, `${MARK.ver}${i}`];
const rows = (from: number, n: number) => Array.from({ length: n }, (_, k) => row(from + k));
const columns = (n = 4) => Array.from({ length: n }, (_, i) => ({ name: `c${i}`, typeName: "text", typeOid: 25, typeDisplay: "text" }));

const started = (n = 4): QueryEvent => ({ type: "started", stmtIndex: 0, kind: "select", columns: columns(n), charOffset: 0 });
const rowsEv = (r: (string | null)[][], more: boolean): QueryEvent => ({ type: "rows", stmtIndex: 0, rows: r, more });
const done: QueryEvent = { type: "done", stmtIndex: 0, tag: "SELECT", rowsAffected: null, rowsSent: 0, truncated: false, elapsedMs: 1, notices: [] };

const selectOk: DataSelectResult = { result: "ok", select: { sql: SQL, versionIndex: 3, hasVersion: true } };

const view = (p: Partial<PlanView> = {}): PlanView => ({
  statements: [{ sql: `UPDATE t SET name = '${MARK.cell}'`, risk: "safe", transactional: true, note: null, redacted: false }],
  warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...p,
});

const ipcErr = (code: string, message = `msg ${code}`) => new IpcError({ code, message, detail: null, hint: null });

const failed = (p: Partial<Extract<DataOutcome, { outcome: "failed" }>> = {}): DataOutcome => ({
  outcome: "failed", failedIndex: 0, sqlstate: null, position: null, conflict: false, outcomeUnknown: false,
  error: { code: "query", message: "pg failure", detail: null, hint: null }, ...p,
});

type RunFn = DataApi["qtRun"];

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
}

const flush = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};

function fakeApi() {
  let sess = 0;
  const api = {
    ddlTargetOf: vi.fn(),
    dataLoad: vi.fn(async () => source()),
    dataSelect: vi.fn(async (): Promise<DataSelectResult> => selectOk),
    qtOpenReadonly: vi.fn(async () => `sess${++sess}`),
    qtRun: vi.fn<RunFn>(async (_s, _sql, _scope, _limit, _c, onEvent) => {
      onEvent(started());
      onEvent(rowsEv(rows(0, 5), false));
      onEvent(done);
      return null;
    }),
    qtMore: vi.fn<DataApi["qtMore"]>(async () => null),
    qtCancel: vi.fn(async () => null),
    qtClose: vi.fn(async () => null),
    dataPreview: vi.fn<DataApi["dataPreview"]>(async () => view()),
    dataApply: vi.fn<DataApi["dataApply"]>(async () => ({ outcome: "applied", statements: 1, elapsedMs: 1, results: [] })),
    dataCancel: vi.fn(async () => null),
    refreshServers: vi.fn(async () => []),
    connect: vi.fn(async () => true),
  };
  return api;
}

function setup(info: { locked?: boolean; production?: boolean } = {}) {
  const api = fakeApi();
  let server = { name: "srv", locked: false, production: false, ...info };
  const timers: { fn: () => void; live: boolean }[] = [];
  let ids = 0;
  const deps: DataControllerDeps = {
    api: api as unknown as DataApi,
    serverInfo: (id) => (id === "S1" ? server : null),
    newId: () => `apply-${++ids}`,
    timer: {
      set: (fn) => {
        const h = { fn, live: true };
        timers.push(h);
        return h;
      },
      clear: (h) => void ((h as { live: boolean }).live = false),
    },
  };
  const ctl = createDataController(deps);
  /** Bekleyen debounce zamanlayıcılarını çalıştırır. */
  const fire = async () => {
    for (const h of timers.splice(0)) if (h.live) h.fn();
    await flush();
  };
  const open = async () => {
    ctl.openTab({ tabId: "g1", serverId: "S1", databaseOid: 5, object: obj });
    await flush();
  };
  return {
    api, ctl, fire, open, timers,
    m: () => ctl.model("g1")!,
    setServer: (p: Partial<typeof server>) => (server = { ...server, ...p }),
  };
}

const T = (value: string) => ({ kind: "text" as const, value });

describe("data-controller", () => {
  let spies: ReturnType<typeof vi.spyOn>[] = [];
  beforeEach(() => {
    spies = (["log", "info", "warn", "error", "debug", "trace"] as const).map((k) => vi.spyOn(console, k));
  });
  afterEach(() => {
    for (const s of spies) s.mockRestore();
  });

  it("açma: databaseOid null, focus dolu, tür uygunsuz → none toast ve sekme yok; kilitli sunucuda açılır", async () => {
    const s = emptyTree();
    const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
    const add = (id: string, parent: string, kind: NodeKind, label: string, extra: Partial<TreeNode> = {}) => {
      s.nodes.set(id, { id, kind, label, detail: null, hasChildren: false, serverId: null, flags, color: null, tag: null, ...extra });
      s.parent.set(id, parent);
    };
    add("srv", ROOT, "server", "srv", { serverId: "S1" });
    add("db", "srv", "database", "app");
    add("sch", "db", "schema", "public");
    add("tbl", "sch", "table", "orders");
    const tree = (): TreeState => s;
    const base: DdlTarget = { serverId: "S1", databaseOid: 5, object: obj, create: null, focus: null, refreshNodeId: null };
    const cfg: ServerConfig = {
      id: "S1", name: "srv", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
      ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: null, label: null, readOnlyLock: true,
    };
    const { ctl, api } = setup({ locked: true });
    const store = createTabsStore();
    const notify = vi.fn();
    const ddlTargetOf = vi.fn(async (): Promise<DdlTarget> => base);
    const opener = createDataOpener({
      ctl, tabs: store, api: { ddlTargetOf }, tree, refreshServers: async () => [cfg], connect: async () => true, notify,
    });
    for (const t of [
      { ...base, databaseOid: null },
      { ...base, focus: { kind: "column" as const, id: 1 } },
      { ...base, object: { kind: "sequence" as const, oid: 9 } },
      { ...base, object: null, create: { kind: "table" as const, context: { schemaOid: 1, tableOid: null } } },
    ]) {
      ddlTargetOf.mockResolvedValueOnce(t as DdlTarget);
      await opener.open("tbl");
      expect(notify).toHaveBeenLastCalledWith(tdata.open.none, "info");
    }
    expect(store.state.data).toHaveLength(0);
    expect(api.dataLoad).not.toHaveBeenCalled();
    // Kilitli sunucu: sekme açılır, yükleme başlar; kapı kilitli.
    await opener.open("tbl");
    await flush();
    expect(store.state.data).toHaveLength(1);
    const tab = store.state.data[0];
    expect(tab.key).toBe("data:S1:5:42");
    expect(tab.where).toBe("srv/app.public");
    expect(tab.title).toContain("orders");
    const m = ctl.model(tab.id)!;
    expect(m.status).toBe("ready");
    expect(m.locked).toBe(true);
    expect(m.gate).toEqual({ kind: "blocked", reason: "locked" });
  });

  it("yükleme: dataLoad → qtOpenReadonly(autocommit) → dataSelect → qtRun; sürüm kolonu son; started/rows olayları satırları ekler; 50 000 sınırında more kapanır", async () => {
    const { api, ctl, open, m } = setup();
    const order: string[] = [];
    api.dataLoad.mockImplementation(async () => (order.push("load"), source()));
    api.qtOpenReadonly.mockImplementation(async () => (order.push("open"), "sess1"));
    api.dataSelect.mockImplementation(async () => (order.push("select"), selectOk));
    api.qtRun.mockImplementation(async (_s, _q, _sc, _l, _c, onEvent) => {
      order.push("run");
      onEvent(started());
      onEvent(rowsEv(rows(0, 600), true));
      onEvent(rowsEv(rows(600, 400), true));
      onEvent(done);
      return null;
    });
    await open();
    expect(order).toEqual(["load", "open", "select", "run"]);
    expect(api.qtOpenReadonly).toHaveBeenCalledWith("S1", 5, { mode: "autocommit", statementTimeoutMs: 30000 });
    expect(api.dataSelect).toHaveBeenCalledWith("S1", 5, obj, "fp1", { order: [], filter: null });
    expect(api.qtRun.mock.calls[0].slice(0, 5)).toEqual(["sess1", SQL, { type: "all" }, 1000, false]);
    expect(m().status).toBe("ready");
    expect(m().loaded!.rows).toHaveLength(1000);
    expect(m().loaded!.versionIndex).toBe(3);
    expect(m().loaded!.rows[999][3]).toBe(`${MARK.ver}999`);
    expect(m().more).toBe(true);

    // Daha fazla: 1000'lik değil büyük parçalarla sınıra hızlı ulaşılır; sınırda more kapanır.
    let next = 1000;
    api.qtMore.mockImplementation(async (session, stmt, onEvent) => {
      expect([session, stmt]).toEqual(["sess1", 0]);
      onEvent(rowsEv(rows(next, 30000), true));
      next += 30000;
      return null;
    });
    await ctl.loadMore("g1");
    expect(m().loaded!.rows).toHaveLength(31000);
    expect(m().more).toBe(true);
    await ctl.loadMore("g1");
    expect(m().loaded!.rows).toHaveLength(ROW_CAP);
    expect(ROW_CAP).toBe(50000);
    expect(m().more).toBe(false);
    expect(m().truncatedAtCap).toBe(true);
    await ctl.loadMore("g1");
    expect(api.qtMore).toHaveBeenCalledTimes(2);
  });

  it("yükleme: kolon sayısı/sürüm indeksi uyuşmazsa loadError; qt_more hatası imleç ipucu verir", async () => {
    const a = setup();
    a.api.qtRun.mockImplementation(async (_s, _q, _sc, _l, _c, onEvent) => {
      onEvent(started(3));
      onEvent(rowsEv(rows(0, 2), false));
      return null;
    });
    await a.open();
    expect(a.m().status).toBe("error");
    expect(a.m().loadError).toBe(tdata.load.columnsMismatch);
    expect(a.m().loaded).toBeNull();

    const b = setup();
    b.api.qtRun.mockImplementation(async (_s, _q, _sc, _l, _c, onEvent) => {
      onEvent(started());
      onEvent(rowsEv(rows(0, 1000), true));
      return null;
    });
    await b.open();
    b.api.qtMore.mockRejectedValue(ipcErr("query", "cursor gone"));
    await b.ctl.loadMore("g1");
    expect(b.m().loadError).toBe(tdata.more.expired);
    expect(b.m().more).toBe(false);
    expect(b.m().loaded!.rows).toHaveLength(1000);
  });

  it("select Invalid → selectErrors dolar, satırlar değişmez; setSelect kirli iken onaysız reddedilir", async () => {
    const { api, ctl, open, m } = setup();
    await open();
    const before = m().loaded!.rows;
    api.dataSelect.mockResolvedValueOnce({ result: "invalid", errors: [{ path: "filter", message: "bad filter" }, { path: "order[0].column", message: "bad col" }, { path: "x", message: "gen" }] });
    expect(await ctl.setSelect("g1", { order: [], filter: `${MARK.filter} >` })).toBe(true);
    expect(m().selectErrors).toEqual({ filter: ["bad filter"], order: ["bad col"], general: ["gen"] });
    expect(m().loaded!.rows).toBe(before);
    expect(m().status).toBe("ready");
    expect(api.qtRun).toHaveBeenCalledTimes(1);

    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    expect(ctl.isDirty("g1")).toBe(true);
    const selects = api.dataSelect.mock.calls.length;
    expect(await ctl.setSelect("g1", { order: [{ column: "name", desc: true }], filter: "" })).toBe(false);
    expect(api.dataSelect).toHaveBeenCalledTimes(selects);
    expect(await ctl.setSelect("g1", { order: [{ column: "name", desc: true }], filter: "" }, { discard: true })).toBe(true);
    expect(api.dataSelect).toHaveBeenLastCalledWith("S1", 5, obj, "fp1", { order: [{ column: "name", desc: true }], filter: null });
    expect(ctl.isDirty("g1")).toBe(false);
    expect(m().selectErrors).toEqual({ filter: [], order: [], general: [] });
  });

  it("düzenleme → debounce'lu önizleme tek istek (yarış: eski sonuç yok sayılır); no-op düzenleme önizleme istemez", async () => {
    const { api, ctl, open, fire, m, timers } = setup();
    await open();
    // no-op: aynı durum → zamanlayıcı yok.
    ctl.edit("g1", (_l, e) => e);
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T(`${MARK.cell}0`)));
    expect(timers).toHaveLength(0);
    expect(m().pending).toBe(false);

    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("a")));
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("b")));
    expect(m().pending).toBe(true);
    await fire();
    expect(api.dataPreview).toHaveBeenCalledTimes(1);
    expect(api.dataPreview.mock.calls[0][2].changes).toEqual([
      { op: "update", key: [`${MARK.key}0`], version: `${MARK.ver}0`, set: [{ column: "name", value: T("b") }] },
    ]);
    expect(api.dataPreview.mock.calls[0][2].fingerprint).toBe("fp1");
    expect(m().viewIsCurrent).toBe(true);

    // Yarış: ilk istek geç döner, ikincisi kazanır.
    const d1 = deferred<PlanView>();
    const d2 = deferred<PlanView>();
    api.dataPreview.mockReturnValueOnce(d1.promise).mockReturnValueOnce(d2.promise);
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("c")));
    await fire();
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("d")));
    await fire();
    d2.resolve(view({ hash: "h2" }));
    await flush();
    d1.resolve(view({ hash: "h-old" }));
    await flush();
    expect(m().view!.hash).toBe("h2");
    expect(m().viewIsCurrent).toBe(true);
    expect(m().pending).toBe(false);

    // Özgüne dönüş: değişiklik yok → istek yok, görünüm düşer.
    const calls = api.dataPreview.mock.calls.length;
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T(`${MARK.cell}0`)));
    await fire();
    expect(api.dataPreview).toHaveBeenCalledTimes(calls);
    expect(m().view).toBeNull();
    expect(ctl.isDirty("g1")).toBe(false);
  });

  it("hata yolları fieldIssues ile doğru hücreye/satıra eşlenir", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    api.dataPreview.mockResolvedValue(view({
      empty: true,
      errors: [
        { path: "changes[0].set[0]", message: "cell bad" },
        { path: "changes[1].version", message: "row bad" },
        { path: "object", message: "general bad" },
      ],
    }));
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("a")));
    ctl.edit("g1", (l, e) => setCell(l, e, 2, "note", T("b")));
    await fire();
    expect(m().issues!.cell.get("0:name")).toEqual(["cell bad"]);
    expect(m().issues!.row.get(1)).toEqual(["row bad"]);
    expect(m().issues!.general).toEqual(["general bad"]);
    expect(m().gate).toEqual({ kind: "blocked", reason: "empty" });
  });

  it("apply başarı: loaded tazelenir (UPDATE yerinde, INSERT sona, DELETE çıkar), edit sıfırlanır, aria-live notu", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("new")));
    ctl.edit("g1", (l, e) => toggleDelete(l, e, 1));
    ctl.edit("g1", (l, e) => addRow(l, e));
    ctl.edit("g1", (l, e) => newRowSetCell(l, e, 1, "name", T("ins")));
    api.dataPreview.mockResolvedValue(view({ risk: "destructive" }));
    await fire();
    expect(m().gate).toEqual({ kind: "needsConfirm", production: false, destructive: true });
    ctl.setConfirm("g1", true);
    api.dataApply.mockResolvedValue({
      outcome: "applied", statements: 3, elapsedMs: 2,
      results: [
        { index: 0, returned: ["K0", "new", null, "V0b"] },
        { index: 1, returned: null },
        { index: 2, returned: ["K9", "ins", null, "V9"] },
      ],
    });
    await ctl.apply("g1");
    expect(api.dataApply).toHaveBeenCalledTimes(1);
    const [sid, db, cs, opts] = api.dataApply.mock.calls[0];
    expect([sid, db]).toEqual(["S1", 5]);
    expect(cs.object).toEqual(obj);
    expect(cs.fingerprint).toBe("fp1");
    expect(cs.changes.map((c) => c.op)).toEqual(["update", "delete", "insert"]);
    expect(opts).toEqual({ applyId: "apply-1", lockTimeoutMs: 5000, expectedHash: "h1", confirmed: true });
    const r = m().loaded!.rows;
    expect(r).toHaveLength(5);
    expect(r[0]).toEqual(["K0", "new", null, "V0b"]);
    expect(r[1][0]).toBe(`${MARK.key}2`);
    expect(r[4]).toEqual(["K9", "ins", null, "V9"]);
    expect(ctl.isDirty("g1")).toBe(false);
    expect(m().view).toBeNull();
    expect(m().confirm.checked).toBe(false);
    expect(m().postApplyNote).toBe(tdata.status.applied(3));
    expect(m().applying).toBe(false);

    // Kullanılamayan returned → yeniden yükleme ipucu.
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("again")));
    api.dataPreview.mockResolvedValue(view());
    await fire();
    api.dataApply.mockResolvedValue({ outcome: "applied", statements: 1, elapsedMs: 1, results: [] });
    await ctl.apply("g1");
    expect(m().postApplyNote).toContain(tdata.status.applied(1));
    expect(m().postApplyNote).toContain(tdata.status.refreshHint);
  });

  it("apply conflict: conflictRow = origin[failedIndex], düzenleme korunur, INSERT için farklı metin, otomatik yeniden deneme YOK (dataApply tek çağrı)", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    ctl.edit("g1", (l, e) => setCell(l, e, 3, "name", T("x")));
    ctl.edit("g1", (l, e) => addRow(l, e));
    ctl.edit("g1", (l, e) => newRowSetCell(l, e, 1, "name", T("ins")));
    await fire();
    api.dataApply.mockResolvedValueOnce(failed({ conflict: true, failedIndex: 0, error: { code: "row_conflict", message: "changed", detail: null, hint: null } }));
    await ctl.apply("g1");
    await flush();
    expect(api.dataApply).toHaveBeenCalledTimes(1);
    expect(m().conflictRow).toBe(3);
    expect(m().outcome!.kind).toBe("conflict");
    expect(m().outcome!.text).toBe(tdata.conflict.updateDelete);
    expect(ctl.isDirty("g1")).toBe(true);
    expect(m().edit.cells.get(3)?.get("name")).toEqual(T("x"));

    await fire();
    api.dataApply.mockResolvedValueOnce(failed({ conflict: true, failedIndex: 1, error: { code: "row_conflict", message: "changed", detail: null, hint: null } }));
    await ctl.apply("g1");
    await flush();
    expect(api.dataApply).toHaveBeenCalledTimes(2);
    expect(m().conflictRow).toBeNull();
    expect(m().failedChange).toEqual({ kind: "insert", newId: 1 });
    expect(m().outcome!.text).toBe(tdata.conflict.insert);
  });

  it("apply outcomeUnknown: kapı kapanır, yeniden yükleme zorunlu, ikinci dataApply çağrılmaz", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await fire();
    api.dataApply.mockResolvedValueOnce(failed({ outcomeUnknown: true }));
    await ctl.apply("g1");
    await flush();
    expect(m().outcomeUnknown).toBe(true);
    expect(m().outcome!.kind).toBe("unknown");
    expect(m().gate).toEqual({ kind: "blocked", reason: "unknown" });
    await fire();
    await ctl.apply("g1");
    expect(api.dataApply).toHaveBeenCalledTimes(1);
    expect(await ctl.reload("g1")).toBe(false);
    expect(await ctl.reload("g1", { discard: true })).toBe(true);
    expect(m().outcomeUnknown).toBe(false);
    expect(ctl.isDirty("g1")).toBe(false);
  });

  it("apply Err: plan_changed → needsReload; locked → kilit; confirmation_required → onay sıfırlanır + önizleme; bağlantı kodu → disconnected", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    const ready = async () => {
      ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T(`v${Math.random()}`)));
      await fire();
    };
    await ready();
    api.dataApply.mockRejectedValueOnce(ipcErr("plan_changed"));
    await ctl.apply("g1");
    expect(m().needsReload).toBe(true);
    expect(m().gate).toEqual({ kind: "blocked", reason: "stale" });
    expect(await ctl.reload("g1", { discard: true })).toBe(true);
    expect(m().needsReload).toBe(false);

    await ready();
    api.dataApply.mockRejectedValueOnce(ipcErr("locked"));
    await ctl.apply("g1");
    expect(m().locked).toBe(true);
    expect(m().gate).toEqual({ kind: "blocked", reason: "locked" });
    await ctl.reload("g1", { discard: true });

    await ready();
    ctl.setConfirm("g1", true);
    const previews = api.dataPreview.mock.calls.length;
    api.dataApply.mockRejectedValueOnce(ipcErr("confirmation_required"));
    await ctl.apply("g1");
    await flush();
    expect(m().confirm.checked).toBe(false);
    expect(api.dataPreview.mock.calls.length).toBe(previews + 1);
    expect(m().outcome!.text).toBe(tdata.gate.confirmationRequired);

    api.dataApply.mockRejectedValueOnce(ipcErr("connection_lost"));
    await ctl.apply("g1");
    expect(m().status).toBe("disconnected");
    expect(ctl.isDirty("g1")).toBe(true);
  });

  it("üretimde Safe plan onay kutusu ister; DELETE onay ister; onaysız dataApply çağrılmaz", async () => {
    const p = setup({ production: true });
    await p.open();
    p.ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await p.fire();
    expect(p.m().production).toBe(true);
    expect(p.m().gate).toEqual({ kind: "needsConfirm", production: true, destructive: false });
    await p.ctl.apply("g1");
    expect(p.api.dataApply).not.toHaveBeenCalled();
    p.ctl.setConfirm("g1", true);
    await p.ctl.apply("g1");
    expect(p.api.dataApply).toHaveBeenCalledTimes(1);
    expect(p.api.dataApply.mock.calls[0][3].confirmed).toBe(true);

    const d = setup();
    await d.open();
    d.api.dataPreview.mockResolvedValue(view({ risk: "destructive" }));
    d.ctl.edit("g1", (l, e) => toggleDelete(l, e, 0));
    await d.fire();
    expect(d.m().gate).toEqual({ kind: "needsConfirm", production: false, destructive: true });
    await d.ctl.apply("g1");
    expect(d.api.dataApply).not.toHaveBeenCalled();

    // Kapı uygulama anında sunucu kaydından yeniden hesaplanır: kayıt kilitlendiyse istek yok.
    const k = setup();
    await k.open();
    k.ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await k.fire();
    expect(k.m().gate).toEqual({ kind: "ready" });
    k.setServer({ locked: true });
    await k.ctl.apply("g1");
    expect(k.api.dataApply).not.toHaveBeenCalled();
    k.ctl.serverChanged("S1");
    expect(k.m().gate).toEqual({ kind: "blocked", reason: "locked" });
  });

  it("apply: önizleme hatası sonrası görünüm yoksa dataApply yok, önce önizleme istenir", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    api.dataPreview.mockRejectedValueOnce(ipcErr("query", "preview down"));
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await fire();
    expect(m().previewError).toBe("preview down");
    expect(m().gate).toEqual({ kind: "blocked", reason: "pending" });
    await ctl.apply("g1");
    await flush();
    expect(api.dataApply).not.toHaveBeenCalled();
    expect(api.dataPreview).toHaveBeenCalledTimes(2);
    expect(m().gate).toEqual({ kind: "ready" });
  });

  it("close: qtClose çağrılır, uçuştaki apply için dataCancel; reload kirli iken discard olmadan false", async () => {
    const { api, ctl, open, fire, m } = setup();
    await open();
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await fire();
    const loads = api.dataLoad.mock.calls.length;
    expect(await ctl.reload("g1")).toBe(false);
    expect(ctl.reloadNow("g1")).toBe(false);
    expect(api.dataLoad).toHaveBeenCalledTimes(loads);

    const d = deferred<DataOutcome>();
    api.dataApply.mockReturnValueOnce(d.promise);
    const p = ctl.apply("g1");
    await flush();
    expect(m().applying).toBe(true);
    ctl.cancelApply("g1");
    expect(api.dataCancel).toHaveBeenLastCalledWith("apply-1");
    ctl.close("g1");
    expect(api.dataCancel).toHaveBeenCalledTimes(2);
    expect(api.qtClose).toHaveBeenCalledWith("sess1", "rollback");
    expect(ctl.model("g1")).toBeNull();
    d.resolve({ outcome: "applied", statements: 1, elapsedMs: 1, results: [] });
    await p;
    expect(ctl.model("g1")).toBeNull();
  });

  it("oturum kaybı: bir kez yeniden bağlan+aç+dene; connection_lost → disconnected; serverSessionsClosed düzenlemeyi korur", async () => {
    const a = setup();
    a.api.qtRun.mockRejectedValueOnce(ipcErr("session_reset"));
    await a.open();
    expect(a.api.connect).toHaveBeenCalledWith("S1");
    expect(a.api.qtOpenReadonly).toHaveBeenCalledTimes(2);
    expect(a.api.qtRun).toHaveBeenCalledTimes(2);
    expect(a.api.qtClose).toHaveBeenCalledWith("sess1", "rollback");
    expect(a.api.qtRun.mock.calls[1][0]).toBe("sess2");
    expect(a.m().status).toBe("ready");

    // İkinci kez de başarısız: üçüncü deneme yok.
    const b = setup();
    b.api.qtRun.mockRejectedValue(ipcErr("not_connected"));
    await b.open();
    expect(b.api.qtRun).toHaveBeenCalledTimes(2);
    expect(b.m().status).toBe("disconnected");

    const c = setup();
    c.api.qtRun.mockRejectedValueOnce(ipcErr("connection_lost"));
    await c.open();
    expect(c.m().status).toBe("disconnected");
    expect(c.api.qtRun).toHaveBeenCalledTimes(1);

    const d = setup();
    await d.open();
    d.ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T("x")));
    await d.fire();
    d.ctl.serverSessionsClosed("S1");
    expect(d.m().status).toBe("disconnected");
    expect(d.ctl.isDirty("g1")).toBe(true);
    expect(d.m().gate).toEqual({ kind: "blocked", reason: "disconnected" });
    // Yeniden bağlan: kaynak aynı → düzenlemeler korunur, önizleme yenilenir.
    const previews = d.api.dataPreview.mock.calls.length;
    expect(await d.ctl.reconnect("g1")).toBe(true);
    await flush();
    expect(d.m().status).toBe("ready");
    expect(d.ctl.isDirty("g1")).toBe(true);
    expect(d.api.dataPreview.mock.calls.length).toBe(previews + 1);
    expect(d.m().gate).toEqual({ kind: "ready" });
    // Kaynak değiştiyse yeniden yükleme zorunlu.
    d.ctl.serverSessionsClosed("S1", true);
    expect(d.m().status).toBe("gone");
    const e = setup();
    await e.open();
    e.ctl.edit("g1", (l, x) => setCell(l, x, 0, "name", T("x")));
    e.ctl.serverSessionsClosed("S1");
    e.api.dataLoad.mockResolvedValueOnce(source({ fingerprint: "fp2" }));
    await e.ctl.reconnect("g1");
    expect(e.m().needsReload).toBe(true);
    expect(e.ctl.isDirty("g1")).toBe(true);
  });

  it("değer sızıntısı yok: console.* çağrılmaz; toast/notify argümanlarında hücre/anahtar/sürüm/SQL yok", async () => {
    const { api, ctl, open, fire } = setup({ production: true });
    await open();
    await ctl.setSelect("g1", { order: [], filter: `${MARK.filter} = 1` });
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "name", T(`${MARK.cell}new`)));
    api.dataPreview.mockRejectedValueOnce(ipcErr("query"));
    await fire();
    ctl.edit("g1", (l, e) => setCell(l, e, 1, "name", T(`${MARK.cell}new2`)));
    await fire();
    ctl.setConfirm("g1", true);
    api.dataApply.mockResolvedValueOnce(failed({ conflict: true }));
    await ctl.apply("g1");
    await fire();
    api.qtMore.mockRejectedValueOnce(ipcErr("query"));
    await ctl.loadMore("g1");
    ctl.close("g1");

    // Açıcı bildirimleri (tek toast yolu).
    const notify = vi.fn();
    const opener = createDataOpener({
      ctl, tabs: createTabsStore(), api: { ddlTargetOf: vi.fn(async () => { throw ipcErr("not_found", "node missing"); }) },
      tree: () => {
        const s = emptyTree();
        s.nodes.set("n", { id: "n", kind: "table", label: "t", detail: null, hasChildren: false, serverId: null, flags: { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false }, color: null, tag: null });
        return s;
      },
      refreshServers: async () => [], connect: async () => true, notify,
    });
    await opener.open("n");
    expect(notify).toHaveBeenCalledTimes(1);
    const args = JSON.stringify(notify.mock.calls);
    for (const v of Object.values(MARK)) expect(args).not.toContain(v);
    for (const s of spies) expect(s).not.toHaveBeenCalled();
  });
});
