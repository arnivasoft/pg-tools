// "Daha fazla satır" ve "iptal" komutları: yalnız tembel parçalar (QueryTab, veri düzenleyicisi) kullanır. Ana paketteki
// commands.ts'ten taşındı (Plan 35 telafisi: Policy/Tablespace menü kancasının baytları; `qtMore`, `qtCancel`; Plan 37: `qtRun`); ana paket bu modülü içe aktarmaz.
import type { QueryEvent } from "./bindings/QueryEvent";
import type { Scope } from "./bindings/Scope";
import { call, channel } from "./commands";

export const qtRun = (session: string, sql: string, scope: Scope, limit: number, confirmed: boolean, onEvent: (ev: QueryEvent) => void) =>
  call<null>("qt_run", { session, sql, scope, limit, confirmed, onEvent: channel(onEvent) });
export const qtMore = (session: string, stmtIndex: number, onEvent: (ev: QueryEvent) => void) =>
  call<null>("qt_more", { session, stmtIndex, onEvent: channel(onEvent) });
export const qtCancel = (session: string) => call<null>("qt_cancel", { session });
