// Dashboard komut sarmalayıcıları (tembel parça). Ana paketteki commands.ts'ten taşındı: ana paket durum çubuğundaki
// sürüme yer açar. Hata `code`ları: `42501` yetki, `invalid` kendi oturumu, `not_found` kapanmış id.
import type { ActivityReport } from "../../ipc/bindings/ActivityReport";
import type { DashInfo } from "../../ipc/bindings/DashInfo";
import type { LockReport } from "../../ipc/bindings/LockReport";
import type { Overview } from "../../ipc/bindings/Overview";
import type { PreparedXact } from "../../ipc/bindings/PreparedXact";
import type { Sample } from "../../ipc/bindings/Sample";
import type { Setting } from "../../ipc/bindings/Setting";
import type { SignalKind } from "../../ipc/bindings/SignalKind";
import type { SignalOutcome } from "../../ipc/bindings/SignalOutcome";
import type { Statements } from "../../ipc/bindings/Statements";
import type { StatementsOrder } from "../../ipc/bindings/StatementsOrder";
import { call } from "../../ipc/commands";

export const dashCommands = {
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
};

export type DashApi = typeof dashCommands;
