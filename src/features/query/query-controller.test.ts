import { afterEach, describe, expect, it, vi } from "vitest";
import type { DraftTab } from "../../ipc/bindings/DraftTab";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { Api } from "../../ipc/commands";
import { emptyTree } from "../explorer/tree-model";
import { createQueryController } from "./query-controller";

const server: ServerConfig = {
  id: "S1", name: "srv", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
  ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: null, label: null, readOnlyLock: false,
};
const draft = (id: string, serverId: string | null = "S1"): DraftTab => ({
  id, title: id, serverId, databaseOid: null, database: "postgres", options: { mode: "autocommit", statementTimeoutMs: 0 },
});

function setup(drafts: [DraftTab, string][]) {
  const log: string[] = [];
  let releaseSave: (() => void) | null = null;
  const api = {
    listServers: vi.fn(async () => [server]),
    qtDraftsLoad: vi.fn(async () => drafts),
    qtDraftsSave: vi.fn(
      (tab: DraftTab) =>
        new Promise<null>((resolve) => {
          log.push(`save:${tab.id}:start`);
          releaseSave = () => {
            log.push(`save:${tab.id}:end`);
            resolve(null);
          };
        }),
    ),
    qtDraftsRemove: vi.fn(async (id: string) => {
      log.push(`remove:${id}`);
      return null;
    }),
  } as unknown as Api;
  const ctl = createQueryController({ api, notify: vi.fn(), tree: emptyTree, connect: async () => false, serversChanged: () => {} });
  return { ctl, api, log, release: () => releaseSave?.() };
}

afterEach(() => vi.useRealTimers());

describe("query-controller taslakları", () => {
  it("kayıt sürerken kapanan sekmenin taslağı kayıttan SONRA silinir (geri gelmez)", async () => {
    vi.useFakeTimers();
    const { ctl, log, release } = setup([[draft("a"), "select 1"]]);
    await ctl.restore();
    ctl.setText("a", "select 2");
    await vi.advanceTimersByTimeAsync(5000); // debounce ateşlendi, kayıt sürüyor
    expect(log).toEqual(["save:a:start"]);
    await ctl.requestClose("a"); // oturum yok → doğrudan kapanır
    expect(log).toEqual(["save:a:start"]); // kaldırma, süren kaydı bekler
    release();
    await vi.runAllTimersAsync();
    expect(log[0]).toBe("save:a:start");
    expect(log[1]).toBe("save:a:end");
    expect(log.slice(2).every((x) => x === "remove:a")).toBe(true);
    expect(log.length).toBeGreaterThan(2);
    // kapandıktan sonra metin değişse de kayıt yok
    ctl.setText("a", "x");
    await vi.advanceTimersByTimeAsync(6000);
    expect(log.filter((x) => x.startsWith("save")).length).toBe(2);
  });

  it("sunucusu silinmiş sekmede düzenleme taslağı yazmaz", async () => {
    vi.useFakeTimers();
    const { ctl, api } = setup([[draft("g", "GONE"), "select 1"]]);
    await ctl.restore();
    expect(ctl.model("g")?.serverGone).toBe(true);
    ctl.setText("g", "select 2");
    await vi.advanceTimersByTimeAsync(6000);
    expect(api.qtDraftsSave).not.toHaveBeenCalled();
  });
});

describe("temizleme sonrası", () => {
  it("resaveDrafts açık sekmeleri hemen yeniden yazar, sunucusu olmayanı yazmaz", async () => {
    const { ctl, api } = setup([
      [draft("a"), "select 1"],
      [draft("g", "GONE"), "select 2"],
    ]);
    await ctl.restore();
    void ctl.resaveDrafts();
    await Promise.resolve();
    const saved = (api.qtDraftsSave as unknown as { mock: { calls: [DraftTab, string][] } }).mock.calls.map(([t, sql]) => [t.id, sql]);
    expect(saved).toEqual([["a", "select 1"]]);
  });
});

describe("openScript (designer betiği Query Tool'da)", () => {
  function scriptSetup(cfg: ServerConfig) {
    const api = {
      listServers: vi.fn(async () => [cfg]),
      qtDraftsSave: vi.fn(async () => null),
      qtOpen: vi.fn(),
      qtRun: vi.fn(),
      saveServer: vi.fn(),
    } as unknown as Api;
    const notify = vi.fn();
    const ctl = createQueryController({ api, notify, tree: emptyTree, connect: async () => true, serversChanged: () => {} });
    return { ctl, api, notify };
  }

  it("metni yeni etkin sekmeye koyar ve çalıştırmaz", async () => {
    const { ctl, api } = scriptSetup({ ...server, label: "test" });
    await ctl.openScript({ serverId: "S1", databaseOid: 42, database: "app", text: "ALTER SEQUENCE s RESTART;" });
    expect(ctl.tabs.state.tabs).toHaveLength(1);
    const tab = ctl.tabs.state.tabs[0];
    expect(ctl.tabs.state.activeId).toBe(tab.id);
    expect(tab.database).toBe("app");
    expect(ctl.text(tab.id)).toBe("ALTER SEQUENCE s RESTART;");
    expect(ctl.model(tab.id)?.databaseOid).toBe(42);
    expect(ctl.model(tab.id)?.run.status).toBe("noSession"); // oturum açılmadı, çalıştırılmadı
    expect(api.qtOpen).not.toHaveBeenCalled();
    expect(api.qtRun).not.toHaveBeenCalled();
  });

  it("kilitli sunucuda da açar (çalıştırma kapısı ayrı)", async () => {
    const { ctl, api } = scriptSetup({ ...server, label: "production", readOnlyLock: true });
    await ctl.openScript({ serverId: "S1", databaseOid: 7, database: "app", text: "DROP INDEX i;" });
    expect(ctl.tabs.state.tabs).toHaveLength(1);
    expect(ctl.text(ctl.tabs.state.tabs[0].id)).toBe("DROP INDEX i;");
    expect(api.qtOpen).not.toHaveBeenCalled();
  });

  it("etiketsiz sunucuda ortam istemini kullanır; iptalde sekme açılmaz", async () => {
    const { ctl } = scriptSetup({ ...server, label: null });
    const p = ctl.openScript({ serverId: "S1", databaseOid: 1, database: "app", text: "x" });
    await vi.waitFor(() => expect(ctl.envGate.request()).not.toBeNull());
    ctl.envGate.answer(null);
    await p;
    expect(ctl.tabs.state.tabs).toHaveLength(0);

    const q = ctl.openScript({ serverId: "S1", databaseOid: 1, database: "app", text: "y" });
    await vi.waitFor(() => expect(ctl.envGate.request()).not.toBeNull());
    ctl.envGate.answer("skip");
    await q;
    expect(ctl.tabs.state.tabs).toHaveLength(1);
    expect(ctl.text(ctl.tabs.state.tabs[0].id)).toBe("y");
  });

  it("sunucu kaydı yoksa sekme açmaz, bildirir", async () => {
    const { ctl, notify } = scriptSetup(server);
    await ctl.openScript({ serverId: "NOPE", databaseOid: 1, text: "x" });
    expect(ctl.tabs.state.tabs).toHaveLength(0);
    expect(notify).toHaveBeenCalledWith(expect.any(String), "error");
  });
});
