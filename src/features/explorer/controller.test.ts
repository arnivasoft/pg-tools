import { describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { IpcError } from "../../ipc/errors";
import { ExplorerController, type ExplorerDeps } from "./controller";
import { ROOT, emptyTree, flatten, type TreeState } from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false };
const n = (id: string, extra: Partial<TreeNode> = {}): TreeNode =>
  ({ id, kind: "table", label: id, detail: null, hasChildren: true, serverId: null, flags, ...extra });
const err = (code: string): IpcError => new IpcError({ code, message: code, detail: null, hint: null } as ErrorPayload);

function setup(children: Record<string, TreeNode[]>, deps: Partial<ExplorerDeps> = {}) {
  let state: TreeState = emptyTree();
  const listChildren = vi.fn(async (id: string) => children[id] ?? []);
  const connectServer = vi.fn(async (_serverId: string, _password: string | null, _remember: boolean) => ({
    versionNum: 180000,
    version: "18.0",
  }));
  const requestPassword = vi.fn(async () => null);
  const c = new ExplorerController(
    () => state,
    (s) => (state = s),
    { api: { listChildren, connectServer }, requestPassword, ...deps },
  );
  return { c, get: () => state, listChildren, connectServer, requestPassword };
}

describe("ExplorerController", () => {
  it("loads children once and toggles expansion", async () => {
    const { c, get, listChildren } = setup({ [ROOT]: [n("a")], a: [n("a1")] });
    await c.load(ROOT);
    await c.toggle("a");
    expect(flatten(get()).map((r) => r.id)).toEqual(["a", "a1"]);
    await c.toggle("a");
    await c.toggle("a");
    expect(listChildren).toHaveBeenCalledTimes(2);
  });

  it("connects a disconnected server before expanding", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get, connectServer } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    expect(connectServer).toHaveBeenCalledWith("u1", null, false);
    const node = get().nodes.get("srv")!;
    expect([node.flags.connected, node.detail]).toEqual([true, "18.0"]);
    expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
  });

  it("asks for a password when required and remembers the choice", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("password_required"))
      .mockResolvedValueOnce({ versionNum: 160004, version: "16.4" });
    const requestPassword = vi.fn(async () => ({ password: "p", remember: true }));
    const { c } = setup({ [ROOT]: [srv] }, { requestPassword, api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword).toHaveBeenCalledWith(expect.objectContaining({ id: "srv" }), false, null);
    expect(connectServer).toHaveBeenLastCalledWith("u1", "p", true);
  });

  it("auth error re-prompts and retries with new password", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("auth"))
      .mockRejectedValueOnce(err("auth"))
      .mockResolvedValueOnce({ versionNum: 180000, version: "18.0" });
    const requestPassword = vi
      .fn()
      .mockResolvedValueOnce({ password: "yanlis", remember: false })
      .mockResolvedValueOnce({ password: "dogru", remember: true });
    const { c, get } = setup({ [ROOT]: [srv] }, { requestPassword, api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword.mock.calls.map((call) => call[1])).toEqual([true, true]);
    expect(requestPassword.mock.calls[0][2]).toEqual(expect.objectContaining({ code: "auth" }));
    expect(connectServer).toHaveBeenLastCalledWith("u1", "dogru", true);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  });

  it("cancelled prompt leaves the server collapsed without error", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi.fn().mockRejectedValue(err("password_required"));
    const { c, get } = setup({ [ROOT]: [srv] }, { api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    const row = flatten(get())[0];
    expect([row.expanded, row.loading, row.error]).toEqual([false, false, null]);
  });

  it("network error is shown on the server node", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi.fn().mockRejectedValue(err("network"));
    const { c, get } = setup({ [ROOT]: [srv] }, { api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(flatten(get())[0].error?.code).toBe("network");
  });

  it("load errors are attached to the parent and cleared by a successful reload", async () => {
    const listChildren = vi.fn().mockRejectedValueOnce(err("query")).mockResolvedValueOnce([n("a")]);
    const { c, get } = setup({}, { api: { listChildren, connectServer: vi.fn() } });
    await c.load(ROOT);
    expect(get().errors.get(ROOT)?.code).toBe("query");
    expect(get().loading.has(ROOT)).toBe(false);
    await c.load(ROOT);
    expect(get().errors.size).toBe(0);
    expect(flatten(get()).map((r) => r.id)).toEqual(["a"]);
  });

  it("keyboard intents select, expand and refresh", async () => {
    const { c, get, listChildren } = setup({ [ROOT]: [n("a"), n("b")], a: [n("a1")] });
    await c.load(ROOT);
    expect(c.handleKey("ArrowDown")).toBe(true);
    expect(get().selected).toBe("a");
    expect(c.handleKey("ArrowRight")).toBe(true);
    await vi.waitFor(() => expect(get().expanded.has("a")).toBe(true));
    expect(c.handleKey("q")).toBe(false);
    c.handleKey("F5");
    await vi.waitFor(() => expect(listChildren).toHaveBeenCalledTimes(3));
    expect(listChildren.mock.calls.at(-1)?.[0]).toBe("a");
  });

  it("reloadServers refreshes root and loaded groups", async () => {
    const group = n("grp:G", { kind: "serverGroup" });
    const { c, listChildren } = setup({ [ROOT]: [group], "grp:G": [] });
    await c.load(ROOT);
    await c.expand("grp:G");
    listChildren.mockClear();
    await c.reloadServers();
    expect(listChildren.mock.calls.map((call) => call[0])).toEqual([ROOT, "grp:G"]);
  });
});

