// İş görünümü (Plan 27; iki diyalogta ortak): komut önizlemesi, plan notları/hataları, ilerleme, günlük ve sonuç.
// Yalnız çizer. Günlük `aria-live="off"` bölgesidir (2000 satır duyurulmaz); sonuç duyurusu `job-live.ts` bölgelerindedir.
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { tb } from "../../app/strings-backup";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import { Button } from "../../ui/Button";
import { elapsedLabel, progressView, resultView, type JobLog, type ResultTone } from "./backup-model";

const TONE: Record<ResultTone, string> = {
  success: "border-accent",
  warning: "border-warn-text",
  error: "border-danger",
  neutral: "border-border",
};
const TITLE_TONE: Record<ResultTone, string> = { success: "", warning: "text-warn-text", error: "text-danger", neutral: "text-muted" };

export interface JobViewProps {
  mode: "backup" | "restore";
  running: boolean;
  plan: JobPlan | null;
  /** Plan bayat (sürüm ilerledi ya da istek sürüyor): önizleme soluk gösterilir. */
  stale: boolean;
  /** Alan satırı bulamayan plan hataları (araç, veritabanı, istek…). */
  general: string[];
  planError: string | null;
  startError: string | null;
  log: JobLog;
  startedAt: number | null;
  cancelRequested: boolean;
  singleTransaction: boolean;
  onCancel: () => void;
  /** Yalnız yedek başarısında verilir. */
  onReveal?: () => void;
}

