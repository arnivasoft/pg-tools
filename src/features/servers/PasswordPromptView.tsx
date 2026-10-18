// Parola istemi görünümü (Kobalte Dialog çeker): ana pakette statik içe aktarılmaz, `loadPasswordPromptView` ile tembel iner.
import { Show } from "solid-js";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import type { PasswordAnswer } from "../explorer/controller";
import type { PasswordRequest } from "./password-prompt-model";
import { tsrv } from "../../app/strings-server";
import { dialogStrings } from "../../ui/dialog-strings";

export default function PasswordPromptView(props: {
  request: PasswordRequest | null;
  password: string;
  remember: boolean;
  onPassword: (v: string) => void;
  onRemember: (v: boolean) => void;
  finish: (answer: PasswordAnswer | null) => void;
}) {
  let inputEl: HTMLInputElement | undefined;
  return (
    <Dialog
      open={props.request !== null}
      onOpenChange={(open) => !open && props.finish(null)}
      title={tsrv.prompt.title(props.request?.node.label ?? "")}
      width="360px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        inputEl?.focus();
      }}
      footer={
        <>
          <Button onClick={() => props.finish(null)}>{dialogStrings.cancel}</Button>
          <Button variant="primary" type="submit" form="pgt-password-form">
            {tsrv.prompt.connect}
          </Button>
        </>
      }
    >
      <form
        id="pgt-password-form"
        class="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          props.finish({ password: props.password, remember: props.remember });
        }}
      >
        <Show when={props.request?.retry}>
          <p class="text-danger">{props.request?.error?.message ?? tsrv.prompt.retry}</p>
          <Show when={props.request?.error?.detail}>
            {(detail) => <p class="text-muted break-words select-text">{detail()}</p>}
          </Show>
        </Show>
        <Field label={tsrv.password}>
          <input
            type="password"
            class={inputClass}
            ref={inputEl}
            value={props.password}
            onInput={(e) => props.onPassword(e.currentTarget.value)}
          />
        </Field>
        <label class="flex items-center gap-2">
          <input type="checkbox" checked={props.remember} onChange={(e) => props.onRemember(e.currentTarget.checked)} />
          {tsrv.savePassword}
        </label>
      </form>
    </Dialog>
  );
}
