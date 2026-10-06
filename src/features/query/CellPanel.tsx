import { Show, createMemo, createSignal } from "solid-js";
import { copyText } from "../../app/clipboard";
import { t } from "../../app/strings";
import { Button } from "../../ui/Button";
import type { ActiveCell } from "./ResultGrid";
import { cellPanelText } from "./grid-model";

export function CellPanel(props: { cell: ActiveCell | null }) {
  const [copied, setCopied] = createSignal(false);
  const body = createMemo(() => {
    const v = props.cell?.value;
    return v === null || v === undefined ? null : cellPanelText(v, props.cell!.column.typeName);
  });
  let timer: number | undefined;
  const copy = () => {
    const v = props.cell?.value;
    if (v === null || v === undefined) return;
    void copyText(v)
      .then(() => {
        setCopied(true);
        window.clearTimeout(timer);
        timer = window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };
  return (
    <div class="flex h-full min-h-0 flex-col bg-bg" aria-label={t.query.cell.title} role="region">
      <div class="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-panel px-3 font-sans text-[12px]">
        <span class="font-semibold">{props.cell ? props.cell.column.name : t.query.cell.title}</span>
        <Show when={props.cell}>
          {(c) => <span class="text-muted">{c().column.typeDisplay}</span>}
        </Show>
        <Show when={props.cell && props.cell.value !== null}>
          <span class="text-muted">{t.query.cell.length(props.cell!.value!.length)}</span>
          <Button class="ml-auto h-6 text-[12px]" onClick={copy}>
            {copied() ? t.query.cell.copied : t.query.cell.copy}
          </Button>
        </Show>
      </div>
      <Show when={body()?.clipped}>
        <p class="border-b border-border px-3 py-1 font-sans text-[12px] text-danger">{t.query.cell.clipped}</p>
      </Show>
      <div class="min-h-0 flex-1 overflow-auto p-3">
        <Show when={props.cell} fallback={<p class="font-sans text-[13px] text-muted">{t.query.cell.empty}</p>}>
          <Show
            when={body()}
            fallback={
              <p class="font-sans text-[13px] italic text-muted">
                {props.cell!.value === null ? t.query.cell.nullValue : t.query.cell.emptyString}
              </p>
            }
          >
            {(b) => (
              <Show when={b().text !== ""} fallback={<p class="font-sans text-[13px] italic text-muted">{t.query.cell.emptyString}</p>}>
                <pre class="whitespace-pre-wrap break-all text-[12px]" style={{ font: '12px "Cascadia Mono", Consolas, monospace' }}>
                  {b().text}
                </pre>
              </Show>
            )}
          </Show>
        </Show>
      </div>
    </div>
  );
}
