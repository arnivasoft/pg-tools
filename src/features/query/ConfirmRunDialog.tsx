import { For, Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import type { ConfirmItem } from "./confirm-model";

const MAX_CHARS = 500;
const clip = (s: string) => {
  const chars = Array.from(s);
  return chars.length > MAX_CHARS ? chars.slice(0, MAX_CHARS).join("") + "…" : s;
};

export function ConfirmRunDialog(props: {
  open: boolean;
  items: ConfirmItem[];
  /** Üretim etiketli sunucu: kırmızı şerit (yalnızca biçim). */
  production: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  let cancelEl: HTMLButtonElement | undefined;
  const c = t.query.confirm;
  return (
    <Dialog
      open={props.open}
      onOpenChange={(open) => !open && props.onCancel()}
      title={c.title}
      width="560px"
      // Yanlışlıkla Enter ile onaylanmasın: odak "Vazgeç"te başlar. Escape = iptal (Kobalte).
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => props.onCancel()}>
            {t.server.cancel}
          </Button>
          <Button variant="primary" class="!bg-danger !text-white" onClick={() => props.onConfirm()}>
            {c.run}
          </Button>
        </>
      }
    >
      <div class={props.production ? "border-l-4 border-danger pl-3" : ""} data-production={props.production}>
        <p class="mb-2">{c.intro}</p>
        <ul class="flex flex-col gap-2">
          <For each={props.items}>
            {(it) => (
              <li class="rounded border border-border p-2">
                <div class="mb-1 flex flex-wrap gap-1">
                  <Show when={it.danger}>
                    {(d) => <span class="rounded bg-danger px-1.5 text-xs text-white">{c.danger[d()]}</span>}
                  </Show>
                  <Show when={it.production}>
                    <span class="rounded bg-danger px-1.5 text-xs text-white">{c.production}</span>
                  </Show>
                </div>
                <pre class="max-h-32 overflow-auto whitespace-pre-wrap break-all font-mono text-xs">{clip(it.text)}</pre>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Dialog>
  );
}

type Request = { items: ConfirmItem[]; production: boolean; resolve: (ok: boolean) => void };

/** Söz tabanlı onay: `ask(items, production)`; `View` bir kez bağlanır. */
export function createRunConfirm() {
  const [request, setRequest] = createSignal<Request | null>(null);
  const ask = (items: ConfirmItem[], production = false) =>
    new Promise<boolean>((resolve) => {
      request()?.resolve(false);
      setRequest({ items, production, resolve });
    });
  const finish = (ok: boolean) => {
    const cur = request();
    if (!cur) return;
    setRequest(null);
    cur.resolve(ok);
  };
  const View = () => (
    <ConfirmRunDialog
      open={request() !== null}
      items={request()?.items ?? []}
      production={request()?.production ?? false}
      onConfirm={() => finish(true)}
      onCancel={() => finish(false)}
    />
  );
  return { ask, View };
}
