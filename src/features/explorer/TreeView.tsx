import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect, createMemo, untrack } from "solid-js";
import { t } from "../../app/strings";
import type { Explorer } from "./explorer-store";
import { ROW_HEIGHT, TreeRow, rowDomId } from "./TreeRow";
import { ROOT } from "./tree-model";

export function TreeView(props: { explorer: Explorer; treeRef?: (el: HTMLElement) => void }) {
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
      aria-activedescendant={ex.state().selected !== null ? rowDomId(ex.state().selected!) : undefined}
      tabIndex={0}
      class="relative h-full overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent"
      onKeyDown={(e) => {
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
