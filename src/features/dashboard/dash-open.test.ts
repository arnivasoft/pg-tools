import { describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import { td } from "../../app/strings-dashboard";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import { OBJECT_TAB_ID, createTabsStore } from "../query/tabs-store";
import type { DashController } from "./dash-controller";
import { createDashOpener } from "./dash-open";

const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };

function tree(): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: TreeNode["kind"], label: string, serverId: string | null = null) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: true, serverId, flags, color: null, tag: null });
    s.parent.set(id, parent);
    s.children.set(parent, [...(s.children.get(parent) ?? []), id]);
  };
  add("srv", ROOT, "server", "prod", "s1");
  add("db", "srv", "database", "app");
  return s;
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

function setup() {
  const state = tree();
  const tabs = createTabsStore();
  const models = new Map<string, { status: string; serverId: string }>();
  const dash = {
    open: vi.fn(async (input: { id: string; serverId: string }) => {
      models.set(input.id, { status: "connecting", serverId: input.serverId });
      return input.id;
    }),
    setActive: vi.fn(),
    model: (id: string) => models.get(id),
    reopen: vi.fn(async () => {}),
  };
  const config = { id: "s1", name: "prod" } as ServerConfig;
  const deps = {
    dash: dash as unknown as DashController,
    api: { qtTargetOf: vi.fn(async (id: string) => ({ serverId: "s1", databaseOid: id === "db" ? 42 : null })) },
    tree: () => state,
    tabs,
    refreshServers: vi.fn(async () => [config]),
    connect: vi.fn(async (_id: string) => true),
    notify: vi.fn(),
  };
  return { opener: createDashOpener(deps), deps, dash, tabs, models };
}

describe("createDashOpener", () => {
  it("veritabanı düğümünden açar: başlık, kimlik, etkinleştirme", async () => {
    const s = setup();
    await s.opener.openFromNode("db");
    expect(s.deps.connect).toHaveBeenCalledWith("s1");
    expect(s.tabs.state.dashboards).toHaveLength(1);
    const tab = s.tabs.state.dashboards[0];
    expect(tab).toMatchObject({ serverId: "s1", databaseOid: 42, title: td.tabTitle("prod", "app") });
    expect(tab.title).toBe("Dashboard · prod/app");
    expect(s.tabs.state.activeId).toBe(tab.id);
    expect(s.dash.open).toHaveBeenCalledWith({ id: tab.id, serverId: "s1", serverName: "prod", databaseOid: 42 });
    expect(s.dash.setActive).toHaveBeenLastCalledWith(tab.id);
  });

  it("aynı sunucu + veritabanı ikinci kez: var olan etkinleşir, yeni oturum açılmaz", async () => {
    const s = setup();
    await s.opener.openFromNode("db");
    s.tabs.activate(OBJECT_TAB_ID);
    await s.opener.openFromNode("db");
    expect(s.tabs.state.dashboards).toHaveLength(1);
    expect(s.tabs.state.activeId).toBe(s.tabs.state.dashboards[0].id);
    expect(s.dash.open).toHaveBeenCalledTimes(1);
    // Sunucu düzeyi ayrı sekmedir.
    await s.opener.openFromNode("srv");
    expect(s.tabs.state.dashboards.map((d) => d.title)).toEqual(["Dashboard · prod/app", "Dashboard · prod"]);
  });

  it("tek seferde bir açma", async () => {
    const s = setup();
    const gate = deferred<{ serverId: string; databaseOid: number | null }>();
    s.deps.api.qtTargetOf.mockReturnValueOnce(gate.promise);
    const first = s.opener.openFromNode("db");
    await s.opener.openFromNode("srv"); // uçuştayken yok sayılır
    gate.resolve({ serverId: "s1", databaseOid: 42 });
    await first;
    expect(s.deps.api.qtTargetOf).toHaveBeenCalledTimes(1);
    expect(s.tabs.state.dashboards).toHaveLength(1);
    await s.opener.openFromNode("srv"); // bittikten sonra yeniden açılabilir
    expect(s.tabs.state.dashboards).toHaveLength(2);
  });

  it("bağlanma reddedilirse (parola iptal) sekme açılmaz; sunucu kaydı yoksa uyarı", async () => {
    const s = setup();
    s.deps.connect.mockResolvedValueOnce(false);
    await s.opener.openFromNode("db");
    expect(s.tabs.state.dashboards).toHaveLength(0);
    expect(s.dash.open).not.toHaveBeenCalled();
    s.deps.refreshServers.mockResolvedValueOnce([]);
    await s.opener.openFromNode("db");
    expect(s.deps.notify).toHaveBeenCalledWith(t.query.ctx.serverGone, "error");
    expect(s.tabs.state.dashboards).toHaveLength(0);
  });

  it("hata toast olarak bildirilir; bilinmeyen düğüm yok sayılır", async () => {
    const s = setup();
    s.deps.api.qtTargetOf.mockRejectedValueOnce({ code: "not_found", message: "yok", detail: null, hint: null });
    await s.opener.openFromNode("db");
    expect(s.deps.notify).toHaveBeenCalledWith("yok", "error");
    await s.opener.openFromNode("nope");
    expect(s.deps.api.qtTargetOf).toHaveBeenCalledTimes(1);
  });

  it("yeniden açma: yalnızca lost sekmede, önce sunucuyu bağlar", async () => {
    const s = setup();
    await s.opener.openFromNode("db");
    const id = s.tabs.state.dashboards[0].id;
    await s.opener.reopen(id); // connecting: yok sayılır
    expect(s.dash.reopen).not.toHaveBeenCalled();
    s.models.set(id, { status: "lost", serverId: "s1" });
    s.deps.connect.mockClear();
    s.deps.connect.mockResolvedValueOnce(false);
    await s.opener.reopen(id);
    expect(s.deps.connect).toHaveBeenCalledWith("s1");
    expect(s.dash.reopen).not.toHaveBeenCalled();
    await s.opener.reopen(id);
    expect(s.dash.reopen).toHaveBeenCalledWith(id);
  });
});
