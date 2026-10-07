// Düzenlenebilir veri ızgarası (tembel DataTabs parçası). `query/ResultGrid` kalıbının KOPYASI (sanallaştırıcı, yapışkan
// başlık, sütun boyutlandırma, `aria-activedescendant`, `data-r`/`data-c` olay yetkilendirmesi); o bileşen içe aktarılmaz
// (Plan 17 Karar 2). Tek hücre odağı; satırlar = yüklü satırlar + yeni satırlar (alta). Kısayollar Karar 8; F5'i QueryTabs
// yakalar (`reloadNow`). Değerler günlüğe/depoya yazılmaz; yalnızca ekranda (ve Ctrl+C ile panoya) gider.
import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, on, untrack } from "solid-js";
import { copyText } from "../../app/clipboard";
import { tdata } from "../../app/strings-data";
import type { CellValue } from "../../ipc/bindings/CellValue";
import type { DataModel } from "./data-controller";
import type { Row } from "./data-edit-model";
import {
  MAX_COL_WIDTH, MIN_COL_WIDTH, NAV_KEYS, ROW_HEIGHT, SAMPLE_ROWS, cellKind, commitText, copyValue, displayCell, editableAt,
  initialWidths, isNumericType, moveFocus, rowCountOf, setValue, slotAt, slotKey, useTextarea, valueAt,
  type GridSlot, type IssueMaps, type NavKey, type Pos, type TextMeasure,
} from "./data-grid-model";
import { ROW_CAP } from "./data-controller";
import type { DataController } from "./data-open";

const HEADER_HEIGHT = 36;
const FONT = '12px "Cascadia Mono", Consolas, monospace';
const SANS_FALLBACK = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
/** Çok satırlı düzenleyicinin yüksekliği / en az genişliği. */
const AREA_HEIGHT = 120;
const AREA_MIN_WIDTH = 280;

