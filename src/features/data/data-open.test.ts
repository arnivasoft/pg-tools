import { describe, expect, it, vi } from "vitest";
import { tdata } from "../../app/strings-data";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { IpcError } from "../../ipc/errors";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import { createTabsStore } from "../query/tabs-store";
import type { DataTabCtl } from "./data-controller";
import { createDataOpener, dataTabKey } from "./data-opener";

const startData = vi.fn();
vi.mock("./DataTabs", () => ({ startData: (...a: unknown[]) => startData(...a) }));

const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
function tree(): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: NodeKind, label: string, extra: Partial<TreeNode> = {}) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: false, serverId: null, flags, color: null, tag: null, ...extra });
    s.parent.set(id, parent);
  };
  add("srv", ROOT, "server", "srv", { serverId: "S1" });
  add("db", "srv", "database", "app");
  add("sch", "db", "schema", "sales");
  add("v", "sch", "view", "v_orders");
  return s;
}
const cfg: ServerConfig = {
  id: "S1", name: "prod-1", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
  ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: null, label: null, readOnlyLock: false,
};
const target: DdlTarget = { serverId: "S1", databaseOid: 5, object: { kind: "view", oid: 77 }, create: null, focus: null, refreshNodeId: null };

function setup() {
  const tabs = createTabsStore();
  const ctl = { openTab: vi.fn((i: { tabId: string }) => i.tabId) };
  const notify = vi.fn();
  const ddlTargetOf = vi.fn(async (): Promise<DdlTarget> => target);
  const connect = vi.fn(async () => true);
  const opener = createDataOpener({
    ctl: ctl as unknown as Pick<DataTabCtl, "openTab">, tabs, api: { ddlTargetOf }, tree, refreshServers: async () => [cfg], connect, notify,
  });
  return { tabs, ctl, notify, ddlTargetOf, connect, opener };
}

describe("data-open", () => {
  it("tek uçuş: süren açılış varken ikinci istek yok sayılır", async () => {
    const s = setup();
    await Promise.all([s.opener.open("v"), s.opener.open("v")]);
    expect(s.ddlTargetOf).toHaveBeenCalledTimes(1);
    expect(s.tabs.state.data).toHaveLength(1);
    expect(s.connect).toHaveBeenCalledWith("S1");
  });

  it("aynı nesnenin ikinci açılışı mevcut sekmeyi etkinleştirir (denetleyici girdisi tek)", async () => {
    const s = setup();
    await s.opener.open("v");
    const id = s.tabs.state.data[0].id;
    s.tabs.activate("object");
    await s.opener.open("v");
    expect(s.tabs.state.data).toHaveLength(1);
    expect(s.tabs.state.activeId).toBe(id);
    expect(s.ctl.openTab).toHaveBeenCalledTimes(1);
    expect(s.ctl.openTab).toHaveBeenCalledWith({ tabId: id, serverId: "S1", databaseOid: 5, object: { kind: "view", oid: 77 } });
    expect(s.tabs.state.data[0]).toMatchObject({ key: dataTabKey("S1", 5, 77), where: "prod-1/app.sales", title: tdata.tab.title("v_orders") });
  });

  it("ddlTargetOf hatası toast; bağlanılamazsa sekme yok", async () => {
    const s = setup();
    s.ddlTargetOf.mockRejectedValueOnce(new IpcError({ code: "not_found", message: "Düğüm yok", detail: null, hint: null }));
    await s.opener.open("v");
    expect(s.notify).toHaveBeenCalledWith("Düğüm yok", "error");
    s.connect.mockResolvedValueOnce(false);
    await s.opener.open("v");
    expect(s.tabs.state.data).toHaveLength(0);
    await s.opener.open("missing-node");
    expect(s.ddlTargetOf).toHaveBeenCalledTimes(2);
  });

  it("createData: denetleyici DataTabs parçasıyla iner; ctl yüzü yüklenmeden güvenli, sonra iletir", async () => {
    const inner = {
      isDirty: vi.fn(() => true), close: vi.fn(), reloadNow: vi.fn(() => false), serverSessionsClosed: vi.fn(), model: vi.fn(() => null),
    };
    const open = vi.fn(async () => {});
    startData.mockReturnValue({ ctl: inner, open });
    const { createData } = await import("./data-open");
    const deps = { notify: vi.fn() } as unknown as Parameters<typeof createData>[0];
    const d = createData(deps);
    expect(d.ctl.isDirty("g1")).toBe(false);
    expect(d.ctl.model("g1")).toBeNull();
    await d.open("v");
    expect(startData).toHaveBeenCalledWith(deps);
    expect(open).toHaveBeenCalledWith("v");
    expect(d.ctl.isDirty("g1")).toBe(true);
    expect(d.ctl.reloadNow("g1")).toBe(false);
    d.ctl.serverSessionsClosed("S1", true);
    expect(inner.serverSessionsClosed).toHaveBeenCalledWith("S1", true);
    d.ctl.close("g1");
    expect(inner.close).toHaveBeenCalledWith("g1");
  });
});
