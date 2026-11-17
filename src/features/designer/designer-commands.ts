// Nesne diyaloğuna (designer tembel parçası) özgü komut sarmalayıcıları. Ana paketteki commands.ts'ten taşındı (Plan 32
// telafisi: nesne arama kancasının baytları). Ortak `ddl_*` komutları (hedef, lookup, önizleme, uygulama) commands.ts'te kalır.
import type { AclDef } from "../../ipc/bindings/AclDef";
import type { CreateContext } from "../../ipc/bindings/CreateContext";
import type { CreatePreset } from "../../ipc/bindings/CreatePreset";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { DefaultAclScope } from "../../ipc/bindings/DefaultAclScope";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import { call } from "../../ipc/commands";

export const designerCommands = {
  ddlDefaults: (serverId: string, databaseOid: number | null, kind: ObjectKind, ctx: CreateContext, preset: CreatePreset | null = null) =>
    call<ObjectDef>("ddl_defaults", { serverId, databaseOid, kind, ctx, preset }),
  ddlLoad: (serverId: string, databaseOid: number | null, object: ObjectRef) => call<ObjectDef>("ddl_load", { serverId, databaseOid, object }),
  ddlLoadAcl: (serverId: string, databaseOid: number | null, object: ObjectRef) => call<AclDef>("ddl_load_acl", { serverId, databaseOid, object }),
  ddlLoadDefaultAcl: (serverId: string, databaseOid: number, scope: DefaultAclScope) =>
    call<DefaultAclDef>("ddl_load_default_acl", { serverId, databaseOid, scope }),
};

export type DesignerApi = typeof designerCommands;
