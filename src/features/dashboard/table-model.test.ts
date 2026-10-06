import { describe, expect, it } from "vitest";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { Setting } from "../../ipc/bindings/Setting";
import { filterActivity, filterSettings, sortActivity, type ActivityFilter } from "./table-model";

const row = (pid: number, p: Partial<ActivityRow> = {}): ActivityRow => ({
  pid, leaderPid: null, database: "db", user: "alice", application: "psql", clientAddr: null, backendType: "client backend",
  state: "active", waitEventType: null, waitEvent: null, backendAgeSecs: 1, xactAgeSecs: null, queryAgeSecs: 1, stateAgeSecs: 1,
  query: "select 1", isSelf: false, ...p,
});
const none: ActivityFilter = { search: "", states: new Set(), onlyActive: false, idleInTxOverSecs: null, database: null };
const f = (p: Partial<ActivityFilter>): ActivityFilter => ({ ...none, ...p });
const ids = (r: ActivityRow[]) => r.map((x) => x.pid);

describe("filterActivity", () => {
  const rows = [
    row(1, { query: "SELECT * FROM Orders", user: "alice" }),
    row(2, { query: "update t set a=1", user: "Bob", state: "idle" }),
    row(3, { query: null, user: null, state: "idle in transaction", stateAgeSecs: 120, database: "other" }),
    row(4, { state: "idle in transaction (aborted)", stateAgeSecs: 10 }),
    row(5, { state: null, query: null }),
  ];
  it("süzgeç yoksa hepsi", () => {
    expect(ids(filterActivity(rows, none))).toEqual([1, 2, 3, 4, 5]);
  });
  it("arama sorgu metni ve kullanıcıda, büyük/küçük harf duyarsız; null alanlar patlatmaz", () => {
    expect(ids(filterActivity(rows, f({ search: "orders" })))).toEqual([1]);
    expect(ids(filterActivity(rows, f({ search: "BOB" })))).toEqual([2]);
    expect(ids(filterActivity(rows, f({ search: "  alice " })))).toEqual([1, 4, 5]);
  });
  it("arama pid'de de eşleşir", () => {
    expect(ids(filterActivity(rows, f({ search: "3" })))).toEqual([3]);
  });
  it("onlyActive yalnızca state=active", () => {
    expect(ids(filterActivity(rows, f({ onlyActive: true })))).toEqual([1]);
  });
  it("states kümesi; boş küme süzmez; null state hiçbirine uymaz", () => {
    expect(ids(filterActivity(rows, f({ states: new Set(["idle"]) })))).toEqual([2]);
    expect(ids(filterActivity(rows, f({ states: new Set(["idle", "active"]) })))).toEqual([1, 2]);
  });
  it("idleInTxOverSecs: yalnızca açık transaction'lı ve stateAge eşiği geçen", () => {
    expect(ids(filterActivity(rows, f({ idleInTxOverSecs: 60 })))).toEqual([3]);
    expect(ids(filterActivity(rows, f({ idleInTxOverSecs: 10 })))).toEqual([3, 4]);
    expect(ids(filterActivity(rows, f({ idleInTxOverSecs: 0 })))).toEqual([3, 4]);
  });
  it("idleInTxOverSecs ile stateAge null olan satır elenir", () => {
    expect(ids(filterActivity([row(9, { state: "idle in transaction", stateAgeSecs: null })], f({ idleInTxOverSecs: 0 })))).toEqual([]);
  });
  it("database tam eşleşme; süzgeçler AND", () => {
    expect(ids(filterActivity(rows, f({ database: "other" })))).toEqual([3]);
    expect(ids(filterActivity(rows, f({ database: "db", onlyActive: true })))).toEqual([1]);
  });
  it("girdiyi değiştirmez", () => {
    const copy = [...rows];
    filterActivity(rows, f({ onlyActive: true }));
    expect(rows).toEqual(copy);
  });
});

