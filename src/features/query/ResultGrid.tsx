import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, on, untrack } from "solid-js";
import { copyText } from "../../app/clipboard";
import { t } from "../../app/strings";
import type { Column } from "../../ipc/bindings/Column";
import { Button } from "../../ui/Button";
import {
  NAV_KEYS, ROW_HEIGHT, SAMPLE_ROWS, MAX_COL_WIDTH, MIN_COL_WIDTH, displayCell, extendTo, inRect, initialWidths,
  isNumericType, moveFocus, selRect, selectAll, selectCell, selectColumn, selectRow, selectionToTsv,
  type NavKey, type Selection, type TextMeasure,
} from "./grid-model";
import { LOAD_MORE_CHUNK, type StmtResult } from "./result-model";

/** İki satırlı başlık: ad (12 px, satır 16) + tür (11 px, satır 14) + dikey boşluk. Satırlar 24 px kalır. */
const HEADER_HEIGHT = 36;
const FONT = '12px "Cascadia Mono", Consolas, monospace';
const SANS_FALLBACK = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';

type Kind = "header" | "type" | "cell";
const measureCtx: Partial<Record<Kind, CanvasRenderingContext2D | null>> = {};
/** Başlık/tür yazı tipi, sayfanın gerçek sans yazı tipiyle (Tailwind `font-sans`) aynıdır. */
function fontFor(kind: Kind): string {
  if (kind === "cell") return FONT;
  const family = getComputedStyle(document.body).fontFamily || SANS_FALLBACK;
  return kind === "header" ? `600 12px ${family}` : `400 11px ${family}`;
}
function measureWith(kind: Kind, s: string, perChar: number): number {
  let ctx = measureCtx[kind];
  if (ctx === undefined) {
    ctx = measureCtx[kind] = document.createElement("canvas").getContext("2d");
    if (ctx) ctx.font = fontFor(kind);
  }
  return ctx ? ctx.measureText(s).width : s.length * perChar;
}
const measure: TextMeasure = {
  header: (s) => measureWith("header", s, 7.5),
  type: (s) => measureWith("type", s, 6.5),
  cell: (s) => measureWith("cell", s, 7.2),
};

/** Odaktaki hücre (Hücre paneli için). `value`: ham tam değer. */
export interface ActiveCell {
  row: number;
  column: Column;
  value: string | null;
}

