// Tek sütunlu birleşik satır farkı: hedef no | kaynak no | işaret | metin. Satırlar sanallaştırılır; yatay kaydırma kabın
// içindedir (genişlik en uzun satırdan `ch` ile hesaplanır, mutlak konumlu satırlar içerik genişliği vermez). Renk tek
// başına bilgi değildir: işaret sütunu `+`/`−` taşır. `lines === null` (çok büyük fark) iki sütunlu düz metin gösterir.
import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Match, Show, Switch, createMemo } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { DiffDetail } from "../../ipc/bindings/DiffDetail";
import type { DiffLine } from "../../ipc/bindings/DiffLine";
import { gutter } from "./diff-model";
import { LINE_HEIGHT, lineClass } from "./diff-view-model";

function Lines(props: { lines: DiffLine[] }) {
  let scrollEl!: HTMLDivElement;
  const v = createVirtualizer({
    get count() {
      return props.lines.length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => LINE_HEIGHT,
    overscan: 20,
  });
  const dims = createMemo(() => {
    let maxNo = 0;
    let maxLen = 0;
    for (const l of props.lines) {
      maxNo = Math.max(maxNo, l.sourceNo ?? 0, l.targetNo ?? 0);
      maxLen = Math.max(maxLen, l.text.length);
    }
    const no = Math.max(3, String(maxNo).length) + 1;
    return { no, width: `calc(${no * 2 + 2 + maxLen}ch + 1rem)` };
  });
  return (
    <div
      ref={scrollEl}
      role="region"
      aria-label={tsd.aria.lines}
      tabIndex={0}
      class="relative min-h-0 flex-1 overflow-auto font-mono text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent"
    >
      <div style={{ height: `${v.getTotalSize()}px`, width: dims().width, "min-width": "100%", position: "relative" }}>
        <For each={v.getVirtualItems()}>
          {(row) => (
            <Show when={props.lines[row.index]}>
              {(l) => {
                const g = () => gutter(l());
                return (
                  <div
                    class={`absolute left-0 right-0 flex whitespace-pre ${lineClass(l().kind)}`}
                    style={{ top: `${row.start}px`, height: `${LINE_HEIGHT}px`, "line-height": `${LINE_HEIGHT}px` }}
                  >
                    <span class="shrink-0 select-none pr-1 text-right text-muted" style={{ width: `${dims().no}ch` }}>
                      {g().target}
                    </span>
                    <span class="shrink-0 select-none pr-1 text-right text-muted" style={{ width: `${dims().no}ch` }}>
                      {g().source}
                    </span>
                    <span
                      class="w-[2ch] shrink-0 select-none text-center"
                      classList={{ "text-danger": l().kind === "removed", "text-[color:var(--srv-green)]": l().kind === "added" }}
                    >
                      {g().sign}
                    </span>
                    <span class="pl-1">{l().text}</span>
                  </div>
                );
              }}
            </Show>
          )}
        </For>
      </div>
    </div>
  );
}

function SideBySide(props: { detail: DiffDetail }) {
  const col = (title: string, text: string) => (
    <div class="flex min-h-0 min-w-0 flex-1 flex-col">
      <div class="shrink-0 border-b border-border px-2 py-0.5 text-[12px] font-semibold">{title}</div>
      <pre tabIndex={0} class="m-0 min-h-0 flex-1 overflow-auto p-2 font-mono text-[12px]">
        {text}
      </pre>
    </div>
  );
  return (
    <div class="flex min-h-0 flex-1 flex-col">
      <p role="status" class="shrink-0 px-2 py-1 text-[12px] text-muted">
        {tsd.detail.tooLarge}
      </p>
      <div class="flex min-h-0 flex-1 divide-x divide-border border-t border-border">
        {col(tsd.detail.targetCol, props.detail.targetText)}
        {col(tsd.detail.sourceCol, props.detail.sourceText)}
      </div>
    </div>
  );
}

/** `picked = false` → seçim yok; `detail = null` → yükleniyor. */
export function LineDiffView(props: { picked: boolean; detail: DiffDetail | null }) {
  return (
    <Switch>
      <Match when={!props.picked}>
        <p class="p-3 text-muted">{tsd.detail.pick}</p>
      </Match>
      <Match when={props.detail === null}>
        <p class="p-3 text-muted">{tsd.loading}</p>
      </Match>
      <Match when={props.detail?.lines}>{(lines) => <Lines lines={lines()} />}</Match>
      <Match when={props.detail}>{(d) => <SideBySide detail={d()} />}</Match>
    </Switch>
  );
}
