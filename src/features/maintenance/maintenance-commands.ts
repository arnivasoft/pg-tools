// Bakım komut sarmalayıcıları (tembel parça; Plan 24 arka ucu). Ana paketteki commands.ts'ten taşındı: ana paket
// pencere çerçevesi korumasına yer açar. Ortak `call` (IpcError) commands.ts'ten gelir.
import type { MaintenanceProgress } from "../../ipc/bindings/MaintenanceProgress";
import type { MaintenanceTarget } from "../../ipc/bindings/MaintenanceTarget";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import { call } from "../../ipc/commands";

/** Hedef bilgisi ve uygulama sırasındaki ilerleme (applyId ApplyOptions'takiyle aynı). */
export const maintenanceCommands = {
  ddlLoadMaintenanceTarget: (serverId: string, databaseOid: number, object: ObjectRef) =>
    call<MaintenanceTarget>("ddl_load_maintenance_target", { serverId, databaseOid, object }),
  ddlMaintenanceProgress: (serverId: string, databaseOid: number, applyId: string) =>
    call<MaintenanceProgress | null>("ddl_maintenance_progress", { serverId, databaseOid, applyId }),
};

export type MaintenanceApi = typeof maintenanceCommands;
