import { Lock, Play, PlayCircle, RotateCcw, Square, Check, Undo2 } from "lucide-solid";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import { t } from "../../app/strings";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TxMode } from "../../ipc/bindings/TxMode";
import { Button } from "../../ui/Button";
import { isProductionLabel } from "../servers/server-form";
import type { TabModel } from "./query-controller";
import { canCancel, canCommit, canRollback } from "./run-model";
import { TIMEOUT_CHOICES, formatElapsed, serverLineColor } from "./workspace-model";

const selectClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:opacity-50";

export function ContextBar(props: {
  model: TabModel;
  config: ServerConfig | null;
  onRun: () => void;
  onRunAll: () => void;
  onCancel: () => void;
  onCommit: () => void;
  onRollback: () => void;
  onMode: (mode: TxMode) => void;
  onTimeout: (ms: number) => void;
  onReconnect: () => void;
}) {
  const c = t.query.ctx;
  const m = () => props.model;
  const production = () => isProductionLabel(props.config?.label);
  const line = () => serverLineColor(props.config);
  const status = () => m().run.status;
  const running = () => status() === "running" || status() === "cancelling";
  const busy = () => running() || status() === "opening" || m().op !== null;
  const txOpen = () => m().run.tx !== "idle";
  const canStart = () => !busy() && !m().serverGone && status() !== "opening";

  // Açık transaction süre sayacı (yalnızca açıkken saniyede bir).
  const [now, setNow] = createSignal(Date.now());
  // Yalnızca transaction başlangıcına bağlı (akan satır olayları zamanlayıcıyı yeniden kurmasın).
  const since = createMemo(() => m().txSince);
  createEffect(() => {
    if (since() === null) return;
    setNow(Date.now());
    const h = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(h));
  });

  const statusText = () => {
    if (m().serverGone) return c.serverGone;
    switch (status()) {
      case "noSession":
        return c.noSession;
      case "opening":
        return c.opening;
      case "running":
        return c.running;
      case "cancelling":
        return c.cancelling;
      case "lost":
        return c.lost;
      default:
        return "";
    }
  };

  return (
    <div role="toolbar" aria-label={c.ariaLabel} class="shrink-0 border-b border-border bg-panel">
      <Show when={line()}>
        {(color) => <div aria-hidden="true" style={{ height: production() ? "3px" : "2px", background: color() }} />}
      </Show>
      <div class="flex min-h-9 flex-wrap items-center gap-1.5 px-2 py-1 text-[12px]">
        <Show when={production()}>
          <span class="rounded px-1.5 font-semibold text-white" style={{ background: line() ?? "var(--danger)" }}>
            {props.config?.label}
          </span>
        </Show>
        <Show when={!production() && props.config?.label}>
          <span class="rounded border border-border px-1.5 text-muted">{props.config?.label}</span>
        </Show>
        <span class="max-w-48 truncate font-semibold" title={m().serverName}>
          {m().serverName}
        </span>
        <span class="text-muted">/</span>
        <span class="max-w-40 truncate" title={m().database ?? ""}>
          {m().database}
        </span>
        <Show when={props.config?.readOnlyLock}>
          <Lock size={12} class="shrink-0 text-muted" role="img" aria-label={c.locked} />
        </Show>
        <div class="mx-1 h-5 w-px bg-border" aria-hidden="true" />
        <select
          class={selectClass}
          aria-label={c.mode}
          title={txOpen() ? c.modeLocked : c.mode}
          value={m().options.mode}
          disabled={txOpen() || busy() || m().serverGone}
          onChange={(e) => props.onMode(e.currentTarget.value as TxMode)}
        >
          <option value="autocommit">{c.modes.autocommit}</option>
          <option value="manual">{c.modes.manual}</option>
        </select>
        <select
          class={selectClass}
          aria-label={c.timeout}
          title={c.timeout}
          value={String(m().options.statementTimeoutMs)}
          disabled={busy() || m().serverGone}
          onChange={(e) => props.onTimeout(Number(e.currentTarget.value))}
        >
          <For each={[...new Set([...TIMEOUT_CHOICES, m().options.statementTimeoutMs])]}>
            {(ms) => <option value={String(ms)}>{c.timeoutLabel(ms)}</option>}
          </For>
        </select>
        <div class="mx-1 h-5 w-px bg-border" aria-hidden="true" />
        <Button class="h-7 text-[12px]" variant="primary" disabled={!canStart()} title={c.runHint} onClick={() => props.onRun()}>
          <Play size={13} /> {c.run}
        </Button>
        <Button class="h-7 text-[12px]" disabled={!canStart()} title={c.runAllHint} onClick={() => props.onRunAll()}>
          <PlayCircle size={13} /> {c.runAll}
        </Button>
        <Button class="h-7 text-[12px]" disabled={!canCancel(m().run)} title={c.cancelHint} onClick={() => props.onCancel()}>
          <Square size={12} /> {c.cancel}
        </Button>
        <Show when={m().options.mode === "manual" || txOpen()}>
          <Button class="h-7 text-[12px]" disabled={!canCommit(m().run) || m().op !== null} onClick={() => props.onCommit()}>
            <Check size={13} /> {c.commit}
          </Button>
          <Button class="h-7 text-[12px]" disabled={!canRollback(m().run) || m().op !== null} onClick={() => props.onRollback()}>
            <Undo2 size={13} /> {c.rollback}
          </Button>
        </Show>
        <Show when={m().run.tx === "inTransaction"}>
          {/* Saniyede bir değişir: canlı bölge değil (ekran okuyucu her tıkta okumasın). */}
          <span aria-live="off" class="rounded bg-selected px-1.5 py-0.5 font-medium">
            {c.openTx(formatElapsed(now() - (since() ?? now())))}
          </span>
        </Show>
        <Show when={m().run.tx === "failed"}>
          <span role="status" class="rounded bg-danger px-1.5 py-0.5 font-medium text-white">
            {c.failedTx}
          </span>
        </Show>
        <div class="flex-1" />
        <Show when={statusText()}>
          <span role="status" aria-live="polite" class="text-muted" classList={{ "text-danger": status() === "lost" || m().serverGone }}>
            {statusText()}
          </span>
        </Show>
        <Show when={status() === "lost" && !m().serverGone}>
          <Button class="h-7 text-[12px]" onClick={() => props.onReconnect()}>
            <RotateCcw size={13} /> {c.reconnect}
          </Button>
        </Show>
      </div>
    </div>
  );
}
