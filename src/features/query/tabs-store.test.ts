import { describe, expect, it } from "vitest";
import {
  OBJECT_TAB_ID,
  activateTab,
  closeTab,
  createTabsStore,
  initialTabs,
  moveTab,
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

describe("createTabsStore", () => {
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
