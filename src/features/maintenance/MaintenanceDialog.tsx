// "Bakım…" diyaloğu (tembel parça; Plan 25). Durum `maintenance-controller`dadır; bu dosya yalnız çizer.
// Ana modülleri (`app/strings`, `explorer/*`, `tree-model`, `designer/*`) içe aktarmaz: metin `strings-maintenance`tan,
// api/ağaç/bildirim App'ten props ile gelir. Önizleme salt okunur `<pre>`dir (CodeMirror yok).
// Klavye: Enter uygular (kapı ve onay tamamsa; IME bileşimi sırasında değil), Esc kapatır (çalışırken iptal onayı sorar).
import { createEffect, createSignal, createUniqueId, For, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { phaseLabel, tm } from "../../app/strings-maintenance";
import type { ToastKind } from "../../app/toast-model";
import type { MaintenanceOp } from "../../ipc/bindings/MaintenanceOp";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { inputClass } from "../../ui/Field";
import {
  createMaintenanceController,
  LOCK_TIMEOUTS,
  type MaintenanceController,
  type MaintenanceControllerDeps,
  type MaintenanceResult,
} from "./maintenance-controller";
import { elapsedLabel, parseVersionNum, progressView, splitFieldErrors, type OptionState } from "./maintenance-model";

export interface MaintenanceDialogDeps {
  api: MaintenanceControllerDeps["api"];
  /** Explorer ağacı: sunucu düğümünün `detail`'i (`ServerInfo.version`) sürüm, `flags.locked` kilit kaynağıdır. */
  tree: () => { nodes: Map<string, TreeNode> };
  notify: (text: string, kind: ToastKind) => void;
}

type BoolKey = "full" | "freeze" | "analyze" | "verbose" | "disablePageSkipping" | "skipLocked" | "concurrently";
const BOOLS: Record<MaintenanceOp["op"], BoolKey[]> = {
  vacuum: ["full", "freeze", "analyze", "verbose", "disablePageSkipping", "skipLocked"],
  analyze: ["verbose", "skipLocked"],
  reindex: ["concurrently", "verbose"],
  cluster: ["verbose"],
};

/** Sunucu düğümünden sürüm ve kilit (bağlı değilse sürüm bilinmez → istemci kapısı yok). */
function serverInfoFrom(tree: MaintenanceDialogDeps["tree"], serverId: string) {
  for (const n of tree().nodes.values()) {
    if (n.kind === "server" && n.serverId === serverId) {
      return { versionNum: n.flags.connected ? parseVersionNum(n.detail) : null, locked: n.flags.locked };
    }
  }
  return null;
}

export function MaintenanceDialog(props: { nodeId: string | null; deps: MaintenanceDialogDeps; onClose: () => void }) {
  const deps = untrack(() => props.deps);
  const ctl = createMaintenanceController({
    api: deps.api,
    serverInfo: (id) => serverInfoFrom(deps.tree, id),
    notify: deps.notify,
    onClosed: () => props.onClose(),
    newId: () => crypto.randomUUID(),
    now: () => Date.now(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  });
  createEffect(
    on(
      () => props.nodeId,
      (id) => {
        if (id !== null) ctl.open(id);
        else if (ctl.state()) ctl.close();
      },
    ),
  );
  onCleanup(() => ctl.close());

  const s = () => ctl.state();
  const applying = () => s()?.phase === "applying";
  const [now, setNow] = createSignal(Date.now());
  createEffect(() => {
    if (!applying()) return;
    setNow(Date.now());
    const h = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(h));
  });

  const title = () => {
    const t = s()?.target;
    return t ? tm.titleFor(t.schema ? `${t.schema}.${t.name}` : t.name) : tm.title;
  };
  const fieldErrors = () => splitFieldErrors(s()?.view?.errors ?? []);
  const gateReason = () => {
    const g = ctl.gate();
    if (!g.ok) return tm.blocked[g.reason];
    return g.confirm && !s()?.confirmed ? tm.blocked.confirm : undefined;
  };
  const onKeyDown: JSX.EventHandler<HTMLDivElement, KeyboardEvent> = (e) => {
    if (e.key !== "Enter" || e.isComposing || e.keyCode === 229 || e.shiftKey || e.ctrlKey || e.altKey) return;
    const tag = (e.target as HTMLElement).tagName;
    if (tag === "BUTTON" || tag === "SELECT" || tag === "TEXTAREA" || tag === "A") return;
    e.preventDefault();
    if (ctl.canApply()) void ctl.apply();
  };
  let noEl: HTMLButtonElement | undefined;
  createEffect(() => {
    if (s()?.closeAsk) queueMicrotask(() => noEl?.focus());
  });

  return (
    <Dialog
      open={props.nodeId !== null && s() !== null}
      onOpenChange={(o) => {
        if (o) return;
        if (s()?.closeAsk) ctl.answerClose(false);
        else ctl.requestClose();
      }}
      title={title()}
      width="640px"
      footer={
        <>
          <Show
            when={applying()}
            fallback={<Button onClick={() => ctl.requestClose()}>{tm.close}</Button>}
          >
            <Button disabled={s()?.cancelRequested} onClick={() => ctl.cancelApply()}>
              {s()?.cancelRequested ? tm.cancelling : tm.cancel}
            </Button>
          </Show>
          <Button
            variant="primary"
            disabled={!ctl.canApply()}
            title={gateReason()}
            onClick={() => ctl.canApply() && void ctl.apply()}
          >
            {s()?.result ? tm.rerun : tm.apply}
          </Button>
        </>
      }
    >
      <Show when={s()}>
        {(cur) => (
          <div class="flex flex-col gap-3 text-[13px]" onKeyDown={onKeyDown}>
            <Show when={cur().locked}>
              <p role="alert" class="text-danger">{tm.locked}</p>
            </Show>
            <Show when={cur().phase === "loading"}>
              <p role="status" class="text-muted">{tm.loading}</p>
            </Show>
            <Show when={cur().phase === "loadError"}>
              <div role="alert" class="flex flex-col gap-2 rounded border border-danger px-3 py-2">
                <p class="font-medium text-danger">{tm.loadFailed}</p>
                <p class="text-muted">{cur().loadError}</p>
                <Show when={cur().loadError !== tm.noTarget}>
                  <div>
                    <Button onClick={() => ctl.retry()}>{tm.retry}</Button>
                  </div>
                </Show>
              </div>
            </Show>
            <Show when={cur().notice}>
              <p role="status" class="rounded border border-border px-3 py-2 text-warn-text">{cur().notice}</p>
            </Show>
            <Show when={cur().closeAsk}>
              <div role="alert" class="flex items-center gap-2 rounded border border-danger px-3 py-2">
                <p class="flex-1 font-medium">{tm.closeAsk}</p>
                <Button ref={noEl} onClick={() => ctl.answerClose(false)}>{tm.closeNo}</Button>
                <Button class="!bg-danger !text-danger-fg" onClick={() => ctl.answerClose(true)}>{tm.closeYes}</Button>
              </div>
            </Show>
            <Show when={cur().closing}>
              <p role="status" class="text-muted">{tm.closeWaiting}</p>
            </Show>
            <Show when={cur().target}>
              {(target) => (
                <>
                  <Show when={!target().editable}>
                    <div role="alert" class="rounded border border-danger px-3 py-2">
                      <p class="font-medium">{tm.unsupported}</p>
                      <ul class="list-disc pl-5">
                        <For each={target().unsupported}>{(u) => <li>{u}</li>}</For>
                      </ul>
                    </div>
                  </Show>
                  <Show when={target().editable && ctl.ops().length === 0}>
                    <p role="alert" class="text-danger">{tm.noOps}</p>
                  </Show>
                  <Show when={cur().versionNum === null && target().editable}>
                    <p class="text-[12px] text-muted">{tm.versionUnknown}</p>
                  </Show>
                  <Show when={ctl.ops().length > 0}>
                    <OpPicker ctl={ctl} />
                    <Show when={cur().operation}>{(op) => <Options ctl={ctl} op={op()} errors={fieldErrors().byOption} />}</Show>
                    <Preview ctl={ctl} top={fieldErrors().top} />
                    <ApplyControls ctl={ctl} />
                  </Show>
                </>
              )}
            </Show>
            <Show when={applying()}>
              <Running ctl={ctl} now={now()} />
            </Show>
            <Show when={cur().result}>{(r) => <Result r={r()} />}</Show>
          </div>
        )}
      </Show>
    </Dialog>
  );
}

function OpPicker(props: { ctl: MaintenanceController }) {
  const name = createUniqueId();
  const s = () => props.ctl.state()!;
  const disabled = () => s().phase !== "ready";
  return (
    <fieldset class="flex flex-col gap-1">
      <legend class="mb-1 text-[12px] text-muted">{tm.operation}</legend>
      <div class="flex flex-wrap gap-x-4 gap-y-1">
        <For each={props.ctl.ops()}>
          {(o) => {
            const reasonId = createUniqueId();
            return (
              <div class="flex flex-col">
                <label class="flex items-center gap-1.5" title={tm.opDesc[o.name]}>
                  <input
                    type="radio"
                    name={name}
                    value={o.name}
                    checked={s().operation?.op === o.name}
                    disabled={disabled() || o.reason !== null}
                    aria-describedby={o.reason ? reasonId : undefined}
                    onChange={() => props.ctl.selectOp(o.name)}
                  />
                  {tm.ops[o.name]}
                </label>
                <Show when={o.reason}>
                  <span id={reasonId} class="text-[11px] text-muted">{o.reason}</span>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
      <Show when={s().operation}>{(op) => <p class="text-[12px] text-muted">{tm.opDesc[op().op]}</p>}</Show>
    </fieldset>
  );
}

/** Seçenek açıklaması + devre dışı gerekçesi + alan hataları (`aria-describedby` hedefi). */
function OptionNotes(props: { id: string; desc: string; st: OptionState | undefined; errors: string[] | undefined }) {
  return (
    <span id={props.id} class="flex flex-col pl-6 text-[11px]">
      <span class="text-muted">{props.desc}</span>
      <Show when={props.st && !props.st.enabled && props.st.reason}>
        <span class="text-warn-text">{props.st!.reason}</span>
      </Show>
      <For each={props.errors ?? []}>{(e) => <span class="text-danger">{e}</span>}</For>
    </span>
  );
}

function Options(props: { ctl: MaintenanceController; op: MaintenanceOp; errors: Record<string, string[]> }) {
  const states = () => props.ctl.optionStates();
  const locked = () => props.ctl.state()?.phase !== "ready";
  const field = (k: string) => (props.op as Record<string, unknown>)[k];
  return (
    <fieldset class="flex flex-col gap-2">
      <legend class="mb-1 text-[12px] text-muted">{tm.options}</legend>
      <Show when={props.op.op === "cluster"}>
        {(() => {
          const id = createUniqueId();
          const notes = createUniqueId();
          const indexes = () => props.ctl.state()?.target?.indexes ?? [];
          const st = () => states().usingIndex;
          return (
            <div class="flex flex-col gap-1">
              <label for={id}>{tm.opts.usingIndex.label}</label>
              <select
                id={id}
                class={inputClass}
                disabled={locked() || !st()?.enabled}
                aria-describedby={notes}
                value={(field("usingIndex") as string | null) ?? ""}
                onChange={(e) => props.ctl.setOptions({ usingIndex: e.currentTarget.value || null })}
              >
                <Show when={field("usingIndex") === null}>
                  <option value="">{tm.usingDefault}</option>
                </Show>
                <For each={indexes()}>{(i) => <option value={i}>{i}</option>}</For>
              </select>
              <OptionNotes id={notes} desc={tm.opts.usingIndex.desc} st={st()} errors={props.errors.usingIndex} />
            </div>
          );
        })()}
      </Show>
      <For each={BOOLS[props.op.op]}>
        {(k) => {
          const notes = createUniqueId();
          const st = () => states()[k];
          return (
            <div class="flex flex-col">
              <label class="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={field(k) === true}
                  disabled={locked() || !(st()?.enabled ?? true)}
                  aria-describedby={notes}
                  onChange={(e) => props.ctl.setOptions({ [k]: e.currentTarget.checked })}
                />
                <span class="font-mono text-[12px]">{tm.opts[k].label}</span>
              </label>
              <OptionNotes id={notes} desc={tm.opts[k].desc} st={st()} errors={props.errors[k]} />
            </div>
          );
        }}
      </For>
      <Show when={props.op.op === "vacuum"}>
        {(() => {
          const icId = createUniqueId();
          const icNotes = createUniqueId();
          const pId = createUniqueId();
          const pNotes = createUniqueId();
          const ic = () => states().indexCleanup;
          const auto = () => states().indexCleanupAuto;
          const par = () => states().parallel;
          return (
            <>
              <div class="flex flex-col gap-1">
                <label for={icId} class="font-mono text-[12px]">{tm.opts.indexCleanup.label}</label>
                <select
                  id={icId}
                  class={inputClass}
                  disabled={locked() || !ic()?.enabled}
                  aria-describedby={icNotes}
                  value={(field("indexCleanup") as string | null) ?? ""}
                  onChange={(e) => props.ctl.setOptions({ indexCleanup: e.currentTarget.value || null })}
                >
                  <option value="">{tm.indexCleanup.default}</option>
                  <option value="auto" disabled={!auto()?.enabled}>
                    {auto()?.enabled ? tm.indexCleanup.auto : `${tm.indexCleanup.auto} — ${auto()?.reason ?? ""}`}
                  </option>
                  <option value="on">{tm.indexCleanup.on}</option>
                  <option value="off">{tm.indexCleanup.off}</option>
                </select>
                <OptionNotes id={icNotes} desc={tm.opts.indexCleanup.desc} st={ic()} errors={props.errors.indexCleanup} />
              </div>
              <div class="flex flex-col gap-1">
                <label for={pId} class="font-mono text-[12px]">{tm.opts.parallel.label}</label>
                <input
                  id={pId}
                  type="number"
                  min="0"
                  max="1024"
                  step="1"
                  class={`${inputClass} !w-28`}
                  disabled={locked() || !par()?.enabled}
                  aria-describedby={pNotes}
                  value={(field("parallel") as number | null) ?? ""}
                  // onInput: Enter'a basıldığında değer önizlemeye girmiş olur (bekleyen önizleme Uygula'yı kapatır).
                  onInput={(e) => {
                    const v = e.currentTarget.value.trim();
                    props.ctl.setOptions({ parallel: v === "" ? null : Number(v) });
                  }}
                />
                <OptionNotes id={pNotes} desc={tm.opts.parallel.desc} st={par()} errors={props.errors.parallel} />
              </div>
            </>
          );
        })()}
      </Show>
    </fieldset>
  );
}

function Preview(props: { ctl: MaintenanceController; top: string[] }) {
  const s = () => props.ctl.state()!;
  const view = () => s().view;
  return (
    <section aria-label={tm.preview} classList={{ "opacity-70": s().pending && view() !== null }}>
      <h3 class="mb-1 text-[12px] text-muted">{tm.preview}</h3>
      <Show when={props.top.length > 0}>
        <ul role="alert" class="mb-1 list-disc pl-5 text-danger">
          <For each={props.top}>{(e) => <li>{e}</li>}</For>
        </ul>
      </Show>
      <Show when={s().previewError}>
        <p role="alert" class="text-danger">{`${tm.previewFailed}: ${s().previewError}`}</p>
      </Show>
      <Show when={view()} fallback={<p class="text-muted">{s().pending ? tm.previewPending : "—"}</p>}>
        {(v) => (
          <>
            <For each={v().statements}>
              {(st) => (
                <div class="flex items-start gap-2 rounded border border-border px-2 py-1">
                  <span
                    class="shrink-0 rounded border px-1.5 text-[11px]"
                    classList={{ "border-danger text-danger": st.risk !== "safe", "border-border text-muted": st.risk === "safe" }}
                  >
                    {tm.risk[st.risk] ?? st.risk}
                  </span>
                  <div class="min-w-0 flex-1">
                    <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px]">{st.sql}</pre>
                    <Show when={st.note}>
                      <p class="text-[12px] text-warn-text">{st.note}</p>
                    </Show>
                  </div>
                </div>
              )}
            </For>
            <Show when={v().warnings.length > 0}>
              <h3 class="mb-1 mt-2 text-[12px] text-muted">{tm.warnings}</h3>
              <ul class="list-disc pl-5 text-[12px] text-warn-text">
                <For each={v().warnings}>{(w) => <li>{w}</li>}</For>
              </ul>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}

function ApplyControls(props: { ctl: MaintenanceController }) {
  const s = () => props.ctl.state()!;
  const busy = () => s().phase === "applying";
  const needsConfirm = () => {
    const g = props.ctl.gate();
    return g.ok ? g.confirm : s().view?.risk === "destructive" || s().view?.risk === "data_loss";
  };
  return (
    <div class="flex flex-wrap items-center gap-3 border-t border-border pt-2">
      <select
        class={`${inputClass} !w-auto`}
        aria-label={tm.lockTimeout}
        title={tm.lockTimeoutTitle}
        disabled={busy()}
        value={String(s().lockTimeoutMs)}
        onChange={(e) => props.ctl.setLockTimeout(Number(e.currentTarget.value))}
      >
        <For each={LOCK_TIMEOUTS}>{(ms) => <option value={String(ms)}>{`${tm.lockTimeout}: ${tm.lockChoice(ms)}`}</option>}</For>
      </select>
      <Show when={needsConfirm() && !s().locked}>
        <label class="flex items-center gap-2">
          <input
            type="checkbox"
            checked={s().confirmed}
            disabled={busy() || !props.ctl.gate().ok}
            onChange={(e) => props.ctl.setConfirmed(e.currentTarget.checked)}
          />
          {tm.confirm}
        </label>
      </Show>
    </div>
  );
}

function Running(props: { ctl: MaintenanceController; now: number }) {
  const s = () => props.ctl.state()!;
  const pv = () => progressView(s().progress, phaseLabel);
  const elapsed = () => elapsedLabel(props.now - (s().startedAt ?? props.now));
  return (
    <section class="flex flex-col gap-1" aria-label={tm.progress}>
      <div
        role="progressbar"
        aria-label={tm.progress}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pv().percent ?? undefined}
        aria-valuetext={pv().text}
        class="h-2 w-full overflow-hidden rounded bg-panel"
      >
        <div
          class="h-full bg-accent"
          classList={{ "animate-pulse": pv().percent === null }}
          style={{ width: `${pv().percent ?? 100}%` }}
        />
      </div>
      <div class="flex justify-between text-[12px] text-muted">
        <span>{pv().text}</span>
        <span>{tm.elapsed(elapsed())}</span>
      </div>
      <Show when={s().progressStopped}>
        <p class="text-[12px] text-warn-text">{tm.progressStopped}</p>
      </Show>
    </section>
  );
}

function Result(props: { r: MaintenanceResult }) {
  const r = () => props.r;
  const head = () =>
    r().kind === "applied" ? tm.doneIn(elapsedLabel(r().elapsedMs)) : `${tm.status[r().kind]} (${elapsedLabel(r().elapsedMs)})`;
  const body = (
    <>
      <p class="font-medium" classList={{ "text-danger": r().kind === "failed" }}>
        {head()}
        <Show when={r().warnings > 0}>
          <span class="ml-2 text-warn-text">{`⚠ ${tm.warnCount(r().warnings)}`}</span>
        </Show>
      </p>
      <Show when={r().error}>
        <p class="whitespace-pre-wrap break-words">{r().error}</p>
      </Show>
      <Show when={r().hint}>
        <p class="text-muted">{`${tm.hint}: ${r().hint}`}</p>
      </Show>
      <Show when={r().ccnew}>
        <p class="text-warn-text">{tm.ccnew}</p>
      </Show>
    </>
  );
  return (
    <div class="flex flex-col gap-2 rounded border border-border px-3 py-2">
      <Show when={r().kind === "failed"} fallback={<div role="status">{body}</div>}>
        <div role="alert">{body}</div>
      </Show>
      <Show when={r().messages.length > 0}>
        <details open={r().warnings > 0}>
          <summary class="cursor-pointer text-[12px] text-muted">{tm.messages(r().messages.length)}</summary>
          <ul class="mt-1 max-h-48 overflow-auto font-mono text-[12px]">
            <For each={r().messages}>
              {(m) => {
                const warn = m.severity.toUpperCase() === "WARNING";
                return (
                  <li class="border-b border-border py-0.5" classList={{ "text-warn-text font-medium": warn }}>
                    {`${warn ? "⚠ " : ""}${m.severity}: ${m.message}`}
                    <Show when={m.detail}>
                      <div class="pl-4 text-muted">{m.detail}</div>
                    </Show>
                    <Show when={m.hint}>
                      <div class="pl-4 text-muted">{m.hint}</div>
                    </Show>
                  </li>
                );
              }}
            </For>
          </ul>
        </details>
      </Show>
    </div>
  );
}
