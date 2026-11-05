// Şema karşılaştırma açıcısı ve kurulumu (tembel parça; DiffTabs ile birlikte iner — bkz. diff-open.ts). Task 3'te gerçeği
// yazılır; şimdilik yalnızca sekmeyi açar.
import { tsd } from "../../app/strings-diff";
import type { DiffTabCtl } from "./diff-controller";
import type { DiffDeps } from "./diff-open";

export function startDiff(deps: DiffDeps): { ctl: DiffTabCtl; open: (nodeId: string) => Promise<void> } {
  const ctl: DiffTabCtl = {
    model: () => null,
    ids: () => [],
    close: () => {},
  };
  const open = async (_nodeId: string): Promise<void> => {
    deps.tabs.openSchemaDiff({ title: tsd.title.initial });
  };
  return { ctl, open };
}
