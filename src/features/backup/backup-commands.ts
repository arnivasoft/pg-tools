// Yedekleme / geri yükleme komut sarmalayıcıları (tembel parça). Ana paketteki commands.ts'ten taşındı: ana paket
// özel başlık çubuğuna yer açar. Ortak `call` (IpcError) ve `channel` commands.ts'ten gelir; parametre adları spec §5.
import type { AppSettings } from "../../ipc/bindings/AppSettings";
import type { BackupEvent } from "../../ipc/bindings/BackupEvent";
import type { BackupRequest } from "../../ipc/bindings/BackupRequest";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import type { RestoreRequest } from "../../ipc/bindings/RestoreRequest";
import type { RestoreSource } from "../../ipc/bindings/RestoreSource";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import { call, channel } from "../../ipc/commands";

/** `jobId` ön yüzde üretilir (crypto.randomUUID()). */
export const backupCommands = {
  backupTools: (serverId: string) => call<ToolDetection>("backup_tools", { serverId }),
  backupPlan: (serverId: string, databaseOid: number, request: BackupRequest) =>
    call<JobPlan>("backup_plan", { serverId, databaseOid, request }),
  backupStart: (serverId: string, databaseOid: number, request: BackupRequest, jobId: string, onEvent: (e: BackupEvent) => void) =>
    call<JobSummary>("backup_start", { serverId, databaseOid, request, jobId, onEvent: channel(onEvent) }),
  backupCancel: (jobId: string) => call<null>("backup_cancel", { jobId }),
  /** Dosyayı Gezgin'de seçili gösterir (mutlak ve var olan yol; yalnız Windows). */
  backupReveal: (path: string) => call<null>("backup_reveal", { path }),
  restoreInspect: (path: string) => call<RestoreSource>("restore_inspect", { path }),
  restorePlan: (serverId: string, databaseOid: number | null, request: RestoreRequest) =>
    call<JobPlan>("restore_plan", { serverId, databaseOid, request }),
  restoreStart: (
    serverId: string,
    databaseOid: number | null,
    request: RestoreRequest,
    jobId: string,
    confirmed: boolean,
    confirmedProduction: boolean,
    onEvent: (e: BackupEvent) => void,
  ) =>
    call<JobSummary>("restore_start", {
      serverId, databaseOid, request, jobId, confirmed, confirmedProduction, onEvent: channel(onEvent),
    }),
  settingsSetPgBinDir: (dir: string | null) => call<AppSettings>("settings_set_pg_bin_dir", { dir }),
};

export type BackupApi = typeof backupCommands;