type MKind = "header" | "type" | "cell";
const measureCtx: Partial<Record<MKind, CanvasRenderingContext2D | null>> = {};
function fontFor(kind: MKind): string {
  if (kind === "cell") return FONT;
  const family = getComputedStyle(document.body).fontFamily || SANS_FALLBACK;
  return kind === "header" ? `600 12px ${family}` : `400 11px ${family}`;
}
function measureWith(kind: MKind, s: string, perChar: number): number {
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

// Durum zeminleri yalnız CSS değişkenleriyle (renk tek başına bilgi değildir: `sr-only` metni de var).
const TONE_CHANGED = "bg-[color-mix(in_srgb,var(--accent)_14%,transparent)]";
const TONE_DELETED = "bg-[color-mix(in_srgb,var(--danger)_10%,transparent)]";
const TONE_NEW = "bg-[color-mix(in_srgb,var(--srv-green)_10%,transparent)]";
const TONE_CONFLICT = "bg-[color-mix(in_srgb,var(--srv-amber)_24%,transparent)]";

/** Izgara komutları (araç çubuğuyla ortak; etkin hücreye uygulanır). */
export interface GridCommands {
  setNull(): void;
  setDefault(): void;
  revert(): void;
  toggleDelete(): void;
  addRow(): void;
  sort(column: string, additive: boolean): void;
}

interface Editing {
  r: number;
  c: number;
  initial: string;
  multi: boolean;
  /** Düzenlenen yuva/kolon ve (var olan satırda) özgün satır nesnesi: arada yeniden yükleme/uygulama olduysa yazılmaz. */
  slot: GridSlot;
  column: string;
  rowRef: Row | null;
}

export function DataGrid(props: {
  data: DataController;
  m: DataModel;
  active: Pos | null;
  setActive: (p: Pos | null) => void;
  /** Düzenleme komutları kapalıysa gerekçesi. */
  blocked: string | null;
  errors: IssueMaps;
  cmd: GridCommands;
}) {
  const uid = createUniqueId();
  let scrollEl!: HTMLDivElement;
  let editorEl: HTMLInputElement | HTMLTextAreaElement | undefined;

  const l = () => props.m.loaded;
  const e = () => props.m.edit;
  const columns = () => l()?.source.columns ?? [];
  const count = () => {
    const x = l();
    return x ? rowCountOf(x, e()) : 0;
  };
  const rowNumWidth = () => Math.max(52, String(count()).length * 9 + 28);

  const [widths, setWidths] = createSignal<number[]>([]);
  let sizedWithRows = false;
  let userResized = false;
  // Kaynak (kolon kümesi) değişince genişlikler sıfırlanır; satırlar ilk geldiğinde örneğe göre yeniden boyutlanır.
  createEffect(
    on(
      () => l()?.source.fingerprint,
      () => {
        const x = untrack(l);
        userResized = false;
        sizedWithRows = (x?.rows.length ?? 0) > 0;
        setWidths(initialWidths(x?.source.columns ?? [], x?.rows.slice(0, SAMPLE_ROWS) ?? [], measure));
      },
    ),
  );
  createEffect(() => {
    const x = l();
    const n = x?.rows.length ?? 0;
    untrack(() => {
      if (x && n > 0 && !sizedWithRows && !userResized) {
        sizedWithRows = true;
        setWidths(initialWidths(x.source.columns, x.rows.slice(0, SAMPLE_ROWS), measure));
      }
    });
  });
  // Satır sayısı küçülürse (yeniden yükleme, silinen satırlar) odak sınıra çekilir.
  createEffect(() => {
    const n = count();
    const cc = columns().length;
    untrack(() => {
      const p = props.active;
      if (!p) return;
      if (n === 0 || cc === 0) props.setActive(null);
      else if (p.r >= n || p.c >= cc) props.setActive({ r: Math.min(p.r, n - 1), c: Math.min(p.c, cc - 1) });
    });
  });

  const rowV = createVirtualizer({
    get count() {
      return count();
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

  const reveal = (p: Pos) => {
    rowV.scrollToIndex(p.r, { align: "auto" });
    colV.scrollToIndex(p.c, { align: "auto" });
  };
  // Etkin hücre dışarıdan değişince de (araç çubuğundan satır ekleme) görünür olsun.
  createEffect(on(() => props.active, (p) => p && reveal(p), { defer: true }));
  const cellId = (r: number, c: number) => `${uid}-${r}-${c}`;
  const activeDescendant = createMemo(() => {
    const f = props.active;
    if (!f) return undefined;
    const rv = rowV.getVirtualItems().some((i) => i.index === f.r);
    const cv = colV.getVirtualItems().some((i) => i.index === f.c);
    return rv && cv ? cellId(f.r, f.c) : undefined;
  });

  // ---- Satır içi düzenleyici ----
  const [editing, setEditing] = createSignal<Editing | null>(null);
  const [draft, setDraft] = createSignal("");

  const cellAt = (r: number, c: number) => {
    const x = l();
    const col = columns()[c];
    const s = x ? slotAt(x, e(), r) : null;
    return x && col && s ? { l: x, s, col } : null;
  };

  /** Yuva hâlâ aynı satırı mı gösteriyor (yeniden yükleme/uygulama satırları değiştirmiş olabilir)? */
  const stillValid = (ed: Editing): boolean => {
    const x = l();
    const s = ed.slot;
    return x !== null && (s.kind === "row" ? x.rows[s.rowIndex] === ed.rowRef : e().inserted.some((n) => n.id === s.newId));
  };
  // Satırlar altından değişirse düzenleyici yazmadan kapanır.
  createEffect(
    on([l, e], () => {
      const ed = untrack(editing);
      if (ed && !stillValid(ed)) setEditing(null);
    }, { defer: true }),
  );

  const startEdit = (p: Pos | null) => {
    if (!p || props.blocked) return;
    const at = cellAt(p.r, p.c);
    if (!at || !editableAt(at.l, e(), at.s, at.col.name).editable) return;
    const v = valueAt(at.l, e(), at.s, at.col.name).value;
    // Tam değer (kırpılmamış) yüklenir; NULL/DEFAULT boş başlar ve değişmeden onaylanırsa yazılmaz.
    const text = v.kind === "text" ? v.value : "";
    reveal(p);
    setDraft(text);
    setEditing({
      r: p.r, c: p.c, initial: text, multi: useTextarea(text), slot: at.s, column: at.col.name,
      rowRef: at.s.kind === "row" ? at.l.rows[at.s.rowIndex] : null,
    });
    queueMicrotask(() => {
      editorEl?.focus();
      if (editorEl instanceof HTMLInputElement) editorEl.select();
    });
  };

  /** Düzenleyiciyi kapatır; `commit` ise değer yazılır. `move` sonrası odak (ızgaraya döner). */
  const finishEdit = (commit: boolean, move?: NavKey, refocus = true) => {
    const ed = editing();
    if (!ed) return;
    setEditing(null);
    const x = l();
    const s = ed.slot;
    if (commit && x && stillValid(ed)) {
      const next = commitText(valueAt(x, e(), s, ed.column).value, ed.initial, draft());
      if (next) props.data.edit(props.m.tabId, (lx, ex) => setValue(lx, ex, s, ed.column, next));
    }
    const p = move ? moveFocus({ r: ed.r, c: ed.c }, move, count(), columns().length, 1) : { r: ed.r, c: ed.c };
    props.setActive(p);
    reveal(p);
    if (refocus) scrollEl.focus();
  };

  const command = (ev: KeyboardEvent): boolean => {
    const k = ev.key.toLowerCase();
    const mod = ev.ctrlKey || ev.metaKey;
    // AltGr = Ctrl+Alt: Ctrl+Alt kombinasyonları kullanılmaz.
    if (ev.altKey && !mod) {
      if (k === "n") props.cmd.setNull();
      else if (k === "d") props.cmd.setDefault();
      else if (k === "s") {
        const col = props.active ? columns()[props.active.c] : undefined;
        if (col) props.cmd.sort(col.name, ev.shiftKey);
      } else return false;
      return true;
    }
    return false;
  };

  const onEditorKey = (ev: KeyboardEvent) => {
    ev.stopPropagation();
    const ed = editing();
    if (!ed) return;
    if (ev.key === "Escape") {
      ev.preventDefault();
      finishEdit(false);
    } else if (ev.key === "Enter" && (!ed.multi || ev.ctrlKey) && !ev.altKey) {
      ev.preventDefault();
      finishEdit(true, ev.shiftKey ? "ArrowUp" : "ArrowDown");
    } else if (ev.key === "Tab") {
      ev.preventDefault();
      finishEdit(true, ev.shiftKey ? "ArrowLeft" : "ArrowRight");
    } else if (ev.altKey && !ev.ctrlKey && !ev.metaKey && (ev.key.toLowerCase() === "n" || ev.key.toLowerCase() === "d")) {
      // NULL/DEFAULT düzenleyicide yazılamaz: komut düzenleyiciyi kapatıp hücreye uygulanır.
      ev.preventDefault();
      finishEdit(false);
      command(ev);
    }
  };

  // ---- Klavye (ızgara) ----
  const onKeyDown = (ev: KeyboardEvent) => {
    if (ev.target !== scrollEl) return;
    const key = ev.key;
    const mod = ev.ctrlKey || ev.metaKey;
    if (command(ev)) {
      ev.preventDefault();
      return;
    }
    if (mod && !ev.altKey && key.toLowerCase() === "c") {
      const p = props.active;
      const at = p ? cellAt(p.r, p.c) : null;
      if (!at) return;
      ev.preventDefault();
      void copyText(copyValue(valueAt(at.l, e(), at.s, at.col.name).value)).catch(() => {});
      return;
    }
    if (mod && !ev.altKey && key.toLowerCase() === "z") {
      ev.preventDefault();
      props.cmd.revert();
      return;
    }
    if (mod || ev.altKey) return;
    if (key === "Enter" || key === "F2") {
      ev.preventDefault();
      startEdit(props.active);
      return;
    }
    if (key === "Delete") {
      ev.preventDefault();
      props.cmd.toggleDelete();
      return;
    }
    if (key === "Insert") {
      ev.preventDefault();
      props.cmd.addRow();
      return;
    }
    if (!NAV_KEYS.has(key) || count() === 0 || columns().length === 0) return;
    ev.preventDefault();
    const cur = props.active;
    const page = Math.floor((scrollEl.clientHeight - HEADER_HEIGHT) / ROW_HEIGHT) - 1;
    const to = cur ? moveFocus(cur, key as NavKey, count(), columns().length, page, ev.ctrlKey) : { r: 0, c: 0 };
    props.setActive(to);
    reveal(to);
  };
  // Odak ilk kez ızgaraya gelince bir hücre etkin olsun (ekran okuyucu bir şey duyursun).
  const onFocus = () => {
    if (!props.active && count() > 0 && columns().length > 0) props.setActive({ r: 0, c: 0 });
  };

  // ---- Fare (olay yetkilendirme) ----
  const posOf = (target: EventTarget | null): Pos | null => {
    const el = (target as HTMLElement | null)?.closest<HTMLElement>("[data-r][data-c]");
    if (!el) return null;
    return { r: Number(el.dataset.r), c: Number(el.dataset.c) };
  };
  const onMouseDown = (ev: MouseEvent) => {
    if (ev.button !== 0 || (ev.target as HTMLElement).closest("[data-resize],[data-editor]")) return;
    const p = posOf(ev.target);
    if (!p) return;
    if (editing()) finishEdit(true);
    ev.preventDefault();
    scrollEl.focus();
    props.setActive(p);
  };
  const onDblClick = (ev: MouseEvent) => {
    const p = posOf(ev.target);
    if (p) startEdit(p);
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
  const onResizeDown = (ev: PointerEvent, c: number) => {
    if (ev.button !== 0) return;
    ev.preventDefault();
    ev.stopPropagation();
    const el = ev.currentTarget as HTMLElement;
    el.setPointerCapture(ev.pointerId);
    const startX = ev.clientX;
    const startW = widths()[c] ?? MIN_COL_WIDTH;
    const move = (pe: PointerEvent) => setWidth(c, startW + pe.clientX - startX);
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  const sortOf = (name: string) => {
    const order = props.m.select.order;
    const i = order.findIndex((o) => o.column === name);
    return i < 0 ? null : { pos: i + 1, desc: order[i].desc, many: order.length > 1 };
  };

  const colLeft = (c: number) => {
    let x = rowNumWidth();
    const w = widths();
    for (let i = 0; i < c; i++) x += w[i] ?? MIN_COL_WIDTH;
    return x;
  };

  const placeholder = (v: CellValue) => (v.kind === "null" ? tdata.sr.null : tdata.sr.default);

  return (
    <div class="flex h-full min-h-0 flex-col bg-bg">
      <div
        ref={scrollEl}
        role="grid"
        aria-label={tdata.grid.label}
        aria-rowcount={count() + 1}
        aria-colcount={columns().length}
        aria-readonly={props.blocked !== null}
        aria-activedescendant={activeDescendant()}
        aria-describedby={`${uid}-keys`}
        tabIndex={0}
        class="relative min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent select-none"
        style={{ font: FONT }}
        onKeyDown={onKeyDown}
        onFocus={onFocus}
        onMouseDown={onMouseDown}
        onDblClick={onDblClick}
      >
        <div
          style={{ width: `${colV.getTotalSize()}px`, height: `${rowV.getTotalSize()}px`, position: "relative", "min-width": "100%" }}
        >
          <div
            role="row"
            aria-rowindex={1}
            class="sticky top-0 z-20 border-b border-border bg-panel"
            style={{ height: `${HEADER_HEIGHT}px`, width: `${colV.getTotalSize()}px` }}
          >
            <div
              role="columnheader"
              class="sticky left-0 z-30 flex items-center border-r border-border bg-panel px-2 font-sans text-[11px] text-muted"
              style={{ width: `${rowNumWidth()}px`, height: `${HEADER_HEIGHT}px`, position: "sticky" }}
            >
              {tdata.grid.rowNumber}
            </div>
            <For each={colV.getVirtualItems()}>
              {(item) => (
                <Show when={columns()[item.index]}>
                  {(col) => {
                    const s = () => sortOf(col().name);
                    return (
                      <div
                        role="columnheader"
                        aria-colindex={item.index + 1}
                        aria-sort={s() ? (s()!.desc ? "descending" : "ascending") : undefined}
                        title={`${col().name} (${col().typeDisplay}) — ${tdata.sort.hint}`}
                        class="absolute top-0 flex cursor-pointer flex-col justify-center overflow-hidden border-r border-border px-2 font-sans hover:bg-hover"
                        classList={{ "text-right": isNumericType(col().typeDisplay) }}
                        style={{ left: `${item.start}px`, width: `${widths()[item.index] ?? MIN_COL_WIDTH}px`, height: `${HEADER_HEIGHT}px` }}
                        onClick={(ev) => {
                          if ((ev.target as HTMLElement).closest("[data-resize]")) return;
                          props.cmd.sort(col().name, ev.shiftKey);
                        }}
                      >
                        <span class="truncate text-[12px] font-semibold leading-4">
                          {col().name}
                          <Show when={s()}>
                            {(x) => (
                              <span class="ml-1 text-accent">
                                <span aria-hidden="true">
                                  {x().desc ? "▼" : "▲"}
                                  {x().many ? x().pos : ""}
                                </span>
                                <span class="sr-only">
                                  {`${x().desc ? tdata.sort.desc : tdata.sort.asc}${x().many ? `, ${tdata.sort.position(x().pos)}` : ""}`}
                                </span>
                              </span>
                            )}
                          </Show>
                        </span>
                        <span class="truncate text-[11px] leading-[14px] text-muted">{col().typeDisplay}</span>
                        <div
                          data-resize
                          aria-hidden="true"
                          class="absolute right-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-accent/40"
                          onPointerDown={(ev) => onResizeDown(ev, item.index)}
                          onClick={(ev) => ev.stopPropagation()}
                        />
                      </div>
                    );
                  }}
                </Show>
              )}
            </For>
          </div>
          <For each={rowV.getVirtualItems()}>
            {(row) => {
              const slot = createMemo(() => {
                const x = l();
                return x ? slotAt(x, e(), row.index) : null;
              });
              const deleted = () => {
                const s = slot();
                return s?.kind === "row" && e().deleted.has(s.rowIndex);
              };
              const isNew = () => slot()?.kind === "new";
              const conflict = () => {
                const s = slot();
                const f = props.m.failedChange;
                if (!s) return false;
                if (s.kind === "row") return props.m.conflictRow === s.rowIndex;
                return props.m.outcome?.kind === "conflict" && f?.kind === "insert" && f.newId === s.newId;
              };
              const rowErr = () => {
                const s = slot();
                return s ? props.errors.row.get(slotKey(s)) ?? null : null;
              };
              const rowState = () =>
                [deleted() ? tdata.sr.deleted : "", isNew() ? tdata.sr.new : "", rowErr() ? tdata.sr.error : ""].filter(Boolean).join(", ");
              return (
                <Show when={slot()}>
                  {(s) => (
                    <div
                      role="row"
                      aria-rowindex={row.index + 2}
                      class="absolute left-0"
                      classList={{
                        [TONE_DELETED]: deleted(),
                        [TONE_NEW]: isNew() && !conflict(),
                        [TONE_CONFLICT]: conflict(),
                      }}
                      style={{ top: `${row.start}px`, height: `${ROW_HEIGHT}px`, width: `${colV.getTotalSize()}px` }}
                    >
                      <div
                        role="rowheader"
                        class="sticky left-0 z-10 flex items-center justify-end gap-1 border-r border-border bg-panel px-2 text-muted"
                        classList={{ "text-danger": rowErr() !== null }}
                        title={rowErr()?.join("\n")}
                        style={{ width: `${rowNumWidth()}px`, height: `${ROW_HEIGHT}px` }}
                      >
                        <Show when={isNew()} fallback={<span>{s().kind === "row" ? row.index + 1 : ""}</span>}>
                          <span class="rounded bg-[color-mix(in_srgb,var(--srv-green)_25%,transparent)] px-1 font-sans text-[10px] text-fg">
                            {tdata.grid.newBadge}
                          </span>
                        </Show>
                        <Show when={rowState()}>
                          <span class="sr-only">{rowState()}</span>
                        </Show>
                      </div>
                      <For each={colV.getVirtualItems()}>
                        {(cv) => (
                          <Show when={columns()[cv.index]}>
                            {(col) => {
                              const v = createMemo(() => {
                                const x = l();
                                return x ? valueAt(x, e(), s(), col().name) : { value: { kind: "null" } as CellValue, changed: false };
                              });
                              const kind = createMemo(() => {
                                const x = l();
                                return x ? cellKind(x, e(), s(), col().name, props.errors.cell) : "normal";
                              });
                              const shown = createMemo(() => {
                                const x = v().value;
                                return displayCell(x.kind === "text" ? x.value : null);
                              });
                              const errs = () => props.errors.cell.get(`${slotKey(s())}:${col().name}`) ?? null;
                              const isFocus = () => {
                                const f = props.active;
                                return f !== null && f.r === row.index && f.c === cv.index;
                              };
                              const editable = () => {
                                const x = l();
                                return x !== null && props.blocked === null && editableAt(x, e(), s(), col().name).editable;
                              };
                              const errId = `${uid}-e-${row.index}-${cv.index}`;
                              return (
                                <div
                                  id={cellId(row.index, cv.index)}
                                  role="gridcell"
                                  aria-colindex={cv.index + 1}
                                  aria-selected={isFocus()}
                                  aria-readonly={!editable()}
                                  aria-invalid={errs() !== null ? true : undefined}
                                  aria-describedby={errs() !== null ? errId : undefined}
                                  data-r={row.index}
                                  data-c={cv.index}
                                  title={errs()?.join("\n") ?? (shown().truncated ? tdata.grid.truncated : undefined)}
                                  class="absolute top-0 overflow-hidden whitespace-nowrap border-r border-border/50 px-2"
                                  classList={{
                                    [TONE_CHANGED]: v().changed && !deleted(),
                                    "text-right": isNumericType(col().typeDisplay),
                                    "text-muted": kind() === "readonly",
                                    "line-through": deleted(),
                                    "text-danger ring-1 ring-inset ring-danger": kind() === "error",
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
                                  <Show
                                    when={v().value.kind === "text"}
                                    fallback={<span class="italic text-muted">{placeholder(v().value)}</span>}
                                  >
                                    {shown().text}
                                  </Show>
                                  <Show when={v().changed}>
                                    <span class="sr-only">{`, ${tdata.sr.changed}`}</span>
                                  </Show>
                                  <Show when={errs()}>
                                    {(list) => (
                                      <span id={errId} class="sr-only">
                                        {`${tdata.sr.error}: ${list().join("; ")}`}
                                      </span>
                                    )}
                                  </Show>
                                </div>
                              );
                            }}
                          </Show>
                        )}
                      </For>
                    </div>
                  )}
                </Show>
              );
            }}
          </For>
          <Show when={editing()}>
            {(ed) => {
              const col = () => columns()[ed().c];
              const box = () => ({
                left: `${colLeft(ed().c)}px`,
                top: `${HEADER_HEIGHT + ed().r * ROW_HEIGHT}px`,
                width: `${Math.max(widths()[ed().c] ?? MIN_COL_WIDTH, ed().multi ? AREA_MIN_WIDTH : 0)}px`,
                height: `${ed().multi ? AREA_HEIGHT : ROW_HEIGHT}px`,
              });
              const cls = "absolute z-[15] m-0 border border-accent bg-bg px-2 text-fg outline-none select-text";
              return (
                <Show
                  when={ed().multi}
                  fallback={
                    <input
                      ref={(el) => (editorEl = el)}
                      type="text"
                      spellcheck={false}
                      data-editor
                      aria-label={tdata.grid.editor(col()?.name ?? "")}
                      class={cls}
                      style={{ ...box(), font: FONT }}
                      value={draft()}
                      onInput={(ev) => setDraft(ev.currentTarget.value)}
                      onKeyDown={onEditorKey}
                      onBlur={() => finishEdit(true, undefined, false)}
                    />
                  }
                >
                  <textarea
                    ref={(el) => (editorEl = el)}
                    spellcheck={false}
                    data-editor
                    aria-label={tdata.grid.editor(col()?.name ?? "")}
                    class={`${cls} resize py-1`}
                    style={{ ...box(), font: FONT }}
                    value={draft()}
                    onInput={(ev) => setDraft(ev.currentTarget.value)}
                    onKeyDown={onEditorKey}
                    onBlur={() => finishEdit(true, undefined, false)}
                  />
                </Show>
              );
            }}
          </Show>
        </div>
        <Show when={props.m.loaded !== null && count() === 0}>
          <p class="sticky left-0 p-4 font-sans text-[13px] text-muted">{tdata.grid.empty}</p>
        </Show>
      </div>
      <p id={`${uid}-keys`} class="sr-only">
        {`${tdata.grid.keys}. ${tdata.sort.keyHint}`}
      </p>
      <div class="flex h-8 shrink-0 items-center gap-3 border-t border-border bg-panel px-3 font-sans text-[12px]">
        <span>{tdata.status.rows(l()?.rows.length ?? 0, props.m.more)}</span>
        <Show when={props.m.truncatedAtCap}>
          <span class="text-danger">{tdata.toolbar.limitReached}</span>
        </Show>
        <Show when={props.m.more && (l()?.rows.length ?? 0) < ROW_CAP}>
          <button
            type="button"
            class="ml-auto inline-flex h-6 items-center rounded border border-border bg-bg px-2.5 text-[12px] hover:bg-hover disabled:opacity-50"
            title={tdata.toolbar.moreTitle}
            disabled={props.m.loadingMore || props.m.applying || props.m.status !== "ready"}
            onClick={() => void props.data.loadMore(props.m.tabId)}
          >
            {props.m.loadingMore ? tdata.status.loading : tdata.toolbar.more}
          </button>
        </Show>
      </div>
    </div>
  );
}
