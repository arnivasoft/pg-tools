// Query Tool sekmesine (QueryTab tembel parçası) özgü komut sarmalayıcıları. Ana paketteki commands.ts'ten taşındı (Plan 32
// telafisi: nesne arama kancasının baytları). Oturum, çalıştırma, transaction ve taslak komutları commands.ts'te kalır
// (sorgu denetleyicisi ana pakettedir). Bu parçadaki explain-controller ve HistoryView de buradan alır.
import type { AppSettings } from "../../ipc/bindings/AppSettings";
import type { CheckDiag } from "../../ipc/bindings/CheckDiag";
import type { CheckTarget } from "../../ipc/bindings/CheckTarget";
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";
import type { CompletionRequest } from "../../ipc/bindings/CompletionRequest";
import type { CsvOptions } from "../../ipc/bindings/CsvOptions";
import type { ExplainOptions } from "../../ipc/bindings/ExplainOptions";
import type { ExplainResult } from "../../ipc/bindings/ExplainResult";
import type { ExportSummary } from "../../ipc/bindings/ExportSummary";
import type { FormatOptions } from "../../ipc/bindings/FormatOptions";
import type { HistoryEntry } from "../../ipc/bindings/HistoryEntry";
import type { Scope } from "../../ipc/bindings/Scope";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import type { TxAction } from "../../ipc/bindings/TxAction";
import type { TxState } from "../../ipc/bindings/TxState";
import { call, channel } from "../../ipc/commands";

export const tabCommands = {
  qtAnalyze: (session: string, sql: string, scope: Scope) => call<StmtPlan[]>("qt_analyze", { session, sql, scope }),
  qtTx: (session: string, action: TxAction) => call<TxState>("qt_tx", { session, action }),
  qtHistoryPushMany: (entries: HistoryEntry[]) => call<number>("qt_history_push_many", { entries }),
  qtDraftsClear: () => call<null>("qt_drafts_clear"),
  // Görsel EXPLAIN: kapsamdaki tek ifadenin planı (JSON metni); kapılar qt_run ile aynı (`confirmation_required`, `locked`).
  qtExplain: (session: string, sql: string, scope: Scope, options: ExplainOptions, confirmed: boolean) =>
    call<ExplainResult>("qt_explain", { session, sql, scope, options, confirmed }),
  qtComplete: (session: string, request: CompletionRequest) => call<CompletionItem[]>("qt_complete", { session, request }),
  qtCheck: (session: string, sql: string, target: CheckTarget) => call<CheckDiag[]>("qt_check", { session, sql, target }),
  qtFormat: (sql: string, options: FormatOptions) => call<string>("qt_format", { sql, options }),
  qtExportCsv: (session: string, sql: string, path: string, options: CsvOptions, onProgress: (rows: number) => void) =>
    call<ExportSummary>("qt_export_csv", { session, sql, path, options, onProgress: channel(onProgress) }),
  qtHistoryList: (query: string | null, limit: number) => call<HistoryEntry[]>("qt_history_list", { query, limit }),
  qtHistoryClear: () => call<null>("qt_history_clear"),
  settingsSetHistory: (enabled: boolean) => call<AppSettings>("settings_set_history", { enabled }),
  settingsSetEditor: (checkEnabled: boolean, format: FormatOptions) =>
    call<AppSettings>("settings_set_editor", { checkEnabled, format }),
};

export type TabApi = typeof tabCommands;
