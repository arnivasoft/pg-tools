// Sekme çalıştırma durum makinesi (saf): oturum durumu, transaction durumu ve son çalıştırmanın sonucu.
// Arka uç sözleşmesi: docs/query-tool-plan6-notes.md.
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { TxState } from "../../ipc/bindings/TxState";
import { applyEvent, canLoadMore, closeMore, emptyResult, type ResultState } from "./result-model";

/**
 * - `noSession`: oturum yok (henüz açılmadı ya da açılamadı)
 * - `opening`: `qt_open` sürüyor
 * - `idle`: oturum hazır
 * - `running`: `qt_run` sürüyor (`scriptDone` gelene kadar)
 * - `cancelling`: iptal istendi, `scriptDone` bekleniyor
 * - `lost`: oturum kayboldu (`not_found` vb.); yeniden açılmalı
 */
export type TabStatus = "noSession" | "opening" | "idle" | "running" | "cancelling" | "lost";
export type TxView = TxState;

export interface RunState {
  status: TabStatus;
  tx: TxView;
  /** Son başlatılan çalıştırmanın kimliği (0 = hiç çalıştırılmadı). */
  runId: number;
  /** Son çalıştırmanın sonucu; yalnızca `runId`'ye ait olaylar uygulanır. */
  result: ResultState | null;
}

export type RunEvent =
  | { type: "open" }
  | { type: "opened"; tx?: TxView }
  | { type: "openFailed" }
  | { type: "runStart"; runId: number }
  /** Kanal olayı; `scriptDone` çalıştırmayı bitirir. Başka `runId`'ye ait olaylar yok sayılır. */
  | { type: "event"; runId: number; event: QueryEvent }
  /** `qt_run` hiç olay göndermeden reddedildi (`busy`, `confirmation_required`, `session_reset`, `locked` …). */
  | { type: "runRejected"; runId: number; error: ErrorPayload }
  | { type: "cancelRequested" }
  | { type: "txChanged"; tx: TxView }
  | { type: "sessionLost" }
  /** `qt_set_options` ızgara imlecini kapattı → "daha fazla" kapanır. */
  | { type: "optionsChanged" };

export function initialRun(): RunState {
  return { status: "noSession", tx: "idle", runId: 0, result: null };
}

const isRunning = (s: TabStatus) => s === "running" || s === "cancelling";

/** Saf geçiş fonksiyonu; geçersiz geçişlerde durum aynen döner. */
export function runReduce(state: RunState, ev: RunEvent): RunState {
  switch (ev.type) {
    case "open":
      return state.status === "noSession" || state.status === "lost" ? { ...state, status: "opening" } : state;
    case "opened":
      return state.status === "opening" ? { ...state, status: "idle", tx: ev.tx ?? "idle" } : state;
    case "openFailed":
      return state.status === "opening" ? { ...state, status: "noSession", tx: "idle" } : state;
    case "runStart":
      if (state.status !== "idle" || ev.runId <= state.runId) return state;
      return { ...state, status: "running", runId: ev.runId, result: emptyResult(ev.runId) };
    case "event": {
      if (ev.runId !== state.runId || !state.result) return state;
      const result = applyEvent(state.result, ev.runId, ev.event);
      const ended = ev.event.type === "scriptDone" && isRunning(state.status);
      if (result === state.result && !ended) return state;
      return { ...state, result, status: ended ? "idle" : state.status };
    }
    case "runRejected":
      if (ev.runId !== state.runId || !isRunning(state.status)) return state;
      return { ...state, status: "idle" };
    case "cancelRequested":
      return state.status === "running" ? { ...state, status: "cancelling" } : state;
    case "txChanged":
      return state.tx === ev.tx ? state : { ...state, tx: ev.tx };
    case "sessionLost":
      return { ...state, status: "lost", tx: "idle" };
    case "optionsChanged":
      return state.result ? { ...state, result: closeMore(state.result) } : state;
  }
}

export function canRun(s: RunState): boolean {
  return s.status === "idle";
}

/** İptal her basışta yeniden gönderilir (PG arada geleni yok sayabilir); `cancelling`'de de basılabilir. */
export function canCancel(s: RunState): boolean {
  return isRunning(s.status);
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

/** Açık ya da hatalı transaction varken sekme kapanışı onay ister. */
export function closeNeedsConfirm(s: RunState): boolean {
  return s.status !== "lost" && (s.tx === "inTransaction" || s.tx === "failed");
}

export type CloseChoice = "commit" | "rollback" | "cancel";

/**
 * Kapanış diyaloğu seçenekleri (`cancel` = sekmeyi kapatma). `failed` → yalnızca `rollback`/`cancel`;
 * çalışırken `commit` sunulmaz (meşgul oturumda `busy` döner). Onay gerekmiyorsa boş.
 */
export function closeChoices(s: RunState): CloseChoice[] {
  if (!closeNeedsConfirm(s)) return [];
  if (s.tx === "failed" || isRunning(s.status)) return ["rollback", "cancel"];
  return ["commit", "rollback", "cancel"];
}
