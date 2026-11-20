// Birden çok tembel parçanın (designer, bakım, yedekleme, şema karşılaştırma) ortak `ddl_*` komutları. Ana paketteki
// commands.ts'ten taşındı (Plan 32 telafisi: nesne arama kancasının baytları); ana paket bu modülü içe aktarmaz.
import type { DdlTarget } from "./bindings/DdlTarget";
import type { ApplyOptions } from "./bindings/ApplyOptions";
import type { ApplyOutcome } from "./bindings/ApplyOutcome";
import type { LookupItem } from "./bindings/LookupItem";
import type { LookupRequest } from "./bindings/LookupRequest";
import type { PlanRequest } from "./bindings/PlanRequest";
import type { PlanView } from "./bindings/PlanView";
import { call } from "./commands";

export const ddlCommands = {
  // Plan 37: ana paketteki commands.ts'ten taşındı (tüm tüketicileri tembel parçalarda).
  ddlTargetOf: (nodeId: string) => call<DdlTarget>("ddl_target_of", { nodeId }),
  ddlLookup: (serverId: string, databaseOid: number | null, request: LookupRequest) =>
    call<LookupItem[]>("ddl_lookup", { serverId, databaseOid, request }),
  ddlPreview: (serverId: string, databaseOid: number | null, request: PlanRequest) =>
    call<PlanView>("ddl_preview", { serverId, databaseOid, request }),
  ddlApply: (serverId: string, databaseOid: number | null, request: PlanRequest, options: ApplyOptions) =>
    call<ApplyOutcome>("ddl_apply", { serverId, databaseOid, request, options }),
  ddlCancel: (applyId: string) => call<null>("ddl_cancel", { applyId }),
};

export type DdlApi = typeof ddlCommands;
