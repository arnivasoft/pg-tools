import { For, Show, createSignal, onCleanup } from "solid-js";
import { t } from "../../app/strings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { createConfirmGate, type ConfirmItem } from "./confirm-model";

const MAX_CHARS = 500;
const MAX_ITEMS = 50;
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
      <div data-production={props.production}>
        <Show when={props.production}>
          <div class="mb-3 rounded bg-danger/10 px-3 py-2 text-sm font-medium text-danger">{c.productionBanner}</div>
        </Show>
        <p class="mb-2">{c.intro}</p>
        <ul class="flex flex-col gap-2">
          <For each={props.items.slice(0, MAX_ITEMS)}>
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
        <Show when={props.items.length > MAX_ITEMS}>
          <p class="mt-2 text-xs text-muted">{c.more(props.items.length - MAX_ITEMS)}</p>
        </Show>
      </div>
    </Dialog>
  );
}

type Request = { items: ConfirmItem[]; production: boolean };

/** Söz tabanlı onay: `ask(items, production)`; `View` bir kez bağlanır, kaldırılırsa bekleyen istek iptal sayılır. */
export function createRunConfirm() {
  const [request, setRequest] = createSignal<Request | null>(null);
  const gate = createConfirmGate<Request>(setRequest);
  const ask = (items: ConfirmItem[], production = false) => gate.ask({ items, production });
  const View = () => {
    onCleanup(gate.dispose);
    return (
      <ConfirmRunDialog
        open={request() !== null}
        items={request()?.items ?? []}
        production={request()?.production ?? false}
        onConfirm={() => gate.finish(true)}
        onCancel={() => gate.finish(false)}
      />
    );
  };
  return { ask, View };
}