describe("sortActivity", () => {
  it("null'lar yönden bağımsız sonda; eşitlikte pid ile kararlı", () => {
    const rows = [
      row(5, { xactAgeSecs: null }), row(3, { xactAgeSecs: 10 }), row(2, { xactAgeSecs: 10 }), row(4, { xactAgeSecs: 99 }),
      row(1, { xactAgeSecs: null }),
    ];
    expect(ids(sortActivity(rows, "xactAge", "asc"))).toEqual([2, 3, 4, 1, 5]);
    expect(ids(sortActivity(rows, "xactAge", "desc"))).toEqual([4, 2, 3, 1, 5]);
  });
  it("pid sayısal sıralanır", () => {
    const rows = [row(10), row(9), row(100)];
    expect(ids(sortActivity(rows, "pid", "asc"))).toEqual([9, 10, 100]);
    expect(ids(sortActivity(rows, "pid", "desc"))).toEqual([100, 10, 9]);
  });
  it("metin sütunları: büyük/küçük harf duyarsız, null sonda", () => {
    const rows = [row(1, { user: "bob" }), row(2, { user: null }), row(3, { user: "Alice" })];
    expect(ids(sortActivity(rows, "user", "asc"))).toEqual([3, 1, 2]);
    expect(ids(sortActivity(rows, "user", "desc"))).toEqual([1, 3, 2]);
  });
  it("client = clientAddr; wait = tür:olay, ikisi de yoksa null", () => {
    const rows = [row(1, { clientAddr: "10.0.0.2" }), row(2, { clientAddr: null }), row(3, { clientAddr: "10.0.0.1" })];
    expect(ids(sortActivity(rows, "client", "asc"))).toEqual([3, 1, 2]);
    const w = [
      row(1, { waitEventType: "Lock", waitEvent: "relation" }), row(2), row(3, { waitEventType: "IO", waitEvent: "DataFileRead" }),
    ];
    expect(ids(sortActivity(w, "wait", "asc"))).toEqual([3, 1, 2]);
  });
  it("database, application, state, queryAge, stateAge anahtarları", () => {
    expect(ids(sortActivity([row(1, { database: "b" }), row(2, { database: "a" })], "database", "asc"))).toEqual([2, 1]);
    expect(ids(sortActivity([row(1, { application: "b" }), row(2, { application: "a" })], "application", "asc"))).toEqual([2, 1]);
    expect(ids(sortActivity([row(1, { state: "idle" }), row(2, { state: "active" })], "state", "asc"))).toEqual([2, 1]);
    expect(ids(sortActivity([row(1, { queryAgeSecs: 5 }), row(2, { queryAgeSecs: 1 })], "queryAge", "asc"))).toEqual([2, 1]);
    expect(ids(sortActivity([row(1, { stateAgeSecs: 9 }), row(2, { stateAgeSecs: 1 })], "stateAge", "asc"))).toEqual([2, 1]);
  });
  it("yeni dizi döner; girdi değişmez; boş girdi", () => {
    const rows = [row(2), row(1)];
    const out = sortActivity(rows, "pid", "asc");
    expect(out).not.toBe(rows);
    expect(ids(rows)).toEqual([2, 1]);
    expect(sortActivity([], "pid", "asc")).toEqual([]);
  });
});

const set = (name: string, p: Partial<Setting> = {}): Setting => ({
  name, value: "1", unit: null, category: "Memory", description: "", source: "default", context: "user", bootValue: null,
  pendingRestart: false, ...p,
});

describe("filterSettings", () => {
  const rows = [
    set("work_mem", { source: "configuration file", description: "Sets the maximum memory for query workspaces" }),
    set("shared_buffers", { source: "default", pendingRestart: true, category: "Resource Usage / Memory" }),
    set("port", { source: "command line", category: "Connections" }),
    set("max_connections", { source: "default" }),
  ];
  const q = { search: "", modifiedOnly: false, pendingRestartOnly: false };
  const names = (r: Setting[]) => r.map((x) => x.name);
  it("süzgeç yoksa hepsi", () => {
    expect(filterSettings(rows, q)).toHaveLength(4);
  });
  it("modifiedOnly: source != 'default'", () => {
    expect(names(filterSettings(rows, { ...q, modifiedOnly: true }))).toEqual(["work_mem", "port"]);
  });
  it("pendingRestartOnly", () => {
    expect(names(filterSettings(rows, { ...q, pendingRestartOnly: true }))).toEqual(["shared_buffers"]);
  });
  it("arama ad, kategori ve açıklamada; duyarsız", () => {
    expect(names(filterSettings(rows, { ...q, search: "WORKSPACES" }))).toEqual(["work_mem"]);
    expect(names(filterSettings(rows, { ...q, search: "max_" }))).toEqual(["max_connections"]);
    expect(names(filterSettings(rows, { ...q, search: "resource usage" }))).toEqual(["shared_buffers"]);
  });
  it("süzgeçler AND", () => {
    expect(names(filterSettings(rows, { search: "mem", modifiedOnly: true, pendingRestartOnly: false }))).toEqual(["work_mem"]);
    expect(filterSettings(rows, { search: "", modifiedOnly: true, pendingRestartOnly: true })).toEqual([]);
  });
});
