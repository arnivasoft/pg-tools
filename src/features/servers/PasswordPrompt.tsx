import { Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import type { PasswordAnswer } from "../explorer/controller";

type Request = { node: TreeNode; retry: boolean; resolve: (answer: PasswordAnswer | null) => void };

export function createPasswordPrompt() {
  const [request, setRequest] = createSignal<Request | null>(null);
  const [password, setPassword] = createSignal("");
  const [remember, setRemember] = createSignal(false);

  const ask = (node: TreeNode, retry: boolean) =>
    new Promise<PasswordAnswer | null>((resolve) => {
      setPassword("");
      setRequest({ node, retry, resolve });
    });

  const finish = (answer: PasswordAnswer | null) => {
    const current = request();
    if (!current) return;
    setRequest(null);
    setPassword("");
    current.resolve(answer);
  };

  const View = () => (
    <Dialog
      open={request() !== null}
      onOpenChange={(open) => !open && finish(null)}
      title={t.password.title(request()?.node.label ?? "")}
      width="360px"
      footer={
        <>
          <Button onClick={() => finish(null)}>{t.server.cancel}</Button>
          <Button variant="primary" type="submit" form="pgt-password-form">
            {t.password.connect}
          </Button>
        </>
      }
    >
      <form
        id="pgt-password-form"
        class="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          finish({ password: password(), remember: remember() });
        }}
      >
        <Show when={request()?.retry}>
          <p class="text-danger">{t.password.retry}</p>
        </Show>
        <Field label={t.server.password}>
          <input
            type="password"
            class={inputClass}
            autofocus
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
        </Field>
        <label class="flex items-center gap-2">
          <input type="checkbox" checked={remember()} onChange={(e) => setRemember(e.currentTarget.checked)} />
          {t.server.savePassword}
        </label>
      </form>
    </Dialog>
  );

  return { ask, View };
}
