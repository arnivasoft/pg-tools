import { Channel, invoke } from "@tauri-apps/api/core";
import type { CheckDiag } from "./bindings/CheckDiag";
import type { CheckTarget } from "./bindings/CheckTarget";
import type { ClosePolicy } from "./bindings/ClosePolicy";
import type { CompletionItem } from "./bindings/CompletionItem";
import type { CompletionRequest } from "./bindings/CompletionRequest";
import type { CsvOptions } from "./bindings/CsvOptions";
import type { AppSettings } from "./bindings/AppSettings";
import type { DraftTab } from "./bindings/DraftTab";
import type { ExportSummary } from "./bindings/ExportSummary";
import type { FormatOptions } from "./bindings/FormatOptions";
import type { HistoryEntry } from "./bindings/HistoryEntry";
import type { ImportItem } from "./bindings/ImportItem";
import type { PropertySection } from "./bindings/PropertySection";
import type { QueryTarget } from "./bindings/QueryTarget";
import type { QueryEvent } from "./bindings/QueryEvent";
import type { Scope } from "./bindings/Scope";
import type { ServerConfig } from "./bindings/ServerConfig";
import type { ServerInfo } from "./bindings/ServerInfo";
import type { SessionOptions } from "./bindings/SessionOptions";
import type { StmtPlan } from "./bindings/StmtPlan";
import type { TreeNode } from "./bindings/TreeNode";
import type { TxAction } from "./bindings/TxAction";
import type { TxState } from "./bindings/TxState";
import type { ActivityReport } from "./bindings/ActivityReport";
import type { DashInfo } from "./bindings/DashInfo";
import type { LockReport } from "./bindings/LockReport";
import type { Overview } from "./bindings/Overview";
import type { PreparedXact } from "./bindings/PreparedXact";
import type { Sample } from "./bindings/Sample";
import type { Setting } from "./bindings/Setting";
import type { SignalKind } from "./bindings/SignalKind";
import type { SignalOutcome } from "./bindings/SignalOutcome";
import type { Statements } from "./bindings/Statements";
import type { StatementsOrder } from "./bindings/StatementsOrder";
import type { ApplyOptions } from "./bindings/ApplyOptions";
import type { ApplyOutcome } from "./bindings/ApplyOutcome";
import type { CreateContext } from "./bindings/CreateContext";
import type { DdlTarget } from "./bindings/DdlTarget";
import type { LookupItem } from "./bindings/LookupItem";
import type { LookupRequest } from "./bindings/LookupRequest";
import type { ObjectDef } from "./bindings/ObjectDef";
import type { ObjectKind } from "./bindings/ObjectKind";
import type { ObjectRef } from "./bindings/ObjectRef";
import type { PlanRequest } from "./bindings/PlanRequest";
import type { PlanView } from "./bindings/PlanView";
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
  /** İlk çizimden sonra: gizli ana pencereyi tema arka planıyla gösterir (bkz. src-tauri/src/window.rs). */
  appReady: (dark: boolean) => call<null>("app_ready", { dark }),
  deleteServer: (serverId: string) => call<null>("delete_server", { serverId }),
  setShowSystem: (value: boolean) => call<null>("set_show_system", { value }),
  quoteIdentifiers: (serverId: string, names: string[]) => call<string[]>("quote_identifiers", { serverId, names }),
  previewPgadminImport: (path: string) => call<ImportItem[]>("preview_pgadmin_import", { path }),
  applyPgadminImport: (path: string, indices: number[]) => call<number>("apply_pgadmin_import", { path, indices }),

  // Sorgu Aracı. Hata `code`ları: confirmation_required | locked | busy | tx_failed | open_transaction | session_reset
  // (bkz. bindings/QueryErrorCode), gerisi genel kodlar. Olaylar Channel ile akar; komut tamamlanınca söz çözülür.
  // "Query Tool'da aç": düğümün sunucusu ve veritabanı OID'i (düğüm kimliği arayüzde ayrıştırılmaz).
  qtTargetOf: (nodeId: string) => call<QueryTarget>("qt_target_of", { nodeId }),
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
  qtCheck: (session: string, sql: string, target: CheckTarget) => call<CheckDiag[]>("qt_check", { session, sql, target }),
  qtFormat: (sql: string, options: FormatOptions) => call<string>("qt_format", { sql, options }),
  qtExportCsv: (session: string, sql: string, path: string, options: CsvOptions, onProgress: (rows: number) => void) =>
    call<ExportSummary>("qt_export_csv", { session, sql, path, options, onProgress: channel(onProgress) }),

  // Kalıcılık: geçmiş (≤500; bir çalıştırmanın girdileri tek çağrıda, tek dosya yazımıyla; arka uç hassas SQL'i
  // ve kapalı geçmişi yazmaz, eklenen sayısını döner), taslaklar, ayarlar.
  qtHistoryPushMany: (entries: HistoryEntry[]) => call<number>("qt_history_push_many", { entries }),
  qtHistoryList: (query: string | null, limit: number) => call<HistoryEntry[]>("qt_history_list", { query, limit }),
  qtHistoryClear: () => call<null>("qt_history_clear"),
  qtDraftsSave: (tab: DraftTab, sql: string) => call<null>("qt_drafts_save", { tab, sql }),
  qtDraftsRemove: (id: string) => call<null>("qt_drafts_remove", { id }),
  qtDraftsLoad: () => call<[DraftTab, string][]>("qt_drafts_load"),
  qtDraftsClear: () => call<null>("qt_drafts_clear"),
  settingsGet: () => call<AppSettings>("settings_get"),
  settingsSetHistory: (enabled: boolean) => call<AppSettings>("settings_set_history", { enabled }),
  settingsSetEditor: (checkEnabled: boolean, format: FormatOptions) =>
    call<AppSettings>("settings_set_editor", { checkEnabled, format }),

  // Dashboard (pg_stat_* izleme). Hata `code`ları: `42501` yetki, `invalid` kendi oturumu, `not_found` kapanmış id.
  dashOpen: (serverId: string, databaseOid: number | null) => call<DashInfo>("dash_open", { serverId, databaseOid }),
  dashClose: (id: string) => call<null>("dash_close", { id }),
  dashSample: (id: string) => call<Sample>("dash_sample", { id }),
  dashOverview: (id: string) => call<Overview>("dash_overview", { id }),
  dashActivity: (id: string) => call<ActivityReport>("dash_activity", { id }),
  dashLocks: (id: string) => call<LockReport>("dash_locks", { id }),
  dashSettings: (id: string) => call<Setting[]>("dash_settings", { id }),
  dashPrepared: (id: string) => call<PreparedXact[]>("dash_prepared", { id }),
  dashStatements: (id: string, order: StatementsOrder) => call<Statements>("dash_statements", { id, order }),
  dashSignal: (id: string, pid: number, kind: SignalKind) => call<SignalOutcome>("dash_signal", { id, pid, kind }),

  // Nesne diyalogları (ddl_*): Tauri parametre adları src-tauri/src/ddl.rs ile birebir (camelCase).
  ddlTargetOf: (nodeId: string) => call<DdlTarget>("ddl_target_of", { nodeId }),
  ddlDefaults: (serverId: string, databaseOid: number | null, kind: ObjectKind, ctx: CreateContext) =>
    call<ObjectDef>("ddl_defaults", { serverId, databaseOid, kind, ctx }),
  ddlLoad: (serverId: string, databaseOid: number | null, object: ObjectRef) => call<ObjectDef>("ddl_load", { serverId, databaseOid, object }),
  ddlLookup: (serverId: string, databaseOid: number | null, request: LookupRequest) =>
    call<LookupItem[]>("ddl_lookup", { serverId, databaseOid, request }),
  ddlPreview: (serverId: string, databaseOid: number | null, request: PlanRequest) =>
    call<PlanView>("ddl_preview", { serverId, databaseOid, request }),
  ddlApply: (serverId: string, databaseOid: number | null, request: PlanRequest, options: ApplyOptions) =>
    call<ApplyOutcome>("ddl_apply", { serverId, databaseOid, request, options }),
  ddlCancel: (applyId: string) => call<null>("ddl_cancel", { applyId }),
};

export type Api = typeof commands;
