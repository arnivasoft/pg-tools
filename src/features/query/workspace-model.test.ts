import { describe, expect, it, vi } from "vitest";
import type { DraftTab } from "../../ipc/bindings/DraftTab";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import { initialRun, type RunState } from "./run-model";
import {
  applyEnvChoice, bulkPolicy, canCommitAny, canOpenQueryTool, choicePolicy, closePlan, createDebouncer, defaultOptions,
  envLockDefault, formatElapsed, initialSql, needsEnvPrompt, newQueryNode, nextTxSince, openContext, pendingTransactions,
  restoreDraft, routeError, sameScope, saveClosesSessions, swapEditorState, toDraft, serverLineColor, type SavedView, type Timers,
} from "./workspace-model";

const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };

function tree(): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: NodeKind, label: string, extra: Partial<TreeNode> = {}) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: true, serverId: null, flags, color: null, tag: null, ...extra });
    s.parent.set(id, parent);
    s.children.set(parent, [...(s.children.get(parent) ?? []), id]);
  };
  add("g", ROOT, "serverGroup", "Grup");
  add("srv", "g", "server", "prod", { serverId: "S1" });
  add("dbs", "srv", "databaseCollection", "Databases");
  add("db", "dbs", "database", "app db");
  add("schs", "db", "schemaCollection", "Schemas");
  add("sch", "schs", "schema", "Sales");
  add("tbls", "sch", "tableCollection", "Tables");
  add("tbl", "tbls", "table", "Order Items");
  add("cols", "tbl", "columnCollection", "Columns");
  add("col", "cols", "column", "id");
  add("parts", "tbl", "partitionCollection", "Partitions");
  add("part", "parts", "table", "order_2024");
  add("views", "sch", "viewCollection", "Views");
  add("view", "views", "view", "v");
  add("roles", "srv", "roleCollection", "Roles");
  add("role", "roles", "role", "admin");
  return s;
}

const server = (over: Partial<ServerConfig> = {}): ServerConfig => ({
  id: "S1", name: "prod", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
  ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: null, label: null, readOnlyLock: false, ...over,
});

const run = (over: Partial<RunState>): RunState => ({ ...initialRun(), status: "idle", ...over });

describe("açma bağlamı", () => {
  it("menü yalnızca sunucu/veritabanı/şema/tablo/görünüm için", () => {
    for (const k of ["server", "database", "schema", "table", "view", "materializedView"] as NodeKind[]) expect(canOpenQueryTool(k)).toBe(true);
    for (const k of ["serverGroup", "column", "tableCollection", "role", "function"] as NodeKind[]) expect(canOpenQueryTool(k)).toBe(false);
  });

  it("tablo: sunucu, veritabanı adı ve [şema, ad]; kimlik ayrıştırılmaz", () => {
    expect(openContext(tree(), "tbl")).toEqual({ nodeId: "tbl", serverId: "S1", database: "app db", relation: ["Sales", "Order Items"] });
    expect(openContext(tree(), "view")?.relation).toEqual(["Sales", "v"]);
  });

  it("sunucu: bakım veritabanı (database null); şema: ilişki yok", () => {
    expect(openContext(tree(), "srv")).toEqual({ nodeId: "srv", serverId: "S1", database: null, relation: null });
    expect(openContext(tree(), "sch")).toEqual({ nodeId: "sch", serverId: "S1", database: "app db", relation: null });
  });

  it("bölüm tablosu ilişki taslağı almaz; desteklenmeyen düğüm null", () => {
    expect(openContext(tree(), "part")?.relation).toBeNull();
    expect(openContext(tree(), "col")).toBeNull();
    expect(openContext(tree(), "nope")).toBeNull();
  });

  it("yeni sorgu: veritabanı atası, yoksa sunucu", () => {
    expect(newQueryNode(tree(), "col")).toBe("db");
    expect(newQueryNode(tree(), "role")).toBe("srv");
    expect(newQueryNode(tree(), "g")).toBeNull();
    expect(newQueryNode(tree(), null)).toBeNull();
  });

  it("taslak: tırnaklı nitelikli ad ile SELECT … LIMIT 100", () => {
    expect(initialSql(['"Sales"', '"Order Items"'])).toBe('SELECT * FROM "Sales"."Order Items" LIMIT 100;\n');
    expect(initialSql(null)).toBe("");
  });

  it("varsayılan mod: üretimde Manuel", () => {
    expect(defaultOptions("Üretim")).toEqual({ mode: "manual", statementTimeoutMs: 0 });
    expect(defaultOptions(" PROD ").mode).toBe("manual");
    expect(defaultOptions("Test").mode).toBe("autocommit");
    expect(defaultOptions(null).mode).toBe("autocommit");
  });
});