describe("ExplorerController robustness", () => {
  const srv = n("srv", { kind: "server", serverId: "u1" });

  it("deduplicates concurrent expand on a disconnected server", async () => {
    let release!: (a: { password: string; remember: boolean }) => void;
    const requestPassword = vi.fn(() => new Promise<{ password: string; remember: boolean }>((r) => (release = r)));
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("password_required"))
      .mockResolvedValueOnce({ versionNum: 180000, version: "18.0" });
    const listChildren = vi.fn(async (id: string) => (id === ROOT ? [srv] : [n("dbs")]));
    const { c } = setup({}, { requestPassword, api: { listChildren, connectServer } });
    await c.load(ROOT);
    const p = Promise.all([c.expand("srv"), c.expand("srv")]);
    await vi.waitFor(() => expect(requestPassword).toHaveBeenCalledTimes(1));
    release({ password: "p", remember: false });
    await p;
    expect(connectServer).toHaveBeenCalledTimes(2);
    expect(requestPassword).toHaveBeenCalledTimes(1);
    expect(listChildren.mock.calls.filter((call) => call[0] === "srv")).toHaveLength(1);
  });

  it("requestPassword throwing sets an error and returns false", async () => {
    const connectServer = vi.fn().mockRejectedValue(err("password_required"));
    const requestPassword = vi.fn(async () => {
      throw err("query");
    });
    const { c, get } = setup(
      {},
      { requestPassword, api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } },
    );
    await c.load(ROOT);
    await expect(c.ensureConnected("srv")).resolves.toBe(false);
    expect(flatten(get())[0].error?.code).toBe("query");
  });

  it("does not resurrect or expand a node removed during the password prompt", async () => {
    let release!: (a: { password: string; remember: boolean }) => void;
    const requestPassword = vi.fn(() => new Promise<{ password: string; remember: boolean }>((r) => (release = r)));
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("password_required"))
      .mockResolvedValue({ versionNum: 1, version: "1" });
    let rootList = [srv];
    const listChildren = vi.fn(async (id: string) => (id === ROOT ? rootList : [n("dbs")]));
    const { c, get } = setup({}, { requestPassword, api: { listChildren, connectServer } });
    await c.load(ROOT);
    const p = c.expand("srv");
    await vi.waitFor(() => expect(requestPassword).toHaveBeenCalled());
    rootList = [];
    await c.load(ROOT);
    expect(get().nodes.has("srv")).toBe(false);
    release({ password: "p", remember: false });
    await p;
    expect(connectServer).toHaveBeenCalledTimes(2);
    expect(get().expanded.has("srv")).toBe(false);
    expect(get().nodes.has("srv")).toBe(false);
    expect(get().errors.has("srv")).toBe(false);
    expect(get().loading.has("srv")).toBe(false);
    expect(listChildren.mock.calls.some((call) => call[0] === "srv")).toBe(false);
  });
});

