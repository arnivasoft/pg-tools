import { For } from "solid-js";
import { tstrip } from "./strings-strip";
import type { ResultState, StmtResult } from "./result-model";

const ICON: Record<StmtResult["status"], string> = { running: "…", ok: "✓", failed: "✗", cancelled: "⏹" };
const TONE: Record<StmtResult["status"], string> = {
  running: "text-muted",
  ok: "text-accent",
  failed: "text-danger",
  cancelled: "text-muted",
};

/** Satır sayısı / etkilenen satır: satır döndüren ifadede yüklenen, diğerlerinde `rowsAffected`. */
function countText(s: StmtResult): string {
  const c = tstrip;
  if (s.status === "running" || s.status === "failed" || s.status === "cancelled") {
    return s.columns.length > 0 ? c.rows(s.rows.length) : "";
  }
  if (s.columns.length > 0) return c.rows(s.rows.length);
  return s.rowsAffected != null ? c.affected(s.rowsAffected) : "";
}

/** İfade başına bir satır; tıklayınca `onSelect(index)` (ızgara o ifadeyi gösterir). */
export function ResultStrip(props: { result: ResultState | null; selected: number | null; onSelect: (index: number) => void }) {
  const c = tstrip;
  return (
    <ul aria-label={c.ariaLabel} class="flex max-h-28 flex-col overflow-auto border-b border-border text-xs">
      <For each={props.result?.stmts ?? []}>
        {(s) => (
          <li>
          <button
            type="button"
            aria-pressed={props.selected === s.index}
            class="flex w-full items-center gap-3 px-2 py-0.5 text-left hover:bg-hover aria-pressed:bg-hover"
            onClick={() => props.onSelect(s.index)}
          >
            <span class="w-6 text-right text-muted">#{s.index + 1}</span>
            <span class="w-20 truncate">{s.tag ?? c.kind[s.kind]}</span>
            <span class={`w-4 ${TONE[s.status]}`} title={c.status[s.status]}>
              <span aria-hidden="true">{ICON[s.status]}</span>
              <span class="sr-only">{c.status[s.status]}</span>
            </span>
            <span class="w-28 truncate">{countText(s)}</span>
            <span class="text-muted">{s.elapsedMs != null ? c.ms(s.elapsedMs) : ""}</span>
          </button>
          </li>
        )}
      </For>
    </ul>
  );
}
