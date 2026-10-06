import { describe, expect, it } from "vitest";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { Setting } from "../../ipc/bindings/Setting";
import {
  ariaSort, durationTone, filterActivity, filterSettings, firstLines, navIndex, nextSort, oneLine, sortActivity, stateTone,
  waitLabel, formatMs, formatRatio, formatSettingValue, hitRatio, preparedAgeBadge, type ActivityFilter,
} from "./table-model";

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

describe("görünüm yardımcıları", () => {
  it("stateTone", () => {
    expect(stateTone("active")).toBe("active");
    expect(stateTone("idle")).toBe("idle");
    expect(stateTone("idle in transaction")).toBe("warn");
    expect(stateTone("idle in transaction (aborted)")).toBe("danger");
    expect(stateTone(null)).toBe("muted");
    expect(stateTone("fastpath function call")).toBe("muted");
  });
  it("durationTone sınırları", () => {
    expect(durationTone(null)).toBe("none");
    expect(durationTone(-1)).toBe("none");
    expect(durationTone(29.9)).toBe("normal");
    expect(durationTone(30)).toBe("warn");
    expect(durationTone(299)).toBe("warn");
    expect(durationTone(300)).toBe("danger");
  });
  it("oneLine ve firstLines", () => {
    expect(oneLine("select  1\n  from t", 100)).toBe("select 1 from t");
    expect(oneLine(null)).toBe("");
    expect(oneLine("abcdef", 3)).toBe("abc…");
    expect(firstLines("a\n\nb\r\nc\nd", 3)).toEqual({ text: "a\nb\nc", truncated: true });
    expect(firstLines("a\nb", 3)).toEqual({ text: "a\nb", truncated: false });
    expect(firstLines(null, 3)).toEqual({ text: "", truncated: false });
  });
  it("navIndex: sınırda durur, seçim yoksa uçlardan başlar, boş listede -1", () => {
    expect(navIndex(-1, "ArrowDown", 5)).toBe(0);
    expect(navIndex(-1, "ArrowUp", 5)).toBe(4);
    expect(navIndex(2, "ArrowDown", 5)).toBe(3);
    expect(navIndex(4, "ArrowDown", 5)).toBe(4);
    expect(navIndex(0, "ArrowUp", 5)).toBe(0);
    expect(navIndex(2, "Home", 5)).toBe(0);
    expect(navIndex(2, "End", 5)).toBe(4);
    expect(navIndex(1, "PageDown", 50, 10)).toBe(11);
    expect(navIndex(3, "PageUp", 50, 10)).toBe(0);
    expect(navIndex(9, "ArrowDown", 5)).toBe(0);
    expect(navIndex(0, "End", 0)).toBe(-1);
  });
  it("nextSort / ariaSort", () => {
    expect(nextSort(null, "pid")).toEqual({ key: "pid", dir: "asc" });
    expect(nextSort({ key: "pid", dir: "asc" }, "pid")).toEqual({ key: "pid", dir: "desc" });
    expect(nextSort({ key: "pid", dir: "desc" }, "user")).toEqual({ key: "user", dir: "asc" });
    expect(ariaSort({ key: "pid", dir: "desc" }, "pid")).toBe("descending");
    expect(ariaSort({ key: "pid", dir: "asc" }, "pid")).toBe("ascending");
    expect(ariaSort({ key: "pid", dir: "asc" }, "user")).toBe("none");
    expect(ariaSort(null, "pid")).toBe("none");
  });
  it("waitLabel", () => {
    expect(waitLabel({ waitEventType: "Lock", waitEvent: "relation" })).toBe("Lock:relation");
    expect(waitLabel({ waitEventType: null, waitEvent: "x" })).toBe("x");
    expect(waitLabel({ waitEventType: null, waitEvent: null })).toBe("");
  });
});

describe("preparedAgeBadge", () => {
  it("1 saat ve üstü old, altı null", () => {
    expect(preparedAgeBadge(3599)).toBeNull();
    expect(preparedAgeBadge(3600)).toBe("old");
    expect(preparedAgeBadge(86400)).toBe("old");
    expect(preparedAgeBadge(0)).toBeNull();
    expect(preparedAgeBadge(Number.NaN)).toBeNull();
  });
});

describe("hitRatio / formatRatio", () => {
  it("hit / (hit + read); erişim yoksa ya da geçersizse null", () => {
    expect(hitRatio(90, 10)).toBe(0.9);
    expect(hitRatio(0, 5)).toBe(0);
    expect(hitRatio(5, 0)).toBe(1);
    expect(hitRatio(0, 0)).toBeNull();
    expect(hitRatio(-1, 5)).toBeNull();
    expect(hitRatio(Number.NaN, 5)).toBeNull();
  });
  it("yüzde biçimi", () => {
    expect(formatRatio(0.9731)).toBe("%97,3");
    expect(formatRatio(1)).toBe("%100,0");
    expect(formatRatio(null)).toBe("—");
  });
});

describe("formatMs", () => {
  it("küçük değerlerde ondalık, 1 sn üstünde süre metni", () => {
    expect(formatMs(0.42)).toBe("0,42 ms");
    expect(formatMs(12.34)).toBe("12,3 ms");
    expect(formatMs(850.4)).toBe("850 ms");
    expect(formatMs(0)).toBe("0 ms");
    expect(formatMs(12_000)).toBe("12 sn");
    expect(formatMs(185_000)).toBe("3 dk 05 sn");
    expect(formatMs(-1)).toBe("—");
    expect(formatMs(Number.NaN)).toBe("—");
  });
});

describe("formatSettingValue", () => {
  it("değer + birim; null → —", () => {
    expect(formatSettingValue({ value: "128", unit: "MB" })).toBe("128 MB");
    expect(formatSettingValue({ value: "on", unit: null })).toBe("on");
    expect(formatSettingValue({ value: "", unit: null })).toBe("");
    expect(formatSettingValue({ value: null, unit: "MB" })).toBe("—");
  });
});
