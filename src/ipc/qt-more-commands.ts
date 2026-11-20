// "Daha fazla satır" ve "iptal" komutları: yalnız tembel parçalar (QueryTab, veri düzenleyicisi) kullanır. Ana paketteki
// commands.ts'ten taşındı (Plan 35 telafisi: Policy/Tablespace menü kancasının baytları; `qtMore`, `qtCancel`); ana paket bu modülü içe aktarmaz.
import type { QueryEvent } from "./bindings/QueryEvent";
import { call, channel } from "./commands";

export const qtMore = (session: string, stmtIndex: number, onEvent: (ev: QueryEvent) => void) =>
  call<null>("qt_more", { session, stmtIndex, onEvent: channel(onEvent) });
export const qtCancel = (session: string) => call<null>("qt_cancel", { session });
