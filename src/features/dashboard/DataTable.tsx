// Genel sanal tablo (Oturumlar, Kilitler): TanStack Virtual, `role="grid"`, başlığa tıklayınca sıralama, klavyeyle satır
// seçimi (Yukarı/Aşağı/Home/End/PageUp/PageDown), Enter = ayrıntı, Delete = iptal isteği. Sorgu sonuç ızgarasından
// (`ResultGrid`) ayrıdır: orada hücre/NULL/tür başlığı özeldir. Sıralama durumunu görünüm tutar.
import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect, createUniqueId, on, untrack, type JSX } from "solid-js";
import { ArrowDown, ArrowUp } from "lucide-solid";
import { ariaSort, isNavKey, navIndex, type SortState } from "./table-model";

export interface DataColumn<T> {
  key: string;
  header: string;
  /** Piksel. */
  width: number;
  cell: (row: T) => JSX.Element;
  sortable?: boolean;
  align?: "left" | "right";
}

export const TABLE_ROW_HEIGHT = 26;
const HEADER_HEIGHT = 28;

export function DataTable<T>(props: {
  columns: DataColumn<T>[];
  rows: readonly T[];
  rowKey: (row: T) => string | number;
  ariaLabel: string;
  selectedKey?: string | number | null;
  onSelect?: (key: string | number | null) => void;
  onActivate?: (row: T) => void;
  onDelete?: (row: T) => void;
  sort?: SortState | null;
  onSort?: (key: string) => void;
  class?: string;
}) {
  const uid = createUniqueId();
  let scrollEl!: HTMLDivElement;
  const totalWidth = () => props.columns.reduce((a, c) => a + c.width, 0);
  const indexOfKey = (k: string | number | null | undefined) =>
    k == null ? -1 : props.rows.findIndex((r) => props.rowKey(r) === k);
  const rowId = (k: string | number) => `${uid}-${k}`;

  const rowV = createVirtualizer({
    get count() {
      return props.rows.length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => TABLE_ROW_HEIGHT,
    overscan: 8,
    paddingStart: HEADER_HEIGHT,
    scrollPaddingStart: HEADER_HEIGHT,
  });

  // Seçili satır yalnızca seçim anahtarı ya da sıralama değişince görünür yapılır; veri yenilemesinin kaydırdığı dizin
  // görünümü geri çekmez.
  createEffect(
    on(
      () => [props.selectedKey, props.sort?.key, props.sort?.dir] as const,
      () => {
        const i = untrack(() => indexOfKey(props.selectedKey));
        if (i >= 0) rowV.scrollToIndex(i, { align: "auto" });
      },
      { defer: true },
    ),
  );

  const onKeyDown = (e: KeyboardEvent) => {
    // Satır içindeki düğmelerin (İptal/Sonlandır) kendi Enter/Boşluk davranışı bozulmasın.
    if (e.target !== e.currentTarget) return;
    const n = props.rows.length;
    const cur = indexOfKey(props.selectedKey);
    if (isNavKey(e.key)) {
      if (n === 0) return;
      e.preventDefault();
      const page = Math.max(1, Math.floor((scrollEl.clientHeight - HEADER_HEIGHT) / TABLE_ROW_HEIGHT) - 1);
      const next = navIndex(cur, e.key, n, page);
      if (next >= 0) props.onSelect?.(props.rowKey(props.rows[next]));
      return;
    }
    const row = cur >= 0 ? props.rows[cur] : undefined;
    if (!row) return;
    if (e.key === "Enter" && props.onActivate) {
      e.preventDefault();
      props.onActivate(row);
    } else if (e.key === "Delete" && props.onDelete) {
      e.preventDefault();
      props.onDelete(row);
    }
  };

  const activeId = () => {
    const k = props.selectedKey;
    return k != null && indexOfKey(k) >= 0 ? rowId(k) : undefined;
  };

  return (
    <div
      ref={scrollEl}
      role="grid"
      aria-label={props.ariaLabel}
      aria-rowcount={props.rows.length + 1}
      aria-colcount={props.columns.length}
      aria-activedescendant={activeId()}
      tabIndex={0}
      class={`relative min-h-0 overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent ${props.class ?? ""}`}
      onKeyDown={onKeyDown}
    >
      <div style={{ width: `${totalWidth()}px`, height: `${rowV.getTotalSize()}px`, position: "relative", "min-width": "100%" }}>
        <div
          role="row"
          aria-rowindex={1}
          class="sticky top-0 z-10 flex border-b border-border bg-panel"
          style={{ height: `${HEADER_HEIGHT}px`, width: `${totalWidth()}px` }}
        >
          <For each={props.columns}>
            {(col, i) => (
              <div
                role="columnheader"
                aria-colindex={i() + 1}
                aria-sort={col.sortable ? ariaSort(props.sort ?? null, col.key) : undefined}
                class="flex shrink-0 items-center overflow-hidden border-r border-border/50 text-[12px] font-semibold"
                classList={{ "justify-end": col.align === "right" }}
                style={{ width: `${col.width}px` }}
              >
                <Show when={col.sortable} fallback={<span class="truncate px-2">{col.header}</span>}>
                  <button
                    type="button"
                    tabIndex={-1}
                    class="flex h-full min-w-0 flex-1 items-center gap-1 px-2 hover:bg-hover"
                    classList={{ "justify-end": col.align === "right" }}
                    onClick={() => props.onSort?.(col.key)}
                  >
                    <span class="truncate">{col.header}</span>
                    <Show when={props.sort?.key === col.key}>
                      <Show when={props.sort?.dir === "asc"} fallback={<ArrowDown size={12} aria-hidden="true" />}>
                        <ArrowUp size={12} aria-hidden="true" />
                      </Show>
                    </Show>
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>
        <For each={rowV.getVirtualItems()}>
          {(item) => (
            <Show when={item.index < props.rows.length}>
              {(() => {
                const row = () => props.rows[item.index];
                const key = () => props.rowKey(row());
                const selected = () => props.selectedKey != null && key() === props.selectedKey;
                return (
                  <div
                    role="row"
                    id={rowId(key())}
                    aria-rowindex={item.index + 2}
                    aria-selected={selected()}
                    class="absolute left-0 flex cursor-default items-center hover:bg-hover"
                    classList={{ "bg-selected": selected() }}
                    style={{ top: `${item.start}px`, height: `${TABLE_ROW_HEIGHT}px`, width: `${totalWidth()}px` }}
                    onClick={() => props.onSelect?.(key())}
                    onDblClick={() => props.onActivate?.(row())}
                  >
                    <For each={props.columns}>
                      {(col, i) => (
                        <div
                          role="gridcell"
                          aria-colindex={i() + 1}
                          class="shrink-0 overflow-hidden whitespace-nowrap px-2 text-[12px]"
                          classList={{ "text-right": col.align === "right" }}
                          style={{ width: `${col.width}px`, "text-overflow": "ellipsis" }}
                        >
                          {col.cell(row())}
                        </div>
                      )}
                    </For>
                  </div>
                );
              })()}
            </Show>
          )}
        </For>
      </div>
    </div>
  );
}
