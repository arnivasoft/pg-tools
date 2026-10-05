import { createSignal } from "solid-js";
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

type Request = { name: string; resolve: (ok: boolean) => void };

/** Söz tabanlı onay: `ask(name)` kullanıcı yanıtlayınca çözülür; `View` bir kez bağlanır. */
export function createDeleteConfirm() {
  const [request, setRequest] = createSignal<Request | null>(null);
  const ask = (name: string) =>
    new Promise<boolean>((resolve) => {
      request()?.resolve(false);
      setRequest({ name, resolve });
    });
  const finish = (ok: boolean) => {
    const cur = request();
    if (!cur) return;
    setRequest(null);
    cur.resolve(ok);
  };
  const View = () => (
    <ConfirmDialog
      open={request() !== null}
      title={t.menu.confirmTitle}
      message={t.menu.confirmMessage(request()?.name ?? "")}
      confirmLabel={t.menu.confirmLabel}
      danger
      onConfirm={() => finish(true)}
      onCancel={() => finish(false)}
    />
  );
  return { ask, View };
}
