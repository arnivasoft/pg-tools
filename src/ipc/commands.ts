import { Channel, invoke } from "@tauri-apps/api/core";
import type { ClosePolicy } from "./bindings/ClosePolicy";
import type { CompletionItem } from "./bindings/CompletionItem";
import type { CompletionRequest } from "./bindings/CompletionRequest";
import type { CsvOptions } from "./bindings/CsvOptions";
import type { ExportSummary } from "./bindings/ExportSummary";
import type { ImportItem } from "./bindings/ImportItem";
import type { PropertySection } from "./bindings/PropertySection";
import type { QueryEvent } from "./bindings/QueryEvent";
import type { Scope } from "./bindings/Scope";
import type { ServerConfig } from "./bindings/ServerConfig";
import type { ServerInfo } from "./bindings/ServerInfo";
import type { SessionOptions } from "./bindings/SessionOptions";
import type { StmtPlan } from "./bindings/StmtPlan";
import type { TreeNode } from "./bindings/TreeNode";
import type { TxAction } from "./bindings/TxAction";
import type { TxState } from "./bindings/TxState";
import { IpcError, toPayload } from "./errors";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new IpcError(toPayload(e));
  }
}

function channel<T>(onMessage: (m: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = onMessage;
  return ch;
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
  previewPgadminImport: (path: string) => call<ImportItem[]>("preview_pgadmin_import", { path }),
  applyPgadminImport: (path: string, indices: number[]) => call<number>("apply_pgadmin_import", { path, indices }),

  // Sorgu Aracı. Hata `code`ları: confirmation_required | locked | busy | tx_failed | open_transaction | session_reset
  // (bkz. bindings/QueryErrorCode), gerisi genel kodlar. Olaylar Channel ile akar; komut tamamlanınca söz çözülür.
  qtOpen: (serverId: string, databaseOid: number | null, options: SessionOptions) =>
    call<string>("qt_open", { serverId, databaseOid, options }),
  qtAnalyze: (session: string, sql: string, scope: Scope) => call<StmtPlan[]>("qt_analyze", { session, sql, scope }),
  qtRun: (session: string, sql: string, scope: Scope, limit: number, confirmed: boolean, onEvent: (ev: QueryEvent) => void) =>
    call<null>("qt_run", { session, sql, scope, limit, confirmed, onEvent: channel(onEvent) }),
  qtMore: (session: string, stmtIndex: number, onEvent: (ev: QueryEvent) => void) =>
    call<null>("qt_more", { session, stmtIndex, onEvent: channel(onEvent) }),
  qtCancel: (session: string) => call<null>("qt_cancel", { session }),
  qtTx: (session: string, action: TxAction) => call<TxState>("qt_tx", { session, action }),
  qtTxState: (session: string) => call<TxState>("qt_tx_state", { session }),
  qtSetOptions: (session: string, options: SessionOptions) => call<null>("qt_set_options", { session, options }),
  qtClose: (session: string, policy: ClosePolicy) => call<null>("qt_close", { session, policy }),
  qtComplete: (session: string, request: CompletionRequest) => call<CompletionItem[]>("qt_complete", { session, request }),
  qtExportCsv: (session: string, sql: string, path: string, options: CsvOptions, onProgress: (rows: number) => void) =>
    call<ExportSummary>("qt_export_csv", { session, sql, path, options, onProgress: channel(onProgress) }),
};

export type Api = typeof commands;