export function ResultGrid(props: {
  result: StmtResult;
  /** "Daha fazla yükle" etkin mi (`canLoadMore`); düğme yalnızca `more`/`truncated` iken görünür. */
  canLoadMore: boolean;
  onLoadMore: () => void;
  onActiveCell?: (cell: ActiveCell | null) => void;
}) {
  const uid = createUniqueId();
  let scrollEl!: HTMLDivElement;

  const rows = () => props.result.rows;
  const columns = () => props.result.columns;
  const rowNumWidth = () => Math.max(44, String(rows().length).length * 9 + 20);

  const [widths, setWidths] = createSignal<number[]>([]);
  let sizedWithRows = false;
  let userResized = false;
  // Sütun kümesi değişince (yeni ifade) genişlikleri ve seçimi sıfırla.
  createEffect(
    on(columns, (cols) => {
      userResized = false;
      sizedWithRows = rows().length > 0;
      setWidths(initialWidths(cols, rows().slice(0, SAMPLE_ROWS), measure));
      setSel(null);
    }),
  );
  // Satırlar ilk kez geldiğinde (başlangıçta boştu) örneğe göre yeniden boyutla.
  let prevRows = 0;
  createEffect(() => {
    const n = rows().length;
    const dropped = props.result.dropped;
    untrack(() => {
      if (dropped || n < prevRows) setSel(null);
      prevRows = n;
      if (n > 0 && !sizedWithRows && !userResized) {
        sizedWithRows = true;
        setWidths(initialWidths(columns(), rows().slice(0, SAMPLE_ROWS), measure));
      }
    });
  });

  const rowV = createVirtualizer({
    get count() {
      return rows().length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
    get paddingStart() {
      return HEADER_HEIGHT;
    },
    scrollPaddingStart: HEADER_HEIGHT,
  });
  const colV = createVirtualizer({
    horizontal: true,
    get count() {
      return columns().length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: (i) => widths()[i] ?? MIN_COL_WIDTH,
    overscan: 2,
    get paddingStart() {
      return rowNumWidth();
    },
    scrollPaddingStart: rowNumWidth(),
  });
  createEffect(() => {
    widths();
    rowNumWidth();
    colV.measure();
  });

  // ---- Seçim ----
  const [sel, setSel] = createSignal<Selection | null>(null);
  const rect = createMemo(() => {
    const s = sel();
    return s ? selRect(s) : null;
  });
  const focus = () => sel()?.focus ?? null;

  createEffect(() => {
    const f = focus();
    untrack(() => {
      if (!f) return props.onActiveCell?.(null);
      const col = columns()[f.c];
      if (!col || f.r >= rows().length) return props.onActiveCell?.(null);
      props.onActiveCell?.({ row: f.r, column: col, value: rows()[f.r][f.c] ?? null });
    });
  });

  const reveal = (r: number, c: number) => {
    rowV.scrollToIndex(r, { align: "auto" });
    colV.scrollToIndex(c, { align: "auto" });
  };
  const cellId = (r: number, c: number) => `${uid}-${r}-${c}`;
  const activeDescendant = createMemo(() => {
    const f = focus();
    if (!f) return undefined;
    const rv = rowV.getVirtualItems().some((i) => i.index === f.r);
    const cv = colV.getVirtualItems().some((i) => i.index === f.c);
    return rv && cv ? cellId(f.r, f.c) : undefined;
  });

  const copy = (headers: boolean) => {
    const r = rect();
    if (!r) return;
    void copyText(selectionToTsv(rows(), columns(), r, { headers })).catch(() => {});
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const key = e.key;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && !e.altKey && key.toLowerCase() === "a") {
      e.preventDefault();
      if (rows().length > 0) setSel(selectAll(rows().length, columns().length));
      return;
    }
    if (mod && !e.altKey && key.toLowerCase() === "c") {
      const ae = document.activeElement as HTMLElement | null;
      if (ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA" || ae.isContentEditable)) return;
      const ds = window.getSelection();
      if (ds && ds.toString() !== "" && ds.anchorNode && scrollEl.contains(ds.anchorNode)) return;
      e.preventDefault();
      copy(e.shiftKey);
      return;
    }
    if (!NAV_KEYS.has(key) || rows().length === 0 || columns().length === 0) return;
    e.preventDefault();
    const cur = sel();
    const page = Math.floor((scrollEl.clientHeight - HEADER_HEIGHT) / ROW_HEIGHT) - 1;
    const from = cur?.focus ?? { r: 0, c: 0 };
    const to = cur ? moveFocus(from, key as NavKey, rows().length, columns().length, page, mod) : from;
    setSel(cur && e.shiftKey ? extendTo(cur, to) : selectCell(to.r, to.c));
    reveal(to.r, to.c);
  };

  // ---- Fare (olay yetkilendirme: tek dinleyici) ----
  let dragging = false;
  const posOf = (target: EventTarget | null): { r: number | null; c: number | null } | null => {
    const el = (target as HTMLElement | null)?.closest<HTMLElement>("[data-r],[data-c]");
    if (!el) return null;
    return { r: el.dataset.r === undefined ? null : Number(el.dataset.r), c: el.dataset.c === undefined ? null : Number(el.dataset.c) };
  };
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("[data-resize]")) return;
    const p = posOf(e.target);
    if (!p) return;
    window.getSelection()?.removeAllRanges();
    scrollEl.focus();
    const n = rows().length;
    const cc = columns().length;
    let next: Selection;
    if (p.r !== null && p.c !== null) next = selectCell(p.r, p.c);
    else if (p.r !== null) next = selectRow(p.r, cc);
    else if (p.c !== null && n > 0) next = selectColumn(p.c, n);
    else return;
    const cur = sel();
    if (e.shiftKey && cur) next = extendTo(cur, next.focus);
    setSel(next);
    dragging = p.r !== null && p.c !== null;
    if (dragging) {
      const up = () => {
        dragging = false;
        window.removeEventListener("mouseup", up);
      };
      window.addEventListener("mouseup", up);
    }
    e.preventDefault();
  };
  const onMouseOver = (e: MouseEvent) => {
    if (!dragging) return;
    const p = posOf(e.target);
    const cur = sel();
    if (p && p.r !== null && p.c !== null && cur) setSel(extendTo(cur, { r: p.r, c: p.c }));
  };

  // ---- Sütun yeniden boyutlandırma ----
  const setWidth = (c: number, w: number) => {
    userResized = true;
    setWidths((prev) => {
      const next = prev.slice();
      next[c] = Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, Math.round(w)));
      return next;
    });
  };
  const onResizeDown = (e: PointerEvent, c: number) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startW = widths()[c] ?? MIN_COL_WIDTH;
    const move = (ev: PointerEvent) => setWidth(c, startW + ev.clientX - startX);
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };
  const autoFit = (c: number) => {
    const col = columns()[c];
    if (!col) return;
    const sample = rows().slice(0, SAMPLE_ROWS).map((r) => [r[c]]);
    setWidth(c, initialWidths([col], sample, measure)[0]);
  };

  const showMore = () => props.result.more || props.result.truncated;

  return (
    <div class="flex h-full min-h-0 flex-col bg-bg">
      <div
        ref={scrollEl}
        role="grid"
        aria-label={t.query.grid.ariaLabel}
        aria-rowcount={rows().length + 1}
        aria-colcount={columns().length}
        aria-multiselectable="true"
        aria-activedescendant={activeDescendant()}
        tabIndex={0}
        class="relative min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent select-none"
        style={{ font: FONT }}
        onKeyDown={onKeyDown}
        onFocus={() => window.getSelection()?.removeAllRanges()}
        onMouseDown={onMouseDown}
        onMouseOver={onMouseOver}
      >
        <Show
          when={!props.result.dropped}
          fallback={<p class="p-4 font-sans text-[13px] text-muted">{t.query.grid.dropped}</p>}
        >
          <div
            style={{
              width: `${colV.getTotalSize()}px`,
              height: `${rowV.getTotalSize()}px`,
              position: "relative",
              "min-width": "100%",
            }}
          >
            <div
              role="row"
              aria-rowindex={1}
              class="sticky top-0 z-20 border-b border-border bg-panel"
              style={{ height: `${HEADER_HEIGHT}px`, width: `${colV.getTotalSize()}px` }}
            >
              <div
                class="sticky left-0 z-30 border-r border-border bg-panel"
                style={{ width: `${rowNumWidth()}px`, height: `${HEADER_HEIGHT}px`, position: "sticky" }}
                role="columnheader"
                aria-label={t.query.grid.rowNumber}
              />
              <For each={colV.getVirtualItems()}>
                {(item) => (
                  <Show when={columns()[item.index]}>
                    {(col) => (
                      <div
                        role="columnheader"
                        aria-colindex={item.index + 1}
                        data-c={item.index}
                        title={`${col().name} (${col().typeDisplay})`}
                        class="absolute top-0 flex flex-col justify-center overflow-hidden border-r border-border px-2 font-sans"
                        classList={{ "text-right": isNumericType(col().typeName) }}
                        style={{ left: `${item.start}px`, width: `${widths()[item.index] ?? MIN_COL_WIDTH}px`, height: `${HEADER_HEIGHT}px` }}
                      >
                        <span class="truncate text-[12px] font-semibold leading-4">{col().name}</span>
                        <span class="truncate text-[11px] leading-[14px] text-muted">{col().typeDisplay}</span>
                        <div
                          data-resize
                          aria-hidden="true"
                          class="absolute right-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-accent/40"
                          onPointerDown={(e) => onResizeDown(e, item.index)}
                          onDblClick={(e) => {
                            e.stopPropagation();
                            autoFit(item.index);
                          }}
                        />
                      </div>
                    )}
                  </Show>
                )}
              </For>
            </div>
            <For each={rowV.getVirtualItems()}>
              {(row) => (
                <Show when={row.index < rows().length}>
                  <div
                    role="row"
                    aria-rowindex={row.index + 2}
                    class="absolute left-0 hover:bg-hover"
                    style={{ top: `${row.start}px`, height: `${ROW_HEIGHT}px`, width: `${colV.getTotalSize()}px` }}
                  >
                    <div
                      data-r={row.index}
                      class="sticky left-0 z-10 border-r border-border bg-panel px-2 text-right text-muted"
                      style={{ width: `${rowNumWidth()}px`, height: `${ROW_HEIGHT}px`, "line-height": `${ROW_HEIGHT}px` }}
                    >
                      {row.index + 1}
                    </div>
                    <For each={colV.getVirtualItems()}>
                      {(cv) => (
                        <Show when={columns()[cv.index]}>
                          {(col) => {
                            const cell = createMemo(() => displayCell(rows()[row.index]?.[cv.index] ?? null));
                            const selected = () => {
                              const r = rect();
                              return r !== null && inRect(r, row.index, cv.index);
                            };
                            const isFocus = () => {
                              const f = focus();
                              return f !== null && f.r === row.index && f.c === cv.index;
                            };
                            return (
                              <div
                                id={cellId(row.index, cv.index)}
                                role="gridcell"
                                aria-colindex={cv.index + 1}
                                aria-selected={selected()}
                                data-r={row.index}
                                data-c={cv.index}
                                class="absolute top-0 overflow-hidden whitespace-nowrap border-r border-border/50 px-2"
                                classList={{
                                  "bg-selected": selected(),
                                  "text-right": isNumericType(col().typeName),
                                  "outline outline-1 -outline-offset-1 outline-accent": isFocus(),
                                }}
                                style={{
                                  left: `${cv.start}px`,
                                  width: `${widths()[cv.index] ?? MIN_COL_WIDTH}px`,
                                  height: `${ROW_HEIGHT}px`,
                                  "line-height": `${ROW_HEIGHT}px`,
                                  "text-overflow": "ellipsis",
                                }}
                              >
                                <Show when={!cell().null} fallback={<span class="italic text-muted">{t.query.grid.nullText}</span>}>
                                  {cell().text}
                                </Show>
                              </div>
                            );
                          }}
                        </Show>
                      )}
                    </For>
                  </div>
                </Show>
              )}
            </For>
          </div>
        </Show>
      </div>
      <div class="flex h-8 shrink-0 items-center gap-3 border-t border-border bg-panel px-3 font-sans text-[12px]">
        <span>{t.query.grid.rowsLoaded(rows().length)}</span>
        <Show when={props.result.truncated}>
          <span class="text-danger">{t.query.grid.limitWarning}</span>
        </Show>
        <Show when={showMore()}>
          <Button class="ml-auto h-6 text-[12px]" disabled={!props.canLoadMore} onClick={() => props.onLoadMore()}>
            {t.query.grid.loadMore(LOAD_MORE_CHUNK)}
          </Button>
        </Show>
      </div>
    </div>
  );
}
