import { Show, createEffect, createMemo, createSignal } from "solid-js";
import { copyText } from "../../app/clipboard";
import { Button } from "../../ui/Button";
import type { ActiveCell } from "./ResultGrid";
import { cellPanelText } from "./grid-model";
import { tgrid } from "./strings-grid";
import { CellTabs } from "./CellTabs";
import { availability, cellFacts, columnKey, formatMemory, pickFormat, type CellFormat } from "./cell-view-model";
import { tcell } from "./strings-cell";

type ViewersModule = typeof import("./CellViewers");
// Tembel parça tek kez yüklenir; hata olursa önbellek temizlenir, sonraki deneme yeniden dener.
let viewersP: Promise<ViewersModule> | undefined;
let seq = 0;

export function CellPanel(props: { cell: ActiveCell | null; wide: boolean; onToggleWide: () => void }) {
  const [copied, setCopied] = createSignal(false);
  const idBase = `cell-${++seq}`;
  const [viewers, setViewers] = createSignal<ViewersModule["default"] | null>(null);
  const [viewersFailed, setViewersFailed] = createSignal(false);
  const body = createMemo(() => {
    const v = props.cell?.value;
    return v === null || v === undefined ? null : cellPanelText(v, props.cell!.column.typeName);
  });
  // Algılama hücre başına bir kez (JSON.parse dahil); render başına değil.
  const facts = createMemo(() => (props.cell ? cellFacts(props.cell.value, props.cell.column) : null));
  const [memTick, setMemTick] = createSignal(0);
  const format = createMemo<CellFormat>(() => {
    memTick();
    const f = facts();
    return f && props.cell ? pickFormat(f, formatMemory.get(columnKey(props.cell.column))) : "text";
  });
  const avail = createMemo(() => {
    const f = facts();
    return f ? availability(f) : null;
  });
  // Elle seçim: hafızaya yazılır ve aynı hücrede anında geçiş için sinyal tetiklenir.
  const choose = (f: CellFormat) => {
    if (!props.cell) return;
    formatMemory.set(columnKey(props.cell.column), f);
    setMemTick((n) => n + 1);
  };
  createEffect(() => {
    if (format() === "text" || viewers()) return;
    setViewersFailed(false);
    const p = (viewersP ??= import("./CellViewers"));
    p.then((m) => setViewers(() => m.default)).catch(() => {
      if (viewersP === p) viewersP = undefined;
      setViewersFailed(true);
    });
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
    <div class="flex h-full min-h-0 flex-col bg-bg" aria-label={tgrid.cell.title} role="region">
      <div class="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-panel px-3 font-sans text-[12px]">
        <span class="font-semibold">{props.cell ? props.cell.column.name : tgrid.cell.title}</span>
        <Show when={props.cell}>
          {(c) => <span class="text-muted">{c().column.typeDisplay}</span>}
        </Show>
        <Show when={props.cell && props.cell.value !== null}>
          <span class="text-muted">{tgrid.cell.length(props.cell!.value!.length)}</span>
          <Button class="ml-auto h-6 text-[12px]" onClick={copy}>
            {copied() ? tgrid.cell.copied : tgrid.cell.copy}
          </Button>
        </Show>
        <Button
          class={`h-6 text-[12px] ${props.cell && props.cell.value !== null ? "" : "ml-auto"}`}
          aria-pressed={props.wide}
          onClick={props.onToggleWide}
        >
          {props.wide ? tcell.narrow : tcell.widen}
        </Button>
      </div>
      <Show when={props.cell && props.cell.value !== null && avail()}>
        {(a) => <CellTabs value={format()} avail={a()} onSelect={choose} idBase={idBase} />}
      </Show>
      <Show when={format() === "text" && body()?.clipped}>
        <p class="border-b border-border px-3 py-1 font-sans text-[12px] text-danger">{tgrid.cell.clipped}</p>
      </Show>
      <div
        class="min-h-0 flex-1 overflow-auto p-3"
        id={`${idBase}-panel`}
        role={props.cell && props.cell.value !== null ? "tabpanel" : undefined}
        aria-labelledby={props.cell && props.cell.value !== null ? `${idBase}-tab-${format()}` : undefined}
      >
        <Show when={props.cell} fallback={<p class="font-sans text-[13px] text-muted">{tgrid.cell.empty}</p>}>
          <Show
            when={format() !== "text" && props.cell!.value !== null && facts()}
            fallback={
              <Show
                when={body()}
                fallback={
                  <p class="font-sans text-[13px] italic text-muted">
                    {props.cell!.value === null ? tgrid.cell.nullValue : tgrid.cell.emptyString}
                  </p>
                }
              >
                {(b) => (
                  <Show when={b().text !== ""} fallback={<p class="font-sans text-[13px] italic text-muted">{tgrid.cell.emptyString}</p>}>
                    <pre class="whitespace-pre-wrap break-all text-[12px]" style={{ font: '12px "Cascadia Mono", Consolas, monospace' }}>
                      {b().text}
                    </pre>
                  </Show>
                )}
              </Show>
            }
          >
            {(fc) => (
              <Show
                when={!viewersFailed()}
                fallback={<p class="font-sans text-[13px] text-danger">{tcell.loadFailed}</p>}
              >
                <Show when={viewers()} fallback={<p class="font-sans text-[13px] text-muted">{tcell.loading}</p>}>
                  {(V) =>
                    // Getter'lar: görüntüleyici özellikleri kendi izleme bağlamında okur (hücre/biçim değişince güncellenir).
                    V()({
                      get format() {
                        return format() as Exclude<CellFormat, "text">;
                      },
                      get value() {
                        return props.cell!.value!;
                      },
                      get column() {
                        return props.cell!.column;
                      },
                      get facts() {
                        return fc();
                      },
                    })
                  }
                </Show>
              </Show>
            )}
          </Show>
        </Show>
      </div>
    </div>
  );
}