describe("ortam istemi", () => {
  it("yalnızca etiketsiz ve atlanmamış sunucuda", () => {
    expect(needsEnvPrompt(server(), new Set())).toBe(true);
    expect(needsEnvPrompt(server({ label: "  " }), new Set())).toBe(true);
    expect(needsEnvPrompt(server({ label: "Test" }), new Set())).toBe(false);
    expect(needsEnvPrompt(server(), new Set(["S1"]))).toBe(false);
  });

  it("üretim seçimi kilidi önerir ve kaydeder; diğerleri kilide dokunmaz", () => {
    expect(envLockDefault(server(), "production")).toBe(true);
    expect(envLockDefault(server(), "test")).toBe(false);
    expect(applyEnvChoice(server(), "production", true)).toMatchObject({ label: "Üretim", readOnlyLock: true });
    expect(applyEnvChoice(server(), "production", false)).toMatchObject({ label: "Üretim", readOnlyLock: false });
    expect(applyEnvChoice(server({ readOnlyLock: true }), "development", false)).toMatchObject({ label: "Geliştirme", readOnlyLock: true });
  });

  it("kayıt oturumları kapatır mı (arka uçla aynı kural)", () => {
    const a = server();
    expect(saveClosesSessions(null, a)).toBe(false);
    expect(saveClosesSessions(a, { ...a, name: "x", color: "red", label: "Test" })).toBe(false);
    expect(saveClosesSessions(a, { ...a, label: "Üretim" })).toBe(true);
    expect(saveClosesSessions(a, { ...a, readOnlyLock: true })).toBe(true);
    expect(saveClosesSessions(a, { ...a, port: 5433 })).toBe(true);
    expect(saveClosesSessions(a, { ...a, ssl: { ...a.ssl, mode: "require" } })).toBe(true);
  });
});

describe("kapanış diyaloğu", () => {
  it("oturum yoksa ya da kayıpsa doğrudan kapanır", () => {
    expect(closePlan(null, run({ tx: "inTransaction" }))).toEqual({ kind: "close", policy: null, busy: false });
    expect(closePlan("s", run({ status: "lost" }))).toEqual({ kind: "close", policy: null, busy: false });
  });

  it("açık transaction: Kaydet/At/Vazgeç; hatalıda Kaydet yok; meşgulde Kaydet yok", () => {
    expect(closePlan("s", run({ tx: "inTransaction" }))).toEqual({ kind: "confirm", choices: ["commit", "rollback", "cancel"], busy: false });
    expect(closePlan("s", run({ tx: "failed" }))).toEqual({ kind: "confirm", choices: ["rollback", "cancel"], busy: false });
    expect(closePlan("s", run({ status: "running", tx: "inTransaction" }))).toEqual({ kind: "confirm", choices: ["rollback", "cancel"], busy: true });
  });

  it("transaction yok: meşgulse rollback (iptal), boştaysa refuse", () => {
    expect(closePlan("s", run({ status: "running" }))).toEqual({ kind: "close", policy: "rollback", busy: true });
    expect(closePlan("s", run({}))).toEqual({ kind: "close", policy: "refuse", busy: false });
    expect(choicePolicy("commit")).toBe("commit");
    expect(choicePolicy("rollback")).toBe("rollback");
  });
});

