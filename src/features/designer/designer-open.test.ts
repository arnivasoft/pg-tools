import { describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import { tdes } from "../../app/strings-designer";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { ROOT, emptyTree, type TreeState } from "../explorer/tree-model";
import { createTabsStore } from "../query/tabs-store";
import type { DesignerController } from "./designer-controller";
import { createDesignerOpener, type DesignerOpenDeps } from "./designer-open";

const flags = { connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
const server = (p: Partial<ServerConfig> = {}): ServerConfig => ({
  id: "S1", name: "srv", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
  ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: null, label: "test", readOnlyLock: false, ...p,
});

function tree(): TreeState {
  const s = emptyTree();
  const add = (id: string, parent: string, kind: NodeKind, label: string, extra: Partial<TreeNode> = {}) => {
    s.nodes.set(id, { id, kind, label, detail: null, hasChildren: false, serverId: null, flags, color: null, tag: null, ...extra });
    s.parent.set(id, parent);
  };
  add("srv", ROOT, "server", "srv", { serverId: "S1" });
  add("db", "srv", "database", "app");
  add("coll-seq", "db", "sequenceCollection", "Sequences");
  add("seq", "coll-seq", "sequence", "orders_id_seq");
  return s;
}

const createTarget: DdlTarget = {
  serverId: "S1", databaseOid: 5, object: null, create: { kind: "sequence", context: { schemaOid: 10, tableOid: null } }, refreshNodeId: "coll-seq",
};
const objectTarget: DdlTarget = {
  serverId: "S1", databaseOid: 5, object: { kind: "sequence", oid: 77 }, create: null, refreshNodeId: "coll-seq",
};

function setup(target: DdlTarget | ((id: string) => Promise<DdlTarget>), cfg: ServerConfig = server()) {
  const log: string[] = [];
  const store = createTabsStore();
  const tabs = {
    get state() {
      return store.state;
    },
    openDesigner: vi.fn((input: Parameters<typeof store.openDesigner>[0]) => {
      const r = store.openDesigner(input);
      log.push(`tab:${r.id}:${r.created}`);
      return r;
    }),
  };
  const designer = {
    openCreate: vi.fn(async (i: { tabId: string }) => void log.push(`create:${i.tabId}`)),
    openAlter: vi.fn(async (i: { tabId: string }) => void log.push(`alter:${i.tabId}`)),
    requestFocus: vi.fn((tabId: string, field: string) => void log.push(`focus:${tabId}:${field}`)),
  };
  const deps = {
    designer: designer as unknown as DesignerController,
    tabs,
    api: { ddlTargetOf: vi.fn(typeof target === "function" ? target : async () => target) },
    tree,
    refreshServers: vi.fn(async () => [cfg]),
    connect: vi.fn(async () => true),
    notify: vi.fn(),
    openDrop: vi.fn(),
  };
  const opener = createDesignerOpener(deps as unknown as DesignerOpenDeps);
  return { opener, deps, designer, tabs, store, log };
}

describe("designer-open", () => {
  it("create: sekme açılır ve openCreate EŞZAMANLI hemen ardından çağrılır", async () => {
    const { opener, designer, store, log, deps } = setup(createTarget);
    await opener.open("coll-seq", "create");
    expect(deps.api.ddlTargetOf).toHaveBeenCalledWith("coll-seq");
    expect(deps.connect).toHaveBeenCalledWith("S1");
    const tab = store.state.designers[0];
    expect(log).toEqual([`tab:${tab.id}:true`, `create:${tab.id}`]);
    expect(tab.where).toBe("srv/app");
    expect(tab.title).toBe(tdes.tab.create("Sequence", "srv", "app"));
    expect(designer.openCreate).toHaveBeenCalledWith({
      tabId: tab.id, serverId: "S1", databaseOid: 5, create: createTarget.create, refreshNodeId: "coll-seq",
    });
    // İkinci "Oluştur" ayrı sekme açar.
    await opener.open("coll-seq", "create");
    expect(store.state.designers).toHaveLength(2);
    expect(designer.openCreate).toHaveBeenCalledTimes(2);
  });

  it("openCreate, openDesigner ile aynı eşzamanlı adımda (araya await girmeden) çağrılır", async () => {
    const { opener, tabs, designer } = setup(createTarget);
    tabs.openDesigner.mockImplementationOnce((input) => {
      const r = createTabsStore().openDesigner(input);
      queueMicrotask(() => expect(designer.openCreate).toHaveBeenCalledOnce());
      return r;
    });
    await opener.open("coll-seq", "create");
    expect(designer.openCreate).toHaveBeenCalledOnce();
  });

  it("edit: alter sekmesi + openAlter; ikinci edit mevcut sekmeyi etkinleştirir", async () => {
    const { opener, designer, store, log } = setup(objectTarget);
    await opener.open("seq", "edit");
    const id = store.state.designers[0].id;
    expect(log).toEqual([`tab:${id}:true`, `alter:${id}`]);
    expect(store.state.designers[0].title).toBe(tdes.tab.edit("Sequence", "orders_id_seq"));
    expect(designer.openAlter).toHaveBeenCalledWith({
      tabId: id, serverId: "S1", databaseOid: 5, object: objectTarget.object, refreshNodeId: "coll-seq",
    });
    store.activate("object");
    await opener.open("seq", "edit");
    expect(store.state.designers).toHaveLength(1);
    expect(store.state.activeId).toBe(id);
    expect(designer.openAlter).toHaveBeenCalledOnce();
    expect(designer.requestFocus).not.toHaveBeenCalled();
  });

  it("rename: yeni sekmede açtıktan sonra, mevcut sekmede doğrudan `name` odağı ister", async () => {
    const { opener, store, log } = setup(objectTarget);
    await opener.open("seq", "rename");
    const id = store.state.designers[0].id;
    expect(log).toEqual([`tab:${id}:true`, `alter:${id}`, `focus:${id}:name`]);
    await opener.open("seq", "rename");
    expect(log.slice(3)).toEqual([`tab:${id}:false`, `focus:${id}:name`]);
  });

  it("drop: sekme açmaz, silme diyaloğunu hedefle açar", async () => {
    const { opener, deps, store } = setup(objectTarget);
    await opener.open("seq", "drop");
    expect(store.state.designers).toHaveLength(0);
    expect(deps.openDrop).toHaveBeenCalledWith({
      serverId: "S1", databaseOid: 5, object: objectTarget.object, refreshNodeId: "coll-seq", name: "orders_id_seq",
    });
  });

  it("ddlTargetOf hedef vermezse / databaseOid yoksa bildirir, sekme açmaz", async () => {
    const empty: DdlTarget = { serverId: "S1", databaseOid: 5, object: null, create: null, refreshNodeId: null };
    for (const [target, mode] of [
      [empty, "edit"], [empty, "create"], [createTarget, "edit"], [objectTarget, "create"],
      [{ ...objectTarget, databaseOid: null }, "edit"], [{ ...createTarget, databaseOid: null }, "create"],
    ] as const) {
      const { opener, deps, store } = setup(target);
      await opener.open("seq", mode);
      expect(deps.notify).toHaveBeenCalledWith(tdes.open.none, "info");
      expect(deps.connect).not.toHaveBeenCalled();
      expect(store.state.designers).toHaveLength(0);
      expect(deps.openDrop).not.toHaveBeenCalled();
    }
  });

  it("kilitli sunucu kaydı: değiştiren açma reddedilir (bağlanmaz, sekme/diyalog yok)", async () => {
    for (const mode of ["create", "edit", "rename", "drop"] as const) {
      const { opener, deps, store } = setup(mode === "create" ? createTarget : objectTarget, server({ readOnlyLock: true }));
      await opener.open(mode === "create" ? "coll-seq" : "seq", mode);
      expect(deps.notify).toHaveBeenCalledWith(tdes.open.locked, "error");
      expect(deps.connect).not.toHaveBeenCalled();
      expect(store.state.designers).toHaveLength(0);
      expect(deps.openDrop).not.toHaveBeenCalled();
    }
  });

  it("sunucu kaydı yoksa bildirir", async () => {
    const { opener, deps, store } = setup({ ...objectTarget, serverId: "GONE" });
    await opener.open("seq", "edit");
    expect(deps.notify).toHaveBeenCalledWith(t.query.ctx.serverGone, "error");
    expect(store.state.designers).toHaveLength(0);
  });

  it("connect false (parola iptal) → sekme açılmaz", async () => {
    const { opener, deps, store, designer } = setup(objectTarget);
    deps.connect.mockResolvedValue(false);
    await opener.open("seq", "edit");
    expect(store.state.designers).toHaveLength(0);
    expect(designer.openAlter).not.toHaveBeenCalled();
  });

  it("hata → errorText bildirimi; bayrak bırakılır", async () => {
    const { opener, deps, store } = setup(async () => {
      throw { code: "unsupported", message: "desteklenmiyor", detail: null, hint: null };
    });
    await opener.open("seq", "edit");
    expect(deps.notify).toHaveBeenCalledWith(expect.stringContaining("desteklenmiyor"), "error");
    deps.api.ddlTargetOf.mockResolvedValue(objectTarget);
    await opener.open("seq", "edit");
    expect(store.state.designers).toHaveLength(1);
  });

  it("tek seferde bir açma: ilk açma beklerken ikinci istek yok sayılır", async () => {
    let release!: (t: DdlTarget) => void;
    const gate = new Promise<DdlTarget>((res) => (release = res));
    const { opener, deps, store } = setup(() => gate);
    const first = opener.open("seq", "edit");
    await opener.open("seq", "rename");
    expect(deps.api.ddlTargetOf).toHaveBeenCalledOnce();
    release(objectTarget);
    await first;
    expect(store.state.designers).toHaveLength(1);
    expect(deps.notify).not.toHaveBeenCalled();
  });

  it("bilinmeyen düğüm: hiçbir şey yapmaz", async () => {
    const { opener, deps } = setup(objectTarget);
    await opener.open("ghost", "edit");
    expect(deps.api.ddlTargetOf).not.toHaveBeenCalled();
  });

  it("databaseName: açılan hedefin veritabanı adını hatırlar", async () => {
    const { opener } = setup(objectTarget);
    expect(opener.databaseName("S1", 5)).toBeNull();
    await opener.open("seq", "edit");
    expect(opener.databaseName("S1", 5)).toBe("app");
  });
});
