// İskelet: gerçek görüntüleyiciler Task 6'da yazılır. Aynı imza korunur.
import type { Column } from "../../ipc/bindings/Column";
import type { CellFacts, CellFormat } from "./cell-view-model";
import type { CellIpc } from "./cell-ipc";
import { tcell } from "./strings-cell";

export default function CellViewers(props: {
  format: Exclude<CellFormat, "text">;
  value: string;
  column: Column;
  facts: CellFacts;
  ipc?: CellIpc;
}) {
  return (
    <p class="font-sans text-[13px] text-muted">
      {tcell.tabs[props.format]} — {tcell.loading}
    </p>
  );
}