describe("uygulama kapanışı / sunucu değişimi sekme listesi", () => {
  const tabs = [
    { id: "a", title: "A", serverId: "S1", session: "x", run: run({ tx: "inTransaction" }) },
    { id: "b", title: "B", serverId: "S2", session: "y", run: run({ tx: "failed" }) },
    { id: "c", title: "C", serverId: "S1", session: "z", run: run({}) },
    { id: "d", title: "D", serverId: "S1", session: null, run: run({ tx: "inTransaction" }) },
    { id: "e", title: "E", serverId: "S1", session: "w", run: run({ status: "lost", tx: "idle" }) },
    { id: "f", title: "F", serverId: "S2", session: "v", run: run({ status: "running", tx: "inTransaction" }) },
  ];

  it("yalnızca oturumlu açık/hatalı transaction'lar; sunucuya göre süzülür", () => {
    expect(pendingTransactions(tabs).map((p) => p.id)).toEqual(["a", "b", "f"]);
    expect(pendingTransactions(tabs, "S1")).toEqual([{ id: "a", title: "A", tx: "inTransaction", busy: false }]);
    expect(pendingTransactions(tabs, "S3")).toEqual([]);
  });

  it("toplu politika: hatalı ve meşgul sekmeler her zaman geri alınır", () => {
    const list = pendingTransactions(tabs);
    expect(list.map((p) => bulkPolicy(p, "commit"))).toEqual(["commit", "rollback", "rollback"]);
    expect(list.map((p) => bulkPolicy(p, "rollback"))).toEqual(["rollback", "rollback", "rollback"]);
    expect(canCommitAny(list)).toBe(true);
    expect(canCommitAny(list.filter((p) => p.id !== "a"))).toBe(false);
  });
});

describe("hata yönlendirme", () => {
  it("koda göre", () => {
    expect(routeError({ code: "busy" })).toBe("toast");
    expect(routeError({ code: "session_reset" })).toBe("reset");
    for (const code of ["not_connected", "unknown_server", "not_found"]) expect(routeError({ code })).toBe("lost");
    for (const code of ["query", "tx_failed", "confirmation_required", "invalid"]) expect(routeError({ code })).toBe("message");
  });

  it("kapsam eşitliği", () => {
    expect(sameScope({ type: "all" }, { type: "all" })).toBe(true);
    expect(sameScope({ type: "atCursor", offset: 3 }, { type: "atCursor", offset: 4 })).toBe(false);
    expect(sameScope(null, { type: "all" })).toBe(false);
  });
});