export function JobView(props: JobViewProps) {
  const preview = () => (props.running ? (props.log.preview ?? props.plan?.preview) : props.plan?.preview) ?? "";
  const progress = () => progressView(props.log.progress, props.mode);

  // Geçen süre: yalnız çalışırken 1 sn'lik zamanlayıcı; efekt yeniden çalışınca ya da bileşen kalkınca temizlenir.
  const [now, setNow] = createSignal(Date.now());
  createEffect(() => {
    if (!props.running) return;
    setNow(Date.now());
    const h = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(h));
  });
  const elapsed = () => elapsedLabel(props.startedAt === null ? 0 : now() - props.startedAt);

  // Günlük: kullanıcı yukarı kaydırmadıysa sona yapışır.
  let logEl: HTMLDivElement | undefined;
  let stick = true;
  const onLogScroll = () => {
    if (logEl) stick = logEl.scrollTop + logEl.clientHeight >= logEl.scrollHeight - 8;
  };
  createEffect(() => {
    props.log.lines.length;
    if (stick && logEl) queueMicrotask(() => logEl && (logEl.scrollTop = logEl.scrollHeight));
  });
  createEffect(() => {
    if (props.running) stick = true;
  });

  const result = () => (props.log.summary && !props.running ? resultView(props.log.summary, props.mode, props.singleTransaction) : null);

  return (
    <div class="flex flex-col gap-3">
      <For each={props.general}>{(m) => <p class="text-danger">{m}</p>}</For>
      <Show when={props.planError}>
        {(e) => (
          <p class="text-danger">
            {tb.planError}: {e()}
          </p>
        )}
      </Show>
      <Show when={(props.plan?.notes.length ?? 0) > 0 && !props.running}>
        <div>
          <p class="mb-1 text-[12px] text-muted">{tb.notes}</p>
          <ul class="list-disc pl-5 text-[12px]">
            <For each={props.plan?.notes ?? []}>{(n) => <li>{n}</li>}</For>
          </ul>
        </div>
      </Show>
      <Show when={preview() !== "" || (props.stale && !props.running)}>
        <div class="flex flex-col gap-1">
          <p class="text-[12px] text-muted">{tb.preview}</p>
          <pre
            tabindex="0"
            aria-label={tb.preview}
            class={`max-h-32 overflow-auto whitespace-pre-wrap break-all rounded border border-border bg-panel px-2 py-1.5 font-mono text-[12px] ${
              props.stale && !props.running ? "opacity-60" : ""
            }`}
          >
            {preview() !== "" ? preview() : tb.previewPending}
          </pre>
        </div>
      </Show>
      <Show when={props.running}>
        <div class="flex flex-col gap-2">
          <div class="flex items-center justify-between gap-2 text-[12px]">
            <span>{progress().text}</span>
            <span class="text-muted">
              {tb.elapsed}: {elapsed()}
            </span>
          </div>
          <div
            role="progressbar"
            aria-label={tb.progressLabel}
            aria-valuemin={progress().percent === null ? undefined : 0}
            aria-valuemax={progress().percent === null ? undefined : 100}
            aria-valuenow={progress().percent ?? undefined}
            aria-valuetext={progress().percent === null ? progress().text : `${progress().percent}% — ${progress().text}`}
            class="h-2 overflow-hidden rounded bg-hover"
          >
            <div
              class={`h-full bg-accent ${progress().percent === null ? "w-full animate-pulse opacity-60" : ""}`}
              style={progress().percent === null ? undefined : { width: `${progress().percent}%` }}
            />
          </div>
        </div>
      </Show>
      <Show when={props.running || props.log.lines.length > 0}>
        <div class="flex flex-col gap-1">
          <p class="text-[12px] text-muted">{tb.log}</p>
          <div
            ref={logEl}
            role="log"
            aria-live="off"
            aria-label={tb.log}
            tabindex="0"
            onScroll={onLogScroll}
            class="max-h-64 overflow-y-auto rounded border border-border bg-panel px-2 py-1 font-mono text-[11px] leading-4"
          >
            <Show when={props.log.lines.length > 0} fallback={<span class="text-muted">{tb.logEmpty}</span>}>
              <For each={props.log.lines}>
                {(l) => (
                  <div
                    class={`whitespace-pre-wrap break-all ${
                      l.severity === "error" ? "text-danger" : l.severity === "warning" ? "text-warn-text" : ""
                    }`}
                  >
                    {l.text}
                  </div>
                )}
              </For>
            </Show>
          </div>
        </div>
      </Show>
      <Show when={props.running}>
        <div>
          <Button disabled={props.cancelRequested} onClick={props.onCancel}>
            {props.cancelRequested ? tb.cancelling : tb.cancel}
          </Button>
        </div>
      </Show>
      <Show when={props.startError}>
        {(e) => (
          <p class="rounded border border-danger px-3 py-2 text-danger">
            {tb.startError}: {e()}
          </p>
        )}
      </Show>
      <Show when={result()}>
        {(r) => (
          <div class={`flex flex-col gap-1 rounded border px-3 py-2 ${TONE[r().tone]}`}>
            <p class={`font-medium ${TITLE_TONE[r().tone]}`}>{r().title}</p>
            <For each={r().lines}>{(l) => <p class="text-[12px] text-muted">{l}</p>}</For>
            <Show when={props.onReveal && props.log.summary?.outputPath}>
              <div class="pt-1">
                <Button onClick={() => props.onReveal?.()}>{tb.reveal}</Button>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

/** Çalışırken kapatma onayı: diyalog içi şerit. "Devam et" açıldığında odaklanır. */
export function CloseAsk(props: { onCancelAndClose: () => void; onContinue: () => void }) {
  let continueEl: HTMLButtonElement | undefined;
  queueMicrotask(() => continueEl?.focus());
  return (
    <div class="flex items-center gap-2 rounded border border-danger px-3 py-2">
      <p class="flex-1 font-medium">{tb.closeAsk}</p>
      <Button class="!bg-danger !text-danger-fg" onClick={props.onCancelAndClose}>{tb.closeAndCancel}</Button>
      <Button ref={continueEl} onClick={props.onContinue}>{tb.closeContinue}</Button>
    </div>
  );
}

/** Enter başlatır: kapı açıksa, IME bileşimi sırasında değil, odak düğme/bağlantı/`textarea`/`select`'te değilken. */
export function enterToStart(canStart: () => boolean, start: () => void): JSX.EventHandler<HTMLDivElement, KeyboardEvent> {
  return (e) => {
    if (e.key !== "Enter" || e.isComposing || e.keyCode === 229 || e.shiftKey || e.ctrlKey || e.altKey) return;
    const tag = (e.target as HTMLElement).tagName;
    if (tag === "BUTTON" || tag === "SELECT" || tag === "TEXTAREA" || tag === "A") return;
    e.preventDefault();
    if (canStart()) start();
  };
}
