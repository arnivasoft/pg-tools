import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect, createMemo, untrack } from "solid-js";
import { t } from "../../app/strings";
import type { Explorer } from "./explorer-store";
import { ROW_HEIGHT, TreeRow, rowDomId } from "./TreeRow";
import { ROOT } from "./tree-model";

export function TreeView(props: { explorer: Explorer; treeRef?: (el: HTMLElement) => void; onCopyName?: (id: string) => void; onOpenQuery?: (id: string) => void }) {
  const ex = props.explorer;
  let scrollEl!: HTMLDivElement;
  const virtualizer = createVirtualizer({
    get count() {
      return ex.rows().length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  const selectedId = createMemo(() => ex.state().selected);
  createEffect(() => {
    const selected = selectedId();
    if (selected === null) return;
    const index = untrack(() => ex.rows().findIndex((r) => r.id === selected));
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
  });

  // aria-activedescendant yalnızca DOM'da gerçekten bulunan bir satırı göstermeli: seçili düğüm
  // süzmeyle gizlenmiş ya da sanal kaydırmada çizilmemiş olabilir.
  const activeDescendant = createMemo(() => {
    const selected = selectedId();
    if (selected === null) return undefined;
    const index = ex.rows().findIndex((r) => r.id === selected);
    if (index < 0 || !virtualizer.getVirtualItems().some((item) => item.index === index)) return undefined;
    return rowDomId(selected);
  });

  const filtering = () => ex.state().filter.trim() !== "";
  const isEmpty = () => ex.rows().length === 0 && (filtering() || ex.state().children.has(ROOT));

  return (
    <div
      ref={(el) => {
        scrollEl = el;
        props.treeRef?.(el);
      }}
      role="tree"
      aria-label={t.explorer.ariaLabel}
      aria-activedescendant={activeDescendant()}
      tabIndex={0}
      class="relative h-full overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent"
      onKeyDown={(e) => {
        // Rozet düğmesi gibi iç öğelerden gelen tuşlar (Enter/Boşluk) ağaç gezintisine karışmasın.
        if (e.target !== e.currentTarget) return;
        const sel = ex.state().selected;
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "c" && sel !== null && (window.getSelection()?.toString() ?? "") === "") {
          e.preventDefault();
          props.onCopyName?.(sel);
          return;
        }
        // Ctrl+Alt+Q: seçili düğümü Query Tool'da aç (Türkçe Q klavyede AltGr+Q = "@", bu yüzden yalnızca ağaçta).
        if (e.ctrlKey && e.altKey && !e.shiftKey && (e.key.toLowerCase() === "q" || e.code === "KeyQ") && sel !== null) {
          e.preventDefault();
          props.onOpenQuery?.(sel);
          return;
        }
        if (ex.controller.handleKey(e.key)) e.preventDefault();
      }}
    >
      <Show when={!isEmpty()} fallback={<p class="p-4 text-muted">{filtering() ? t.explorer.noMatch : t.explorer.empty}</p>}>
        <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
          <For each={virtualizer.getVirtualItems()}>
            {(item) => (
              <Show when={ex.rows()[item.index]}>
                {(row) => (
                  <TreeRow
                    row={row()}
                    top={item.start}
                    selected={ex.state().selected === row().id}
                    onSelect={() => ex.controller.select(row().id)}
                    onToggle={() => void ex.controller.toggle(row().id)}
                    onRetry={() => void ex.controller.retry(row().id)}
                    onRefreshParent={() => {
                      const parent = ex.state().parent.get(row().id);
                      if (parent === undefined) return;
                      void (parent === ROOT ? ex.controller.reloadServers() : ex.controller.refresh(parent));
                    }}
                  />
                )}
              </Show>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