describe("ExplorerController password prompt details", () => {
  it("passes the auth error (with the server's detail) to the retry prompt", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const auth = new IpcError({ code: "auth", message: "Parola hatalı", detail: "password authentication failed for user \"x\"", hint: null });
    const connectServer = vi.fn().mockRejectedValueOnce(auth).mockResolvedValueOnce({ versionNum: 1, version: "1" });
    const requestPassword = vi.fn(async () => ({ password: "p", remember: false }));
    const { c } = setup({}, { requestPassword, api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword).toHaveBeenCalledWith(expect.objectContaining({ id: "srv" }), true, auth.payload);
  });

  it("a non-auth connect error (e.g. pg_hba rejection) is shown on the node, not re-prompted", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi.fn().mockRejectedValue(err("connect"));
    const requestPassword = vi.fn(async () => ({ password: "p", remember: false }));
    const { c, get } = setup({}, { requestPassword, api: { listChildren: async (id: string) => (id === ROOT ? [srv] : []), connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword).not.toHaveBeenCalled();
    expect(get().errors.get("srv")?.code).toBe("connect");
  });
});

describe("ExplorerController lost sessions", () => {
  const srv = n("srv", { kind: "server", serverId: "u1" });

  for (const code of ["connection_lost", "not_connected"]) {
    it(`${code} while loading marks the owning server disconnected and the next expand reconnects`, async () => {
      let lost = false;
      const listChildren = vi.fn(async (id: string) => {
        if (id === ROOT) return [srv];
        if (id === "srv") return [n("dbs", { kind: "databaseCollection" })];
        if (lost) {
          lost = false;
          throw err(code);
        }
        return [n("db1", { kind: "database", hasChildren: false })];
      });
      const connectServer = vi.fn(async () => ({ versionNum: 180000, version: "18.0" }));
      const { c, get } = setup({}, { api: { listChildren, connectServer } });
      await c.load(ROOT);
      await c.expand("srv");
      await c.expand("dbs");
      c.select("db1");
      lost = true;
      await c.refresh("dbs");

      const server = get().nodes.get("srv")!;
      expect([server.flags.connected, server.detail]).toEqual([false, null]);
      expect(get().errors.get("srv")?.code).toBe(code);
      expect(get().errors.has("dbs")).toBe(false);
      expect(get().children.has("srv")).toBe(false);
      expect(get().nodes.has("dbs")).toBe(false);
      expect(get().selected).toBe("srv");
      expect(flatten(get()).map((r) => r.id)).toEqual(["srv"]);

      await c.expand("srv");
      expect(connectServer).toHaveBeenCalledTimes(2);
      expect(get().nodes.get("srv")!.flags.connected).toBe(true);
      expect(get().errors.has("srv")).toBe(false);
      expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
    });
  }

  it("other load errors stay on the loading node", async () => {
    const listChildren = vi.fn(async (id: string) => {
      if (id === ROOT) return [srv];
      if (id === "srv") return [n("dbs")];
      throw err("query");
    });
    const { c, get } = setup({}, { api: { listChildren, connectServer: vi.fn(async () => ({ versionNum: 1, version: "1" })) } });
    await c.load(ROOT);
    await c.expand("srv");
    await c.expand("dbs");
    expect(get().errors.get("dbs")?.code).toBe("query");
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  });

  it("sessionLost marks the owning server disconnected, clears its subtree and the next expand reconnects", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get, connectServer } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    c.select("dbs");

    const payload: ErrorPayload = { code: "connection_lost", message: "Bağlantı koptu", detail: "x", hint: null };
    expect(c.sessionLost("dbs", payload)).toBe(true);

    const server = get().nodes.get("srv")!;
    expect([server.flags.connected, server.detail]).toEqual([false, null]);
    expect(get().nodes.has("dbs")).toBe(false);
    expect(get().errors.get("srv")?.code).toBe("connection_lost");
    expect(get().selected).toBe("srv");

    await c.expand("srv");
    expect(connectServer).toHaveBeenCalledTimes(2);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
  });

  it("sessionLost ignores unrelated error codes and nodes outside any server", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get } = setup({ [ROOT]: [srv, n("grp:G", { kind: "serverGroup" })], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    const query: ErrorPayload = { code: "query", message: "m", detail: null, hint: null };
    expect(c.sessionLost("dbs", query)).toBe(false);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
    const lost: ErrorPayload = { code: "not_connected", message: "m", detail: null, hint: null };
    expect(c.sessionLost("grp:G", lost)).toBe(false);
    expect(c.sessionLost("yok", lost)).toBe(false);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  });
});
