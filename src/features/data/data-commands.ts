// Veri düzenleyici komut sarmalayıcıları (tembel parça). commands.ts'e eklenmez: ana paket büyümez (Plan 17
// Karar 1). Tauri parametre adları src-tauri/src/data.rs ve query.rs (`qt_open_readonly`) ile birebir (camelCase).
// Hata `code`ları: `locked`, `confirmation_required`, `plan_changed`, `invalid_def`, bağlantı ve PG kodları
// (bkz. data-errors.ts). `row_conflict` komut hatası değil, `DataOutcome::Failed`'dır.
import type { DataApplyOptions } from "../../ipc/bindings/DataApplyOptions";
import type { DataChangeSet } from "../../ipc/bindings/DataChangeSet";
import type { DataOutcome } from "../../ipc/bindings/DataOutcome";
import type { DataSelectResult } from "../../ipc/bindings/DataSelectResult";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { SelectRequest } from "../../ipc/bindings/SelectRequest";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SessionOptions } from "../../ipc/bindings/SessionOptions";
import { call } from "../../ipc/commands";
import type { Api } from "../../ipc/commands";

export const dataLoad = (serverId: string, databaseOid: number, object: ObjectRef) =>
  call<DataSource>("data_load", { serverId, databaseOid, object });
export const dataSelect = (serverId: string, databaseOid: number, object: ObjectRef, fingerprint: string, request: SelectRequest) =>
  call<DataSelectResult>("data_select", { serverId, databaseOid, object, fingerprint, request });
export const dataPreview = (serverId: string, databaseOid: number, changeSet: DataChangeSet) =>
  call<PlanView>("data_preview", { serverId, databaseOid, changeSet });
export const dataApply = (serverId: string, databaseOid: number, changeSet: DataChangeSet, options: DataApplyOptions) =>
  call<DataOutcome>("data_apply", { serverId, databaseOid, changeSet, options });
export const dataCancel = (applyId: string) => call<null>("data_cancel", { applyId });
/** Sekmeye özel, sunucu kilidinden bağımsız salt okunur okuma oturumu; oturum kimliğini döner. */
export const qtOpenReadonly = (serverId: string, databaseOid: number | null, options: SessionOptions) =>
  call<string>("qt_open_readonly", { serverId, databaseOid, options });

/** commands.ts'ten devralınan üyeler (imzalar aynen). */
export type DataBaseApi = Pick<Api, "ddlTargetOf" | "qtRun" | "qtMore" | "qtCancel" | "qtClose">;

/** Denetleyicinin tüm dış bağımlılığı (testlerde sahtelenir). */
export interface DataApi extends DataBaseApi {
  dataLoad: typeof dataLoad;
  dataSelect: typeof dataSelect;
  dataPreview: typeof dataPreview;
  dataApply: typeof dataApply;
  dataCancel: typeof dataCancel;
  qtOpenReadonly: typeof qtOpenReadonly;
  refreshServers: () => Promise<ServerConfig[]>;
  connect: (serverId: string) => Promise<boolean>;
}

export function createDataApi(
  api: DataBaseApi,
  refreshServers: DataApi["refreshServers"],
  connect: DataApi["connect"],
): DataApi {
  return {
    ddlTargetOf: api.ddlTargetOf,
    qtRun: api.qtRun,
    qtMore: api.qtMore,
    qtCancel: api.qtCancel,
    qtClose: api.qtClose,
    dataLoad,
    dataSelect,
    dataPreview,
    dataApply,
    dataCancel,
    qtOpenReadonly,
    refreshServers,
    connect,
  };
}
