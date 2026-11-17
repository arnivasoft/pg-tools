import { Channel, invoke } from "@tauri-apps/api/core";
import type { ClosePolicy } from "./bindings/ClosePolicy";
import type { AppSettings } from "./bindings/AppSettings";
import type { DraftTab } from "./bindings/DraftTab";
import type { PropertySection } from "./bindings/PropertySection";
import type { QueryTarget } from "./bindings/QueryTarget";
import type { QueryEvent } from "./bindings/QueryEvent";
import type { Scope } from "./bindings/Scope";
import type { ServerConfig } from "./bindings/ServerConfig";
import type { ServerInfo } from "./bindings/ServerInfo";
import type { SessionOptions } from "./bindings/SessionOptions";
import type { TreeNode } from "./bindings/TreeNode";
import type { TxState } from "./bindings/TxState";
import type { DdlTarget } from "./bindings/DdlTarget";
import { IpcError, toPayload } from "./errors";

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new IpcError(toPayload(e));
  }
}

export function channel<T>(onMessage: (m: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = onMessage;
  return ch;
}

export const commands = {
  listServers: () => call<ServerConfig[]>("list_servers"),
  saveServer: (config: ServerConfig, password: string | null) => call<ServerConfig>("save_server", { config, password }),
  connectServer: (serverId: string, password: string | null, remember: boolean) =>
    call<ServerInfo>("connect_server", { serverId, password, remember }),
  disconnectServer: (serverId: string) => call<null>("disconnect_server", { serverId }),
  listChildren: (id: string) => call<TreeNode[]>("list_children", { id }),
  getProperties: (id: string) => call<PropertySection[]>("get_properties", { id }),
  getDdl: (id: string) => call<string>("get_ddl", { id }),
  startupNotice: () => call<string | null>("startup_notice"),
  /** İlk çizimden sonra: gizli ana pencereyi tema arka planıyla gösterir (bkz. src-tauri/src/window.rs). */
  /** `frame`: pencere düğmeleri yüklenemedi; Windows çerçevesi geri açılır. */
  appReady: (dark: boolean, frame: boolean) => call<null>("app_ready", { dark, frame }),
  /** Özel başlık çubuğu: pencere işlemi; dönüş büyütülmüş mü (bkz. src-tauri/src/window.rs). */
  win: (action: "drag" | "min" | "max" | "close") => call<boolean>("win", { action }),
  deleteServer: (serverId: string) => call<null>("delete_server", { serverId }),
  setShowSystem: (value: boolean) => call<null>("set_show_system", { value }),
  quoteIdentifiers: (serverId: string, names: string[]) => call<string[]>("quote_identifiers", { serverId, names }),

  // Sorgu Aracı. Hata `code`ları: confirmation_required | locked | busy | tx_failed | open_transaction | session_reset
  // (bkz. bindings/QueryErrorCode), gerisi genel kodlar. Olaylar Channel ile akar; komut tamamlanınca söz çözülür.
  // "Query Tool'da aç": düğümün sunucusu ve veritabanı OID'i (düğüm kimliği arayüzde ayrıştırılmaz).
  qtTargetOf: (nodeId: string) => call<QueryTarget>("qt_target_of", { nodeId }),
  qtOpen: (serverId: string, databaseOid: number | null, options: SessionOptions) =>
    call<string>("qt_open", { serverId, databaseOid, options }),
  qtRun: (session: string, sql: string, scope: Scope, limit: number, confirmed: boolean, onEvent: (ev: QueryEvent) => void) =>
    call<null>("qt_run", { session, sql, scope, limit, confirmed, onEvent: channel(onEvent) }),
  qtMore: (session: string, stmtIndex: number, onEvent: (ev: QueryEvent) => void) =>
    call<null>("qt_more", { session, stmtIndex, onEvent: channel(onEvent) }),
  qtCancel: (session: string) => call<null>("qt_cancel", { session }),
  qtTxState: (session: string) => call<TxState>("qt_tx_state", { session }),
  qtSetOptions: (session: string, options: SessionOptions) => call<null>("qt_set_options", { session, options }),
  qtClose: (session: string, policy: ClosePolicy) => call<null>("qt_close", { session, policy }),

  // Kalıcılık: geçmiş (≤500; bir çalıştırmanın girdileri tek çağrıda, tek dosya yazımıyla; arka uç hassas SQL'i
  // ve kapalı geçmişi yazmaz, eklenen sayısını döner), taslaklar, ayarlar.
  qtDraftsSave: (tab: DraftTab, sql: string) => call<null>("qt_drafts_save", { tab, sql }),
  qtDraftsRemove: (id: string) => call<null>("qt_drafts_remove", { id }),
  qtDraftsLoad: () => call<[DraftTab, string][]>("qt_drafts_load"),
  settingsGet: () => call<AppSettings>("settings_get"),

  // Nesne diyalogları (ddl_*): Tauri parametre adları src-tauri/src/ddl.rs ile birebir (camelCase). Yalnız tembel
  // parçaların kullandığı ddl_* komutları features/*/…-commands.ts ve ddl-commands.ts'tedir (Plan 32 telafisi).
  ddlTargetOf: (nodeId: string) => call<DdlTarget>("ddl_target_of", { nodeId }),
};

export type Api = typeof commands;
