import { For, type JSX } from "solid-js";
import { CELL_FORMATS, nextTab, type Availability, type CellFormat } from "./cell-view-model";
import { tcell } from "./strings-cell";

/** Erişilebilir sekme çubuğu: tek tabindex=0 (seçili), oklar yalnız odağı taşır, Enter/Space/tıklama etkin sekmeyi seçer. */
export function CellTabs(props: {
  value: CellFormat;
  avail: Record<CellFormat, Availability>;
  onSelect: (f: CellFormat) => void;
  idBase: string;
}): JSX.Element {
  const tabId = (f: CellFormat) => `${props.idBase}-tab-${f}`;
  const select = (f: CellFormat) => {
    if (props.avail[f].enabled) props.onSelect(f);
  };
  const onKeyDown = (e: KeyboardEvent, f: CellFormat) => {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      document.getElementById(tabId(nextTab(f, e.key)))?.focus();
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      select(f);
    }
  };
  return (
    <div
      role="tablist"
      aria-label={tcell.ariaLabel}
      class="flex h-7 shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border bg-panel px-2 font-sans text-[12px]"
    >
      <For each={CELL_FORMATS}>
        {(f) => {
          const a = () => props.avail[f];
          return (
            <button
              type="button"
              role="tab"
              id={tabId(f)}
              aria-selected={props.value === f}
              aria-controls={`${props.idBase}-panel`}
              aria-disabled={a().enabled ? undefined : "true"}
              tabindex={props.value === f ? 0 : -1}
              title={a().reason ?? undefined}
              class="shrink-0 rounded px-2 py-0.5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
              classList={{
                "bg-bg font-semibold": props.value === f,
                "opacity-50 cursor-not-allowed": !a().enabled,
                "hover:bg-bg": a().enabled,
              }}
              onClick={() => select(f)}
              onKeyDown={(e) => onKeyDown(e, f)}
            >
              {tcell.tabs[f]}
              {a().reason ? <span class="sr-only"> — {a().reason}</span> : null}
            </button>
          );
        }}
      </For>
    </div>
  );
}
