import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invoke(cmd, args),
  Channel: class {
    onmessage: ((m: unknown) => void) | null = null;
  },
}));

import type { DataChangeSet } from "../../ipc/bindings/DataChangeSet";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { SessionOptions } from "../../ipc/bindings/SessionOptions";
import { IpcError, toPayload } from "../../ipc/errors";
import { createDataApi, dataApply, dataCancel, dataLoad, dataPreview, dataSelect, qtOpenReadonly } from "./data-commands";
import type { DataApi } from "./data-commands";

const obj: ObjectRef = { kind: "table", oid: 42 };
const cs: DataChangeSet = { object: obj, fingerprint: "f", changes: [] };

describe("data-commands", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("Tauri komut ve parametre adları (camelCase) arka uçla birebir", async () => {
    invoke.mockResolvedValue("ok");
    await dataLoad("s", 7, obj);
    await dataSelect("s", 7, obj, "fp", { order: [], filter: null });
    await dataPreview("s", 7, cs);
    await dataApply("s", 7, cs, { applyId: "a", lockTimeoutMs: 5000, expectedHash: "h", confirmed: false });
    await dataCancel("a");
    const opts = {} as SessionOptions;
    expect(await qtOpenReadonly("s", null, opts)).toBe("ok");
    expect(invoke.mock.calls).toEqual([
      ["data_load", { serverId: "s", databaseOid: 7, object: obj }],
      ["data_select", { serverId: "s", databaseOid: 7, object: obj, fingerprint: "fp", request: { order: [], filter: null } }],
      ["data_preview", { serverId: "s", databaseOid: 7, changeSet: cs }],
      ["data_apply", { serverId: "s", databaseOid: 7, changeSet: cs, options: { applyId: "a", lockTimeoutMs: 5000, expectedHash: "h", confirmed: false } }],
      ["data_cancel", { applyId: "a" }],
      ["qt_open_readonly", { serverId: "s", databaseOid: null, options: opts }],
    ]);
  });

  it("hatalar Error olarak döner ve toPayload ile aynı yükü verir (IpcError ana paketten içe aktarılmaz, R1)", async () => {
    invoke.mockRejectedValue({ code: "locked", message: "kilitli" });
    const err = await dataLoad("s", 1, obj).then(() => null, (e: unknown) => e);
    expect(err instanceof Error).toBe(true);
    expect(err instanceof IpcError).toBe(false);
    expect(toPayload(err)).toEqual({ code: "locked", message: "kilitli", detail: null, hint: null });
    invoke.mockRejectedValue("düz metin");
    const err2 = await dataCancel("a").then(() => null, (e: unknown) => e);
    expect(toPayload(err2)).toEqual({ code: "unknown", message: "düz metin", detail: null, hint: null });
  });

  it("DataApi sahte olarak kurulabilir; createDataApi tüm üyeleri bağlar", () => {
    const fake: DataApi = {
      ddlTargetOf: async () => ({ serverId: "s", databaseOid: 1, object: obj, create: null, focus: null, refreshNodeId: null }),
      dataLoad: async () => { throw new Error("yok"); },
      dataSelect: async () => ({ result: "invalid", errors: [] }),
      dataPreview: async () => ({ statements: [], warnings: [], errors: [], hash: "h", risk: "safe", empty: true }),
      dataApply: async () => ({ outcome: "applied", statements: 0, elapsedMs: 0, results: [] }),
      dataCancel: async () => null,
      qtOpenReadonly: async () => "sess",
      qtRun: async () => null,
      qtMore: async () => null,
      qtCancel: async () => null,
      qtClose: async () => null,
      refreshServers: async () => [],
      connect: async () => true,
    };
    expect(Object.keys(fake)).toHaveLength(13);
    const refreshServers = vi.fn(async () => []);
    const connect = vi.fn(async () => true);
    const api = createDataApi(
      { ddlTargetOf: fake.ddlTargetOf, qtRun: fake.qtRun, qtMore: fake.qtMore, qtCancel: fake.qtCancel, qtClose: fake.qtClose },
      refreshServers,
      connect,
    );
    expect(Object.keys(api).sort()).toEqual(Object.keys(fake).sort());
    expect(api.qtRun).toBe(fake.qtRun);
    expect(api.connect).toBe(connect);
    expect(api.dataLoad).toBe(dataLoad);
  });
});
