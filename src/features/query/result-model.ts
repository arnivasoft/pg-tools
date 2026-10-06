// Sonuç modeli (saf): bir sekmenin son çalıştırmasının ifade sonuçları ve iletileri.
// Arka uç sözleşmesi: docs/query-tool-plan6-notes.md ("Olay akışı").
import type { Column } from "../../ipc/bindings/Column";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { NoticeInfo } from "../../ipc/bindings/NoticeInfo";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { StmtKind } from "../../ipc/bindings/StmtKind";

/** Arka ucun kullanıcı iptalinde (57014) gönderdiği ileti; iptal yalnızca bu iletiyle tanınır (ayrı kod yok). */
export const USER_CANCEL_MESSAGE = "Sorgu kullanıcı tarafından iptal edildi.";

/** Bir sekmede bellekte tutulan en çok satır; aşılınca en eski ifadelerin satırları boşaltılır. */
export const MAX_TAB_ROWS = 50_000;

/** "Daha fazla yükle" parça boyutu (arka uç `qt_more` ile aynı). */
export const LOAD_MORE_CHUNK = 1000;

export type StmtStatus = "running" | "ok" | "failed" | "cancelled";

/** `position`: ifade metnine göre 1 tabanlı karakter konumu; `charOffset`: ifadenin betikteki karakter ofseti. */
export type StmtError = ErrorPayload & { position: number | null; charOffset: number };

/** Tek bir ifadenin sonucu. `dropped`: satırları 50.000 sınırı yüzünden bellekten atıldı. */
export interface StmtResult {
  index: number;
  kind: StmtKind;
  columns: Column[];
  rows: (string | null)[][];
  /** Son `rows` olayının `more` bayrağı: imleçten daha fazla satır alınabilir. */
  more: boolean;
  /** `done.truncated`; imleç yolunda tam `limit` satırda da `true` olabilir. */
  truncated: boolean;
  charOffset: number;
  tag?: string;
  rowsAffected?: number | null;
  elapsedMs?: number;
  status: StmtStatus;
  error?: StmtError;
  dropped: boolean;
}

/** İleti paneli girdisi: ifade NOTICE'ı, hata, iptal ya da tamamlanma etiketi (`info`, ör. "INSERT 0 5"). */
export type Message =
  | { level: "notice"; stmtIndex: number; notice: NoticeInfo }
  | { level: "error" | "cancelled"; stmtIndex: number; error: StmtError }
  | { level: "info"; stmtIndex: number; tag: string; rowsAffected: number | null; elapsedMs: number };

/** Bir çalıştırmanın sonucu. `done` yalnızca `scriptDone` ile `true` olur (`invoke` çözülmesi bitti demek değildir). */
export interface ResultState {
  runId: number;
  stmts: StmtResult[];
  messages: Message[];
  done: boolean;
  stoppedOnError: boolean;
  /** Izgara imleci kapandı (ör. `qt_set_options`); "daha fazla" artık istenemez. */
  moreClosed: boolean;
}

export function emptyResult(runId: number): ResultState {
  return { runId, stmts: [], messages: [], done: false, stoppedOnError: false, moreClosed: false };
}

/** Hata kullanıcı iptali mi? (`statement_timeout` aynı kodla ama başka iletiyle gelir → iptal sayılmaz.) */
export function isUserCancel(error: ErrorPayload): boolean {
  return error.message === USER_CANCEL_MESSAGE;
}

export function totalRows(state: ResultState): number {
  let n = 0;
  for (const s of state.stmts) n += s.rows.length;
  return n;
}

function newStmt(index: number, kind: StmtKind, columns: Column[], charOffset: number): StmtResult {
  return { index, kind, columns, rows: [], more: false, truncated: false, charOffset, status: "running", dropped: false };
}

function upsert(state: ResultState, index: number, f: (s: StmtResult) => StmtResult, fallback?: () => StmtResult): StmtResult[] | null {
  const i = state.stmts.findIndex((s) => s.index === index);
  if (i < 0) {
    if (!fallback) return null;
    return [...state.stmts, f(fallback())];
  }
  const next = state.stmts.slice();
  next[i] = f(state.stmts[i]);
  return next;
}

