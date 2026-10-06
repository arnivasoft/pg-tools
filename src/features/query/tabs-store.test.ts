import { describe, expect, it } from "vitest";
import {
  OBJECT_TAB_ID,
  activateTab,
  closeTab,
  createTabsStore,
  initialTabs,
  isDashTabId,
  isQueryTabId,
  moveTab,
  openDashboard,
  openTab,
  renameTab,
  type TabsState,
} from "./tabs-store";

const open3 = () => {
  let s = initialTabs();
  const ids: string[] = [];
  for (const db of ["a", "a", "b"]) {
    const r = openTab(s, { serverId: "s1", serverName: "prod", database: db });
    s = r.state;
    ids.push(r.id);
  }
  return { s, ids };
};

describe("tabs-store (saf)", () => {
  it("başlangıçta Nesne sekmesi etkin", () => {
    const s = initialTabs();
    expect(s.activeId).toBe(OBJECT_TAB_ID);
    expect(s.tabs).toEqual([]);
  });

  it("varsayılan başlık <sunucu>/<db> #n ve yeni sekme etkin", () => {
    const { s, ids } = open3();
    expect(s.tabs.map((t) => t.title)).toEqual(["prod/a #1", "prod/a #2", "prod/b #1"]);
    expect(new Set(ids).size).toBe(3);
    expect(s.activeId).toBe(ids[2]);
  });

  it("verilen başlık ve kimlik; çakışan kimlik yenilenir", () => {
    let s: TabsState = openTab(initialTabs(), { serverId: "s", serverName: "x", database: "d", title: " T ", id: "q1" }).state;
    expect(s.tabs[0]).toMatchObject({ id: "q1", title: "T" });
    const r = openTab(s, { serverId: "s", serverName: "x", database: "d", id: "q1" });
    s = r.state;
    expect(r.id).not.toBe("q1");
    expect(openTab(s, { serverId: "s", serverName: "x", database: "d", id: OBJECT_TAB_ID }).id).not.toBe(OBJECT_TAB_ID);
  });

  it("Nesne sekmesi kapatılamaz", () => {
    const { s } = open3();
    expect(closeTab(s, OBJECT_TAB_ID)).toBe(s);
  });

  it("etkin sekme kapanınca sağdaki, yoksa soldaki, yoksa Nesne", () => {
    let { s, ids } = open3();
    s = activateTab(s, ids[1]);
    s = closeTab(s, ids[1]);
    expect(s.activeId).toBe(ids[2]);
    s = closeTab(s, ids[2]);
    expect(s.activeId).toBe(ids[0]);
    s = closeTab(s, ids[0]);
    expect(s.activeId).toBe(OBJECT_TAB_ID);
  });

  it("etkin olmayan sekme kapanınca etkin değişmez", () => {
    const { s, ids } = open3();
    expect(closeTab(s, ids[0]).activeId).toBe(ids[2]);
  });

  it("yeniden adlandırma; boş yok sayılır", () => {
    const { s, ids } = open3();
    expect(renameTab(s, ids[0], "  rapor ").tabs[0].title).toBe("rapor");
    expect(renameTab(s, ids[0], "   ")).toBe(s);
  });

  it("etkinleştirme: bilinmeyen kimlik yok sayılır, Nesne olur", () => {
    const { s } = open3();
    expect(activateTab(s, "yok")).toBe(s);
    expect(activateTab(s, OBJECT_TAB_ID).activeId).toBe(OBJECT_TAB_ID);
  });

  it("sıralama", () => {
    const { s, ids } = open3();
    expect(moveTab(s, ids[2], 0).tabs.map((t) => t.id)).toEqual([ids[2], ids[0], ids[1]]);
    expect(moveTab(s, ids[0], 99).tabs.map((t) => t.id)).toEqual([ids[1], ids[2], ids[0]]);
    expect(moveTab(s, ids[0], 0)).toBe(s);
  });
});

