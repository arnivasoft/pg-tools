// Bakım diyaloğu (tembel parça). GEÇİCİ İSKELET (Plan 25 Task 1): yalnız "Hazırlanıyor" + kapat; gerçek akış Task 3'te.
// Ana modülleri (`app/strings`, `explorer/actions`, `tree-model`) içe aktarmaz; metin `strings-maintenance`tan gelir.
import { tm } from "../../app/strings-maintenance";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";

export function MaintenanceDialog(props: { open: boolean; onClose: () => void }) {
  return (
    <Dialog
      open={props.open}
      onOpenChange={(o) => !o && props.onClose()}
      title={tm.title}
      footer={<Button onClick={props.onClose}>{tm.close}</Button>}
    >
      <p role="status">{tm.preparing}</p>
    </Dialog>
  );
}
