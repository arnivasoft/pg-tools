// QueryTab parçasının (QueryTab.tsx, ContextBar.tsx, ResultGrid.tsx) saf yardımcıları: yalnız bu tembel parça
// içe aktarır; ana paket modüllerinden (`workspace-model.ts`, `run-model.ts`, `result-model.ts`) ayrı tutulur ki
// ana pakete bayt eklemesin.
import type { Scope } from "../../ipc/bindings/Scope";
import { MAX_TAB_ROWS, type ResultState } from "./result-model";
import type { RunState } from "./run-model";

export const TIMEOUT_CHOICES = [30_000, 300_000, 0] as const;

export function sameScope(a: Scope | null, b: Scope | null): boolean {
  return a !== null && b !== null && JSON.stringify(a) === JSON.stringify(b);
}

// ---------------------------------------------------------------- Transaction süresi

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
}

// ---------------------------------------------------------------- Sekme ↔ editör durumu

export interface SavedView<S> {
  state: S;
  scrollTop: number;
  scrollLeft: number;
}

/**
 * Tek editör, sekme başına bir durum. `from` sekmesinin anlık durumu kaydedilir; `to` için kayıtlı durum ya da
 * `create()` döner. Aynı sekmeye dönüşte yalnızca kaydırma geri yüklenir (durum değişmez).
 */
export function swapEditorState<S>(
  book: Map<string, SavedView<S>>,
  from: string | null,
  current: SavedView<S> | null,
  to: string,
  create: () => S,
): SavedView<S> {
  if (from !== null && current !== null) book.set(from, current);
  const saved = book.get(to);
  if (saved) return saved;
  const fresh = { state: create(), scrollTop: 0, scrollLeft: 0 };
  book.set(to, fresh);
  return fresh;
}

// ---------------------------------------------------------------- Çalıştırma düğmeleri

/** İptal her basışta yeniden gönderilir (PG arada geleni yok sayabilir); `cancelling`'de de basılabilir. */
export function canCancel(s: RunState): boolean {
  return s.status === "running" || s.status === "cancelling";
}

/** `Failed` transaction'da COMMIT reddedilir (`tx_failed`). */
export function canCommit(s: RunState): boolean {
  return s.status === "idle" && s.tx === "inTransaction";
}

export function canRollback(s: RunState): boolean {
  return s.status === "idle" && (s.tx === "inTransaction" || s.tx === "failed");
}

/** "Daha fazla" düğmesi: oturum boşta ve ifade son çalıştırmada `canLoadMore` koşullarını sağlıyor. */
export function canLoadMoreRows(s: RunState, stmtIndex: number): boolean {
  return s.status === "idle" && !!s.result && s.result.runId === s.runId && canLoadMore(s.result, stmtIndex);
}

// ---------------------------------------------------------------- "Daha fazla yükle"

/** "Daha fazla yükle" parça boyutu (arka uç `qt_more` ile aynı). */
export const LOAD_MORE_CHUNK = 1000;

/**
 * Arka uç her ifadeden önce önceki ızgara imlecini kapatır; açık kalabilecek tek imleç çalıştırmanın **son**
 * ifadesinindir. "Daha fazla" yalnızca: çalıştırma bitti (`scriptDone`), imleç kapatılmadı (`closeMore`), ifade
 * son ifade, `ok`, `more`, satırları atılmadı ve kendi satırları sınırın altında. Yeni çalıştırma yeni bir
 * `ResultState` (`emptyResult`) başlattığından eski ızgaralar için kendiliğinden false.
 */
export function canLoadMore(state: ResultState, stmtIndex: number): boolean {
  if (!state.done || state.moreClosed) return false;
  const last = state.stmts[state.stmts.length - 1];
  if (!last || last.index !== stmtIndex || last.status !== "ok") return false;
  return last.more && !last.dropped && last.rows.length < MAX_TAB_ROWS;
}