describe("tabs-store dashboard sekmeleri", () => {
  const dash = (s: TabsState, serverId = "s1", databaseOid: number | null = null, database: string | null = null) =>
    openDashboard(s, { serverId, databaseOid, title: database ? `D prod/${database}` : "D prod" });

  it("açılan dashboard etkin olur; kimlik sorgu sekmeleriyle çakışmaz; başlık verilen", () => {
    const { s, ids } = open3();
    const a = dash(s);
    expect(a.created).toBe(true);
    expect(a.state.activeId).toBe(a.id);
    expect(ids).not.toContain(a.id);
    expect(a.id).not.toBe(OBJECT_TAB_ID);
    expect(a.state.dashboards[0].title).toBe("D prod");
    const b = dash(a.state, "s1", 42, "app");
    expect(b.id).not.toBe(a.id);
    expect(b.state.dashboards[1].title).toBe("D prod/app");
    // Sonra açılan sorgu sekmesi de dashboard kimliğiyle çakışmaz.
    const q = openTab(b.state, { serverId: "s1", serverName: "prod", database: "a", id: a.id });
    expect(q.id).not.toBe(a.id);
    expect(q.state.dashboards).toHaveLength(2);
  });

  it("aynı sunucu + veritabanı ikinci kez açılınca var olan etkinleşir", () => {
    let { s } = open3();
    const a = dash(s, "s1", 7, "app");
    s = activateTab(a.state, OBJECT_TAB_ID);
    const again = dash(s, "s1", 7, "app");
    expect(again.created).toBe(false);
    expect(again.id).toBe(a.id);
    expect(again.state.dashboards).toHaveLength(1);
    expect(again.state.activeId).toBe(a.id);
    // Farklı veritabanı ya da sunucu düzeyi ayrı sekmedir.
    expect(dash(again.state, "s1", null).created).toBe(true);
    expect(dash(again.state, "s2", 7, "app").created).toBe(true);
  });

  it("kapanınca görünüm sırasıyla (sorgular, sonra dashboard'lar) komşu etkinleşir", () => {
    const { s: s0, ids } = open3();
    const a = dash(s0, "s1");
    const b = dash(a.state, "s2");
    let s = b.state;
    // Son sorgu sekmesi kapanınca sağındaki ilk dashboard.
    s = closeTab(activateTab(s, ids[2]), ids[2]);
    expect(s.activeId).toBe(a.id);
    // Dashboard kapanınca sağdaki dashboard.
    s = closeTab(s, a.id);
    expect(s.activeId).toBe(b.id);
    // Son dashboard kapanınca soldaki sorgu sekmesi.
    s = closeTab(s, b.id);
    expect(s.activeId).toBe(ids[1]);
    expect(s.dashboards).toEqual([]);
    expect(s.tabs.map((x) => x.id)).toEqual([ids[0], ids[1]]);
  });

  it("yalnız dashboard varken kapanınca Nesne; etkin değilse etkin değişmez", () => {
    const a = dash(initialTabs());
    const b = dash(a.state, "s2");
    expect(closeTab(b.state, a.id).activeId).toBe(b.id);
    expect(closeTab(closeTab(b.state, b.id), a.id).activeId).toBe(OBJECT_TAB_ID);
  });

  it("yeniden adlandırma ve etkinleştirme dashboard kimliğini tanır", () => {
    const a = dash(initialTabs());
    expect(renameTab(a.state, a.id, " izleme ").dashboards[0].title).toBe("izleme");
    expect(renameTab(a.state, a.id, "  ")).toBe(a.state);
    const s = activateTab(a.state, OBJECT_TAB_ID);
    expect(activateTab(s, a.id).activeId).toBe(a.id);
  });

  it("isQueryTabId / isDashTabId", () => {
    const { s, ids } = open3();
    const a = dash(s);
    expect(isQueryTabId(a.state, ids[0])).toBe(true);
    expect(isQueryTabId(a.state, a.id)).toBe(false);
    expect(isQueryTabId(a.state, OBJECT_TAB_ID)).toBe(false);
    expect(isDashTabId(a.state, a.id)).toBe(true);
    expect(isDashTabId(a.state, ids[0])).toBe(false);
    expect(isDashTabId(a.state, OBJECT_TAB_ID)).toBe(false);
  });

  it("sorgu sekmesi sıralaması dashboard'lara dokunmaz", () => {
    const { s, ids } = open3();
    const a = dash(s);
    const moved = moveTab(a.state, ids[2], 0);
    expect(moved.dashboards).toBe(a.state.dashboards);
    expect(moveTab(a.state, a.id, 0)).toBe(a.state);
  });
});

describe("createTabsStore", () => {
  it("dashboard açma: var olanı yeniden kullanır", () => {
    const store = createTabsStore();
    const a = store.openDashboard({ serverId: "s", databaseOid: 1, title: "D" });
    expect(a.created).toBe(true);
    store.activate(OBJECT_TAB_ID);
    expect(store.openDashboard({ serverId: "s", databaseOid: 1, title: "D" })).toEqual({ id: a.id, created: false });
    expect(store.state.activeId).toBe(a.id);
    store.close(a.id);
    expect(store.state.dashboards).toHaveLength(0);
    expect(store.state.activeId).toBe(OBJECT_TAB_ID);
  });

  it("saf işlevleri store'a uygular", () => {
    const store = createTabsStore();
    const id = store.open({ serverId: "s", serverName: "x", database: "d" });
    expect(store.state.activeId).toBe(id);
    store.rename(id, "yeni");
    expect(store.state.tabs[0].title).toBe("yeni");
    store.close(id);
    expect(store.state.tabs).toHaveLength(0);
    expect(store.state.activeId).toBe(OBJECT_TAB_ID);
  });
});