describe("taslaklar", () => {
  it("sekme → taslak ve geri (bağlanmadan; sunucu yoksa salt okunur)", () => {
    const d = toDraft({ id: "11111111-1111-1111-1111-111111111111", title: "prod/app #1", serverId: "S1", databaseOid: 5, database: "app", options: { mode: "manual", statementTimeoutMs: 30000 } });
    expect(d).toEqual({ id: "11111111-1111-1111-1111-111111111111", title: "prod/app #1", serverId: "S1", databaseOid: 5, database: "app", options: { mode: "manual", statementTimeoutMs: 30000 } });
    const r = restoreDraft(d, "select 1", [server()]);
    expect(r).toMatchObject({ serverGone: false, serverName: "prod", database: "app", text: "select 1", options: { mode: "manual" } });
    expect(restoreDraft(d, "", [])).toMatchObject({ serverGone: true, database: "app" });
    expect(restoreDraft({ ...d, serverId: null }, "", [server()]).serverGone).toBe(true);
  });

  it("eski taslakta ad yoksa: bakım veritabanı ya da OID", () => {
    const d: DraftTab = { id: "x", title: "t", serverId: "S1", databaseOid: null, database: null, options: { mode: "autocommit", statementTimeoutMs: 0 } };
    expect(restoreDraft(d, "", [server()]).database).toBe("postgres");
    expect(restoreDraft({ ...d, databaseOid: 9 }, "", [server()]).database).toBe("OID 9");
  });

  it("debounce: 5 sn sessizlikte bir kez; flush hemen; cancel unutur", async () => {
    let now = 0;
    const queue: { at: number; fn: () => void; id: number }[] = [];
    let n = 0;
    const timers: Timers = {
      set: (fn, ms) => {
        queue.push({ at: now + ms, fn, id: ++n });
        return n;
      },
      clear: (h) => {
        const i = queue.findIndex((q) => q.id === h);
        if (i >= 0) queue.splice(i, 1);
      },
    };
    const advance = (ms: number) => {
      now += ms;
      for (const q of queue.filter((q) => q.at <= now)) {
        queue.splice(queue.indexOf(q), 1);
        q.fn();
      }
    };
    const save = vi.fn();
    const d = createDebouncer(5000, save, timers);
    d.touch("a");
    advance(4000);
    d.touch("a");
    advance(4000);
    expect(save).not.toHaveBeenCalled();
    advance(1000);
    expect(save).toHaveBeenCalledTimes(1);
    d.touch("b");
    d.touch("c");
    await d.flush("b");
    expect(save).toHaveBeenLastCalledWith("b");
    d.cancel("c");
    advance(10000);
    expect(save).toHaveBeenCalledTimes(2);
    d.touch("a");
    d.touch("b");
    await d.flushAll();
    expect(save).toHaveBeenCalledTimes(4);
    expect(d.isPending("a")).toBe(false);
    await d.flush("zzz"); // bekleyen yoksa çalışmaz
    expect(save).toHaveBeenCalledTimes(4);
  });
});

describe("transaction süresi", () => {
  it("açılış anı tutulur, boşa dönünce sıfırlanır", () => {
    expect(nextTxSince("idle", "inTransaction", null, 100)).toBe(100);
    expect(nextTxSince("inTransaction", "failed", 100, 200)).toBe(100);
    expect(nextTxSince("inTransaction", "idle", 100, 200)).toBeNull();
    expect(nextTxSince("inTransaction", "inTransaction", null, 300)).toBe(300);
    expect(formatElapsed(65_000)).toBe("01:05");
    expect(formatElapsed(3_726_000)).toBe("1:02:06");
    expect(formatElapsed(-5)).toBe("00:00");
  });
});

describe("sekme ↔ editör durumu", () => {
  it("geçişte eski kaydedilir, yeni yoksa oluşturulur, varsa geri gelir", () => {
    const book = new Map<string, SavedView<string>>();
    const create = vi.fn(() => "fresh");
    const first = swapEditorState(book, null, null, "a", create);
    expect(first).toEqual({ state: "fresh", scrollTop: 0, scrollLeft: 0 });
    const toB = swapEditorState(book, "a", { state: "A-edited", scrollTop: 40, scrollLeft: 2 }, "b", () => "B");
    expect(toB.state).toBe("B");
    const backToA = swapEditorState(book, "b", { state: "B2", scrollTop: 7, scrollLeft: 0 }, "a", create);
    expect(backToA).toEqual({ state: "A-edited", scrollTop: 40, scrollLeft: 2 });
    expect(book.get("b")?.state).toBe("B2");
    expect(create).toHaveBeenCalledTimes(1);
  });
});

describe("sunucu çizgisi rengi", () => {
  it("renk > üretim tehlike rengi > yok", () => {
    expect(serverLineColor({ color: "teal", label: "Üretim" })).toBe("var(--srv-teal)");
    expect(serverLineColor({ color: null, label: "Üretim" })).toBe("var(--danger)");
    expect(serverLineColor({ color: null, label: "Test" })).toBeNull();
    expect(serverLineColor(null)).toBeNull();
  });
});
