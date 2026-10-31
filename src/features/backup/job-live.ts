// Ekran okuyucu duyuruları (Plan 27): kalıcı `role="status"` / `role="alert"` bölgelerinin metni. Bölgeler her zaman bağlıdır;
// yalnız metin değişir (sonradan eklenen bölge güvenilir duyurulmaz). Günlük satırları duyurulmaz (aria-live="off").
import { tb } from "../../app/strings-backup";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import { resultView } from "./backup-model";

export interface LiveInput {
  phase: "loading" | "loadError" | "ready" | "running";
  loadError: string | null;
  closing: boolean;
  closeAsk: boolean;
  summary: JobSummary | null;
  mode: "backup" | "restore";
  singleTransaction: boolean;
  startError: string | null;
}

export function liveTexts(i: LiveInput): { status: string; alert: string } {
  const r = i.summary !== null && i.phase !== "running" ? resultView(i.summary, i.mode, i.singleTransaction) : null;
  const resultText = r ? [r.title, ...r.lines].join(" ") : "";
  const status = [i.phase === "loading" ? tb.loading : "", i.closing ? tb.closeWaiting : "", r && r.tone !== "error" ? resultText : ""]
    .filter(Boolean)
    .join(" ");
  const alert = [
    i.phase === "loadError" ? [tb.loadFailed, i.loadError ?? ""].filter(Boolean).join(" ") : "",
    i.closeAsk ? tb.closeAsk : "",
    r && r.tone === "error" ? resultText : "",
    i.startError ? `${tb.startError}: ${i.startError}` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return { status, alert };
}
