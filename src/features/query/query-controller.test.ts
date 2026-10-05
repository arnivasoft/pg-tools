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