/** Sınır aşılırsa `keep` dışındaki en eski ifadelerin satırlarını boşaltır. */
function enforceCap(stmts: StmtResult[], keep: number): StmtResult[] {
  let total = 0;
  for (const s of stmts) total += s.rows.length;
  if (total <= MAX_TAB_ROWS) return stmts;
  const next = stmts.slice();
  for (let i = 0; i < next.length && total > MAX_TAB_ROWS; i++) {
    const s = next[i];
    if (s.index === keep || s.rows.length === 0) continue;
    total -= s.rows.length;
    next[i] = { ...s, rows: [], dropped: true };
  }
  return next;
}

/**
 * Bir `QueryEvent`'i duruma uygular. `runId` durumun çalıştırmasıyla eşleşmezse (eski çalıştırmadan geç gelen
 * olay) durum **aynen** döner. `qt_more` aynı çalıştırmanın kimliğiyle `rows` olayları gönderir; bunlar
 * `scriptDone` sonrasında da eklenir. Bilinmeyen ifadeye gelen `rows`/`done` yok sayılır; `failed` ise (ör. ifadeler
 * arası iptal) `started` olmadan da bir ifade kaydı oluşturur.
 */
export function applyEvent(state: ResultState, runId: number, ev: QueryEvent): ResultState {
  if (runId !== state.runId) return state;
  // `scriptDone` sonrası yalnızca `qt_more`'un `rows` olayları gelebilir.
  if (state.done && ev.type !== "rows") return state;
  switch (ev.type) {
    case "started": {
      const stmts = upsert(
        state,
        ev.stmtIndex,
        () => newStmt(ev.stmtIndex, ev.kind, ev.columns, ev.charOffset),
        () => newStmt(ev.stmtIndex, ev.kind, ev.columns, ev.charOffset),
      );
      return stmts ? { ...state, stmts } : state;
    }
    case "rows": {
      const stmts = upsert(state, ev.stmtIndex, (s) => ({
        ...s,
        rows: ev.rows.length ? s.rows.concat(ev.rows) : s.rows,
        more: ev.more,
      }));
      return stmts ? { ...state, stmts: enforceCap(stmts, ev.stmtIndex) } : state;
    }
    case "done": {
      const stmts = upsert(state, ev.stmtIndex, (s) => ({
        ...s,
        status: "ok",
        tag: ev.tag,
        rowsAffected: ev.rowsAffected,
        elapsedMs: ev.elapsedMs,
        truncated: ev.truncated,
      }));
      if (!stmts) return state;
      const messages: Message[] = state.messages.concat(
        ev.notices.map((notice) => ({ level: "notice" as const, stmtIndex: ev.stmtIndex, notice })),
        [{ level: "info", stmtIndex: ev.stmtIndex, tag: ev.tag, rowsAffected: ev.rowsAffected, elapsedMs: ev.elapsedMs }],
      );
      return { ...state, stmts, messages };
    }
    case "failed": {
      const error: StmtError = { ...ev.error, position: ev.position, charOffset: ev.charOffset };
      const status: StmtStatus = isUserCancel(ev.error) ? "cancelled" : "failed";
      const stmts = upsert(
        state,
        ev.stmtIndex,
        (s) => ({ ...s, status, error, more: false }),
        () => newStmt(ev.stmtIndex, "other", [], ev.charOffset),
      )!;
      const messages: Message[] = state.messages.concat(
        ev.notices.map((notice) => ({ level: "notice" as const, stmtIndex: ev.stmtIndex, notice })),
        [{ level: status === "cancelled" ? "cancelled" : "error", stmtIndex: ev.stmtIndex, error }],
      );
      return { ...state, stmts, messages };
    }
    case "scriptDone":
      return { ...state, done: true, stoppedOnError: ev.stoppedOnError };
  }
}

/** Izgara imleci kapandı (`qt_set_options`, ya da `qt_more` hata verdi): "daha fazla" kapanır. */
export function closeMore(state: ResultState): ResultState {
  return state.moreClosed ? state : { ...state, moreClosed: true };
}

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
