// Onay diyaloğunun görünümü (Kobalte Dialog'u çeker): ana pakette statik içe aktarılmaz, `loadConfirmDialogView` ile tembel iner.
import { t } from "../app/strings";
import { Button } from "./Button";
import { Dialog } from "./Dialog";

export function ConfirmDialog(props: {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  let cancelEl: HTMLButtonElement | undefined;
  return (
    <Dialog
      open={props.open}
      onOpenChange={(open) => !open && props.onCancel()}
      title={props.title}
      width="380px"
      // Yıkıcı eylem yanlışlıkla Enter ile onaylanmasın: odak "Vazgeç"te başlar.
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => props.onCancel()}>
            {t.server.cancel}
          </Button>
          <Button
            variant="primary"
            class={props.danger ? "!bg-danger !text-white" : ""}
            onClick={() => props.onConfirm()}
          >
            {props.confirmLabel}
          </Button>
        </>
      }
    >
      <p>{props.message}</p>
    </Dialog>
  );
}

export default ConfirmDialog;
