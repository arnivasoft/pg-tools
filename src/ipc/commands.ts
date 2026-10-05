import { invoke } from "@tauri-apps/api/core";
import type { PropertySection } from "./bindings/PropertySection";
import type { ServerConfig } from "./bindings/ServerConfig";
import type { ServerInfo } from "./bindings/ServerInfo";
import type { TreeNode } from "./bindings/TreeNode";
import { IpcError, toPayload } from "./errors";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new IpcError(toPayload(e));
  }
}

export const commands = {
  listServers: () => call<ServerConfig[]>("list_servers"),
  saveServer: (config: ServerConfig, password: string | null) => call<ServerConfig>("save_server", { config, password }),
  testConnection: (config: ServerConfig, password: string | null) => call<ServerInfo>("test_connection", { config, password }),
  connectServer: (serverId: string, password: string | null, remember: boolean) =>
    call<ServerInfo>("connect_server", { serverId, password, remember }),
  disconnectServer: (serverId: string) => call<null>("disconnect_server", { serverId }),
  listChildren: (id: string) => call<TreeNode[]>("list_children", { id }),
  getProperties: (id: string) => call<PropertySection[]>("get_properties", { id }),
  getDdl: (id: string) => call<string>("get_ddl", { id }),
  startupNotice: () => call<string | null>("startup_notice"),
  deleteServer: (serverId: string) => call<null>("delete_server", { serverId }),
  setShowSystem: (value: boolean) => call<null>("set_show_system", { value }),
  quoteIdentifiers: (serverId: string, names: string[]) => call<string[]>("quote_identifiers", { serverId, names }),
};

export type Api = typeof commands;
