// Fark listesi: sanallaştırılmış `role="listbox"` (çoklu seçim). Satır = seçim kutusu, tür, ad, durum, risk rozeti ve
// not/hata rozeti. Klavye: ↑/↓ odağı taşır, Home/End uçlara gider, Boşluk odaktaki öğenin seçimini değiştirir. Odak ve
// seçim durumu denetleyicidedir; bu bileşen yalnız çizer ve olayları iletir.
import { createVirtualizer } from "@tanstack/solid-virtual";
import { CircleAlert, Info } from "lucide-solid";
import { For, Show, createEffect, createUniqueId, on, untrack } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import { RiskBadge } from "../designer/RiskBadge";
import type { DiffController } from "./diff-open";
import { itemLabel, kindLabel, problemText } from "./diff-model";
import { ROW_HEIGHT, noteBadge, rowAria, statusText } from "./diff-view-model";

export function DiffList(props: {
  diff: DiffController;
  tabId: string;
  items: readonly DiffItem[];
  selected: ReadonlySet<string>;
  focused: string | null;
}) {
  const uid = createUniqueId();
  let scrollEl!: HTMLDivElement;
  const indexOf = (k: string | null) => (k === null ? -1 : props.items.findIndex((i) => i.key === k));
  const rowId = (i: number) => `${uid}-r${i}`;

  const v = createVirtualizer({
    get count() {
      return props.items.length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  // Odak satırı görünür alana kaydırılır (yalnız odak anahtarı değişince; liste yenilemesi görünümü geri çekmez).
  createEffect(
    on(
      () => props.focused,
      (k) => {
        const i = untrack(() => indexOf(k));
        if (i >= 0) v.scrollToIndex(i, { align: "auto" });
      },
      { defer: true },
    ),
  );

  const onKeyDown = (e: KeyboardEvent) => {
    const id = props.tabId;
    switch (e.key) {
      case "ArrowDown":
        props.diff.moveFocus(id, 1);
        break;
      case "ArrowUp":
        props.diff.moveFocus(id, -1);
        break;
      case "Home":
        props.diff.moveFocus(id, "home");
        break;
      case "End":
        props.diff.moveFocus(id, "end");
        break;
      case " ":
        if (props.focused === null) return;
        props.diff.toggle(id, props.focused);
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  const activeId = () => {
    const i = indexOf(props.focused);
    return i >= 0 ? rowId(i) : undefined;
  };

  return (
    <div
      ref={scrollEl}
      role="listbox"
      aria-multiselectable="true"
      aria-label={tsd.aria.list}
      aria-activedescendant={activeId()}
      tabIndex={0}
      class="relative min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent"
      onKeyDown={onKeyDown}
    >
      <div style={{ height: `${v.getTotalSize()}px`, position: "relative" }}>
        <For each={v.getVirtualItems()}>
          {(row) => (
            <Show when={props.items[row.index]}>
              {(it) => {
                const sel = () => props.selected.has(it().key);
                const foc = () => props.focused === it().key;
                const aria = () => rowAria(it(), sel(), foc());
                const badge = () => noteBadge(it());
                const reasonId = `${uid}-n${row.index}`;
                const reason = () => `${tsd.notSelectable}${problemText(it()) ?? ""}`;
                return (
                  <div
                    role="option"
                    id={rowId(row.index)}
                    aria-selected={aria().selected}
                    aria-label={aria().label}
                    class="absolute left-0 right-0 flex cursor-default items-center gap-2 px-2 text-[12px] hover:bg-hover"
                    classList={{ "bg-selected": foc() }}
                    style={{ top: `${row.start}px`, height: `${ROW_HEIGHT}px` }}
                    onMouseDown={(e) => {
                      // DOM odağı listbox'ta kalır (satırlar sanal; odaklanan satır kaldırılınca odak body'ye düşerdi).
                      e.preventDefault();
                      scrollEl.focus({ preventScroll: true });
                    }}
                    onClick={() => props.diff.focus(props.tabId, it().key)}
                  >
                    <input
                      type="checkbox"
                      tabIndex={-1}
                      class="shrink-0"
                      checked={sel()}
                      disabled={!it().selectable}
                      title={it().selectable ? undefined : reason()}
                      aria-label={tsd.aria.select(itemLabel(it()))}
                      aria-describedby={it().selectable ? undefined : reasonId}
                      onClick={(e) => e.stopPropagation()}
                      onChange={() => {
                        props.diff.toggle(props.tabId, it().key);
                        props.diff.focus(props.tabId, it().key);
                      }}
                    />
                    <Show when={!it().selectable}>
                      <span id={reasonId} class="sr-only">
                        {reason()}
                      </span>
                    </Show>
                    <span class="w-28 shrink-0 truncate text-muted">{kindLabel(it().kind)}</span>
                    <span class="min-w-0 flex-1 truncate font-mono" title={itemLabel(it())}>
                      {itemLabel(it())}
                    </span>
                    <span class="w-28 shrink-0 truncate text-muted" classList={{ "text-danger": it().problem !== null }}>
                      {statusText(it())}
                    </span>
                    <RiskBadge risk={it().risk} />
                    <span class="flex w-9 shrink-0 items-center justify-end gap-0.5">
                      <Show when={badge()}>
                        {(b) => (
                          <span
                            class="inline-flex items-center gap-0.5"
                            classList={{ "text-danger": b().tone === "error", "text-muted": b().tone === "info" }}
                            title={b().title}
                          >
                            <Show when={b().tone === "error"} fallback={<Info size={12} aria-hidden="true" />}>
                              <CircleAlert size={12} aria-hidden="true" />
                            </Show>
                            {b().count}
                          </span>
                        )}
                      </Show>
                    </span>
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
