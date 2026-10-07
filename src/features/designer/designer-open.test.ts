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
  add("dbs", "srv", "databaseCollection", "Databases");
  add("db", "dbs", "database", "app");
  add("roles", "srv", "roleCollection", "Login/Group Roles");
  add("rl", "roles", "role", "app_user");
  add("coll-view", "sch", "viewCollection", "Views");
  add("coll-seq", "db", "sequenceCollection", "Sequences");
  add("seq", "coll-seq", "sequence", "orders_id_seq");
  add("coll-sch", "db", "schemaCollection", "Schemas");
  add("sch", "coll-sch", "schema", "public");
  add("coll-tbl", "sch", "tableCollection", "Tables");
  add("tbl", "coll-tbl", "table", "orders");
  add("coll-col", "tbl", "columnCollection", "Columns");
  add("col", "coll-col", "column", "id");
  return s;
}

const createTarget: DdlTarget = {
  serverId: "S1", databaseOid: 5, object: null, create: { kind: "sequence", context: { schemaOid: 10, tableOid: null } }, focus: null, refreshNodeId: "coll-seq",
};
const objectTarget: DdlTarget = {
  serverId: "S1", databaseOid: 5, object: { kind: "sequence", oid: 77 }, create: null, focus: null, refreshNodeId: "coll-seq",
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
    activate: vi.fn((id: string) => store.activate(id)),
    rename: vi.fn((id: string, title: string) => store.rename(id, title)),
  };
  const designer = {
    openCreate: vi.fn(async (i: { tabId: string }) => void log.push(`create:${i.tabId}`)),
    openAlter: vi.fn(async (i: { tabId: string }) => void log.push(`alter:${i.tabId}`)),
    findAlter: vi.fn((): string | null => null),
    ids: vi.fn((): string[] => []),
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

  it("focus (kolon/kısıt) hedefinde drop ve rename reddedilir; edit açılır", async () => {
    const focused: DdlTarget = { ...objectTarget, focus: { kind: "column", id: 1 } };
    for (const mode of ["drop", "rename"] as const) {
      const { opener, deps, store } = setup(focused);
      await opener.open("seq", mode);
      expect(deps.notify).toHaveBeenCalledWith(tdes.open.rowInTable, "info");
      expect(deps.openDrop).not.toHaveBeenCalled();
      expect(deps.connect).not.toHaveBeenCalled();
      expect(store.state.designers).toHaveLength(0);
    }
    const { opener, store } = setup(focused);
    await opener.open("seq", "edit");
    expect(store.state.designers).toHaveLength(1);
  });

  it("ddlTargetOf hedef vermezse / databaseOid yoksa bildirir, sekme açmaz", async () => {
    const empty: DdlTarget = { serverId: "S1", databaseOid: 5, object: null, create: null, focus: null, refreshNodeId: null };
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

const tableTarget = (focus: DdlTarget["focus"]): DdlTarget => ({
  serverId: "S1", databaseOid: 5, object: { kind: "table", oid: 500 }, create: null, focus, refreshNodeId: "tbl",
});

describe("designer-open Task 4", () => {
  it("kolon düğümünden edit: tablo sekmesi açılır ve requestFocus(id, 'column:3')", async () => {
    const { opener, store, designer } = setup(tableTarget({ kind: "column", id: 3 }));
    await opener.open("col", "edit");
    const tab = store.state.designers[0];
    expect(designer.openAlter).toHaveBeenCalledTimes(1);
    expect(designer.requestFocus).toHaveBeenCalledExactlyOnceWith(tab.id, "column:3");
    expect(tab.title).toBe(tdes.tab.edit("Table", "orders"));
  });

  it("context budama: canlı sekmenin bağlamı korunur, ölü sekmeninki silinir; create sekmesi koleksiyon düğümünü saklar", async () => {
    const a = setup(async (id) => (id === "seq" ? objectTarget : tableTarget(null)));
    await a.opener.open("tbl", "edit");
    const first = a.store.state.designers[0].id;
    a.designer.ids.mockReturnValue([first]);
    await a.opener.open("seq", "edit");
    const second = a.store.state.designers[1].id;
    expect(a.opener.context(first)).not.toBeNull();
    expect(a.opener.context(second)).not.toBeNull();
    a.designer.ids.mockReturnValue([second]);
    a.designer.findAlter.mockReturnValue(second);
    await a.opener.open("col", "edit");
    expect(a.opener.context(first)).toBeNull();
    expect(a.opener.context(second)).not.toBeNull();
    const c = setup(createTarget);
    await c.opener.open("coll-seq", "create");
    expect(c.opener.context(c.store.state.designers[0].id)).toEqual({ databaseNodeId: "db", tableNodeId: null, collectionNodeId: "coll-seq" });
  });

  it("kısıt odağı mevcut sekmede: yeni sekme açmadan activate + requestFocus('constraint:40')", async () => {
    const { opener, store, designer, tabs } = setup(tableTarget({ kind: "constraint", id: 40 }));
    designer.findAlter.mockReturnValue("D9");
    await opener.open("tbl", "edit");
    expect(tabs.activate).toHaveBeenCalledExactlyOnceWith("D9");
    expect(designer.requestFocus).toHaveBeenCalledExactlyOnceWith("D9", "constraint:40");
    expect(tabs.openDesigner).not.toHaveBeenCalled();
    expect(store.state.designers).toHaveLength(0);
  });

  it("findAlter eşleşirse (create'ten dönüşen sekme) openDesigner çağrılmaz, activate edilir", async () => {
    const { opener, designer, tabs } = setup(objectTarget);
    designer.findAlter.mockReturnValue("D1");
    await opener.open("seq", "edit");
    expect(designer.findAlter).toHaveBeenCalledWith("S1", 5, objectTarget.object);
    expect(tabs.activate).toHaveBeenCalledWith("D1");
    expect(tabs.openDesigner).not.toHaveBeenCalled();
    expect(designer.openAlter).not.toHaveBeenCalled();
  });

  it("focus hedefinde drop/rename: tdes.open.rowInTable bildirimi, bağlanma yok, sekme yok", async () => {
    for (const mode of ["drop", "rename"] as const) {
      const { opener, deps, store } = setup(tableTarget({ kind: "column", id: 1 }));
      await opener.open("col", mode);
      expect(deps.notify).toHaveBeenCalledWith(tdes.open.rowInTable, "info");
      expect(deps.connect).not.toHaveBeenCalled();
      expect(store.state.designers).toHaveLength(0);
    }
  });

  it("context: tablo düğümünden ve kolon düğümünden açılışta tableNodeId = tablo düğümü; databaseNodeId = db", async () => {
    for (const nodeId of ["tbl", "col"]) {
      const { opener, store } = setup(tableTarget(nodeId === "col" ? { kind: "column", id: 3 } : null));
      await opener.open(nodeId, "edit");
      expect(opener.context(store.state.designers[0].id)).toEqual({ databaseNodeId: "db", tableNodeId: "tbl" });
    }
    const { opener, store } = setup(objectTarget);
    await opener.open("seq", "edit");
    expect(opener.context(store.state.designers[0].id)).toEqual({ databaseNodeId: "db", tableNodeId: null });
    expect(opener.context("yok")).toBeNull();
  });
});

const roleTarget: DdlTarget = {
  serverId: "S1", databaseOid: null, object: { kind: "role", oid: 16384 }, create: null, focus: null, refreshNodeId: "roles",
};
const dbTarget: DdlTarget = {
  serverId: "S1", databaseOid: null, object: { kind: "database", oid: 5 }, create: null, focus: null, refreshNodeId: "dbs",
};

describe("designer-open Plan 15: sunucu kapsamı", () => {
  it("rol düğümünden edit: databaseOid null, where = sunucu adı, başlıkta veritabanı adı yok", async () => {
    const { opener, store, designer, tabs } = setup(roleTarget);
    await opener.open("rl", "edit");
    const tab = store.state.designers[0];
    expect(tabs.openDesigner).toHaveBeenCalledWith(expect.objectContaining({ databaseOid: null, where: "srv" }));
    expect(tab).toMatchObject({ databaseOid: null, where: "srv", key: "alter:role:S1:null:16384" });
    expect(tab.title).toBe(tdes.tab.edit("Role", "app_user"));
    expect(designer.openAlter).toHaveBeenCalledWith({ tabId: tab.id, serverId: "S1", databaseOid: null, object: roleTarget.object, refreshNodeId: "roles" });
    expect(designer.findAlter).toHaveBeenCalledWith("S1", null, roleTarget.object);
  });

  it("rol create: başlık tdes.tab.create(kind, sunucu, null)", async () => {
    const t: DdlTarget = { ...roleTarget, object: null, create: { kind: "role", context: { schemaOid: null, tableOid: null } } };
    const { opener, store, designer } = setup(t);
    await opener.open("roles", "create");
    const tab = store.state.designers[0];
    expect(tab.title).toBe(tdes.tab.create("Role", "srv", null));
    expect(tab.where).toBe("srv");
    expect(designer.openCreate).toHaveBeenCalledWith(expect.objectContaining({ databaseOid: null }));
  });

  it("veritabanı düğümünden Database edit: databaseOid null (düğüm veritabanının altında olsa da), where = sunucu", async () => {
    const { opener, store, designer } = setup(dbTarget);
    await opener.open("db", "edit");
    const tab = store.state.designers[0];
    expect(tab).toMatchObject({ databaseOid: null, where: "srv" });
    expect(designer.openAlter).toHaveBeenCalledWith(expect.objectContaining({ databaseOid: null, object: dbTarget.object }));
    expect(opener.databaseName("S1", 5)).toBeNull(); // dbNames yalnız veritabanı kapsamlıda
  });

  it("savunma: arka uç sunucu kapsamlı türe databaseOid verse bile null gönderilir", async () => {
    const { opener, store, designer } = setup({ ...dbTarget, databaseOid: 5 });
    await opener.open("db", "edit");
    expect(store.state.designers[0]).toMatchObject({ databaseOid: null, where: "srv" });
    expect(designer.openAlter).toHaveBeenCalledWith(expect.objectContaining({ databaseOid: null }));
  });

  it("databaseOid null + view hedefi → tdes.open.none", async () => {
    const t: DdlTarget = { serverId: "S1", databaseOid: null, object: { kind: "view", oid: 900 }, create: null, focus: null, refreshNodeId: "coll-view" };
    const { opener, deps, store } = setup(t);
    await opener.open("coll-view", "edit");
    expect(deps.notify).toHaveBeenCalledWith(tdes.open.none, "info");
    expect(store.state.designers).toHaveLength(0);
    expect(deps.connect).not.toHaveBeenCalled();
  });

  it("drop: rol/database için openDrop({ databaseOid: null })", async () => {
    for (const [target, node, name] of [[roleTarget, "rl", "app_user"], [dbTarget, "db", "app"]] as const) {
      const { opener, deps } = setup(target);
      await opener.open(node, "drop");
      expect(deps.openDrop).toHaveBeenCalledWith({ serverId: "S1", databaseOid: null, object: target.object, refreshNodeId: target.refreshNodeId, name });
    }
  });
});
