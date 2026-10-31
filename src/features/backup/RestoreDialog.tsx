// "Geri yükle…" diyaloğu (Plan 27). Durumun tek kaynağı `RestoreController`dır; bu dosya yalnız çizer ve olayları ona iletir.
// Onay kutusu / sunucu adı alanı yalnız güncel plan gelmişken etkilidir (plan bekleyen ya da bayatken devre dışı).
// Klavye: Enter başlatır (kapı ve onay tamamsa; IME sırasında, `textarea`/`select`'te değil), Esc kapatır (çalışırken onay sorar).
import { createEffect, createMemo, createUniqueId, For, on, Show } from "solid-js";
import { tb } from "../../app/strings-backup";
import type { RestoreTarget } from "../../ipc/bindings/RestoreTarget";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { inputClass } from "../../ui/Field";
import { canStart as gateCanStart, restoreOptionStates, splitFieldErrors, type RestoreKey } from "./backup-model";
import type { RestoreController } from "./backup-controller";
import { CheckOption, Group, NumberOption } from "./controls";
import { liveTexts } from "./job-live";
import { CloseAsk, enterToStart, JobView } from "./JobView";
import { ToolsPanel } from "./ToolsPanel";

const BOOL_KEYS: RestoreKey[] = ["clean", "ifExists", "singleTransaction", "exitOnError", "schemaOnly", "dataOnly", "noOwner", "noPrivileges"];
const OPTION_FIELDS: RestoreKey[] = [...BOOL_KEYS, "jobs"];
const TARGETS: RestoreTarget[] = ["existing", "create_from_archive"];

export function RestoreDialog(props: { ctl: RestoreController; tree: () => { nodes: Map<string, TreeNode> } }) {
  const ctl = props.ctl;
  const s = () => ctl.state();
  const reasonId = createUniqueId();
  const srcId = createUniqueId();
  const srcErrId = createUniqueId();
  const dbSelId = createUniqueId();
  const nameId = createUniqueId();
  const running = () => s()?.phase === "running";
  const ready = () => s()?.phase === "ready";

  // Sunucu düğümünden açıldıysa "mevcut" hedef seçicisinin listesi bir kez tembel yüklenir.
  const needDbs = createMemo(() => {
    const cur = s();
    return !!cur && cur.phase === "ready" && cur.fromServer && cur.databases === null;
  });
  createEffect(on(needDbs, (need) => need && void ctl.loadDatabases()));

  const states = () => {
    const cur = s();
    return cur ? restoreOptionStates(cur.source?.format ?? "custom", cur.target, cur.options) : null;
  };
  const errors = () => {
    const cur = s();
    const st = states();
    if (!cur || !st) return { byField: {} as Record<string, string>, general: [] as string[] };
    const fields = ["path", "target", ...OPTION_FIELDS.filter((k) => st[k].visible).map((k) => `options.${k}`)];
    return splitFieldErrors(cur.plan?.errors ?? [], fields);
  };
  const err = (k: RestoreKey) => errors().byField[`options.${k}`];
  const live = () => {
    const cur = s();
    return cur
      ? liveTexts({
          phase: cur.phase, loadError: cur.loadError, closing: cur.closing, closeAsk: cur.closeAsk, summary: cur.log.summary,
          mode: "restore", singleTransaction: cur.options.singleTransaction, startError: cur.startError,
        })
      : { status: "", alert: "" };
  };
  /** Plan güncel mi? Onay denetimleri yalnız güncel plan gelmişken etkilidir. */
  const planCurrent = () => {
    const cur = s();
    return !!cur && !cur.pending && cur.planFor === cur.version;
  };
  const gateKind = () => ctl.gate();
  const canStart = () => {
    const cur = s();
    return !!cur && cur.phase === "ready" && gateCanStart(gateKind(), cur.confirm);
  };
  const gateReason = (): string | undefined => {
    const g = gateKind();
    const cur = s();
    if (g.kind === "blocked") return tb.blocked[g.reason];
    if (!cur || canStart()) return undefined;
    if (g.kind === "needsProductionName") return cur.confirm.checked ? tb.needName : tb.needConfirm;
    return g.kind === "needsConfirm" ? tb.needConfirm : undefined;
  };
  const risk = () => {
    const p = s()?.plan;
    return p && p.errors.length === 0 && p.risk !== "safe" ? p.risk : null;
  };
  const dangerous = () => risk() === "data_loss";
  const title = () => {
    const cur = s();
    if (!cur) return tb.restoreTitle;
    const name = cur.fromServer ? cur.serverName : (props.tree().nodes.get(cur.nodeId)?.label ?? "");
    return name !== "" ? tb.titleRestore(name) : tb.restoreTitle;
  };
  const dbLabel = () => {
    const cur = s();
    return cur?.dbNodeId ? (props.tree().nodes.get(cur.dbNodeId)?.label ?? "") : "";
  };

  const bool = (k: RestoreKey) => {
    const st = () => states()![k];
    return (
      <Show when={st().visible}>
        <CheckOption
          label={tb.optionLabels[k as keyof typeof tb.optionLabels]}
          checked={s()?.options[k] === true}
          state={st()}
          error={err(k)}
          disabled={!ready()}
          note={k === "singleTransaction" ? tb.restoreSingleTxWarn : undefined}
          onChange={(v) => ctl.setOption(k, v as never)}
        />
      </Show>
    );
  };

  return (
    <Dialog
      open={s() !== null}
      onOpenChange={(o) => {
        if (o) return;
        if (s()?.closeAsk) ctl.answerClose(false);
        else ctl.requestClose();
      }}
      title={title()}
      width="640px"
      footer={
        <>
          <span id={reasonId} class="mr-auto self-center text-[12px] text-muted">
            {gateReason() ?? ""}
          </span>
          <Button onClick={() => ctl.requestClose()}>{tb.close}</Button>
          <Button
            variant="primary"
            class={dangerous() ? "!bg-danger !text-danger-fg" : ""}
            disabled={!canStart()}
            title={gateReason()}
            aria-describedby={gateReason() ? reasonId : undefined}
            onClick={() => canStart() && void ctl.start()}
          >
            {s()?.log.summary ? tb.again : tb.restoreTitle}
          </Button>
        </>
      }
    >
      <div role="status" class="sr-only">{live().status}</div>
      <div role="alert" class="sr-only">{live().alert}</div>
      <Show when={s()}>
        {(cur) => (
          <div class="flex flex-col gap-3 text-[13px]" onKeyDown={enterToStart(canStart, () => void ctl.start())}>
            <Show when={cur().phase === "loading"}>
              <p class="text-muted">{tb.loading}</p>
            </Show>
            <Show when={cur().phase === "loadError"}>
              <div class="flex flex-col gap-2 rounded border border-danger px-3 py-2">
                <p class="font-medium text-danger">{tb.loadFailed}</p>
                <Show when={cur().loadError && cur().loadError !== tb.loadFailed}>
                  <p class="text-muted">{cur().loadError}</p>
                </Show>
                <div>
                  <Button onClick={() => ctl.open(cur().nodeId)}>{tb.retry}</Button>
                </div>
              </div>
            </Show>
            <Show when={cur().locked}>
              <p class="text-danger">{tb.blocked.locked}</p>
            </Show>
            <Show when={cur().closeAsk}>
              <CloseAsk onCancelAndClose={() => ctl.answerClose(true)} onContinue={() => ctl.answerClose(false)} />
            </Show>
            <Show when={cur().closing}>
              <p class="text-muted">{tb.closeWaiting}</p>
            </Show>
            <Show when={cur().phase === "ready" || cur().phase === "running"}>
              <ToolsPanel
                job="restore"
                tools={cur().tools}
                toolsError={cur().toolsError}
                pgBinDir={cur().pgBinDir}
                binDirError={cur().binDirError}
                serverMajor={cur().serverMajor}
                sourceFormat={cur().source?.format ?? null}
                disabled={!ready()}
                onChoose={() => ctl.chooseBinDir()}
                onClear={() => ctl.clearBinDir()}
                onRescan={() => ctl.rescanTools()}
              />

              {/* Kaynak */}
              <div class="flex flex-col gap-1">
                <label for={srcId} class="text-[12px] text-muted">{tb.source}</label>
                <div class="flex gap-2">
                  <input
                    id={srcId}
                    readOnly
                    class={`${inputClass} font-mono`}
                    value={cur().source?.path ?? ""}
                    placeholder={tb.sourceEmpty}
                    aria-describedby={cur().sourceError || errors().byField.path ? srcErrId : undefined}
                    aria-invalid={cur().sourceError || errors().byField.path ? true : undefined}
                  />
                  <Button disabled={!ready()} onClick={() => void ctl.chooseSource("file")}>{tb.chooseFile}</Button>
                  <Button disabled={!ready()} onClick={() => void ctl.chooseSource("dir")}>{tb.chooseFolder}</Button>
                </div>
                <Show when={cur().inspecting}>
                  <p class="text-[12px] text-muted">{tb.inspecting}</p>
                </Show>
                <Show when={cur().sourceError ?? errors().byField.path}>
                  {(m) => <p id={srcErrId} class="text-[12px] text-danger">{m()}</p>}
                </Show>
                <Show when={cur().source}>
                  {(src) => (
                    <div class="flex flex-col gap-0.5 rounded border border-border px-3 py-2 text-[12px]">
                      <p class="font-medium">{tb.sourceFormat(tb.formats[src().format])}</p>
                      <Show when={src().archive}>
                        {(a) => (
                          <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
                            <dt class="text-muted">{tb.archive.name}</dt>
                            <dd class="break-all">{a().dbName ?? tb.archive.unknown}</dd>
                            <dt class="text-muted">{tb.archive.dumpVersion}</dt>
                            <dd>{a().archiveVersion ?? tb.archive.unknown}</dd>
                            <dt class="text-muted">{tb.archive.serverVersion}</dt>
                            <dd>{a().serverVersion ?? tb.archive.unknown}</dd>
                            <dt class="text-muted">{tb.archive.items}</dt>
                            <dd>{a().items}</dd>
                            <Show when={a().counts.length > 0}>
                              <dt class="text-muted">{tb.tablesSummary}</dt>
                              <dd class="break-all">
                                {a().counts.slice(0, 6).map(([k, n]) => `${k} ${n}`).join(", ")}
                                {a().counts.length > 6 ? ", …" : ""}
                              </dd>
                            </Show>
                          </dl>
                        )}
                      </Show>
                    </div>
                  )}
                </Show>
              </div>

              {/* Hedef */}
              <fieldset class="flex flex-col gap-1" disabled={!ready()}>
                <legend class="mb-1 text-[12px] text-muted">{tb.target}</legend>
                <For each={TARGETS}>
                  {(t) => {
                    const noteId = createUniqueId();
                    const plainBlocked = () => t === "create_from_archive" && cur().source?.format === "plain";
                    return (
                      <div class="flex flex-col gap-1">
                        <label class="flex items-center gap-1.5">
                          <input
                            type="radio"
                            name={srcId + "-target"}
                            value={t}
                            checked={cur().target === t}
                            disabled={plainBlocked()}
                            aria-describedby={plainBlocked() ? noteId : undefined}
                            onChange={() => ctl.setTarget(t)}
                          />
                          {t === "existing" ? tb.targetExisting : tb.targetCreate}
                        </label>
                        <Show when={plainBlocked()}>
                          <span id={noteId} class="pl-6 text-[11px] text-warn-text">{tb.reason.plainNoCreate}</span>
                        </Show>
                        <Show when={t === "create_from_archive" && cur().target === t && cur().source?.archive?.dbName}>
                          <span class="pl-6 font-mono text-[12px]">{tb.targetCreateName(cur().source?.archive?.dbName ?? "")}</span>
                        </Show>
                        <Show when={t === "existing" && cur().target === t}>
                          <div class="pl-6">
                            <Show
                              when={cur().fromServer}
                              fallback={<span class="font-mono text-[12px]">{dbLabel()}</span>}
                            >
                              <select
                                id={dbSelId}
                                class={inputClass}
                                aria-label={tb.targetExisting}
                                disabled={cur().databases === null}
                                value={cur().dbNodeId ?? ""}
                                onChange={(e) => e.currentTarget.value !== "" && void ctl.setTargetDb(e.currentTarget.value)}
                              >
                                <option value="" selected={cur().dbNodeId === null}>
                                  {cur().databases === null ? tb.targetLoading : tb.targetPick}
                                </option>
                                <For each={cur().databases ?? []}>
                                  {(d) => <option value={d.nodeId} selected={d.nodeId === cur().dbNodeId}>{d.label}</option>}
                                </For>
                              </select>
                            </Show>
                          </div>
                        </Show>
                      </div>
                    );
                  }}
                </For>
                <Show when={errors().byField.target}>
                  {(m) => <p class="text-[12px] text-danger">{m()}</p>}
                </Show>
              </fieldset>

              {/* Seçenekler */}
              <Show when={OPTION_FIELDS.some((k) => states()?.[k].visible)}>
                <Group legend={tb.options} disabled={!ready()}>
                  <For each={BOOL_KEYS}>{(k) => bool(k)}</For>
                  <Show when={states()!.jobs.visible}>
                    <NumberOption
                      label={tb.optionLabels.jobs}
                      value={cur().options.jobs}
                      state={states()!.jobs}
                      error={err("jobs")}
                      disabled={!ready()}
                      onChange={(v) => ctl.setOption("jobs", v)}
                    />
                  </Show>
                </Group>
              </Show>

              {/* Risk ve onay */}
              <Show when={risk()}>
                {(r) => (
                  <div class={`flex flex-col gap-2 rounded border px-3 py-2 ${r() === "data_loss" ? "border-danger" : "border-warn-text"}`}>
                    <p class="flex items-center gap-2">
                      <span
                        class={`rounded px-1.5 py-0.5 text-[11px] font-medium ${
                          r() === "data_loss" ? "bg-danger text-danger-fg" : "border border-warn-text text-warn-text"
                        }`}
                      >
                        {r() === "data_loss" ? tb.riskBadge.dataLoss : tb.riskBadge.destructive}
                      </span>
                      <span class="text-[12px]">{r() === "data_loss" ? tb.risk.dataLoss : tb.risk.destructive}</span>
                    </p>
                    <label class="flex items-start gap-2">
                      <input
                        type="checkbox"
                        class="mt-0.5"
                        checked={cur().confirm.checked}
                        disabled={!ready() || !planCurrent()}
                        onChange={(e) => ctl.setConfirm({ checked: e.currentTarget.checked })}
                      />
                      <span>{tb.confirmRestore}</span>
                    </label>
                    <Show when={r() === "data_loss" && cur().production}>
                      <div class="flex flex-col gap-1">
                        <label for={nameId} class="text-[12px]">{tb.confirmName(cur().serverName)}</label>
                        <input
                          id={nameId}
                          class={inputClass}
                          autocomplete="off"
                          spellcheck={false}
                          value={cur().confirm.typedName}
                          placeholder={tb.confirmProductionLabel}
                          disabled={!ready() || !planCurrent()}
                          onInput={(e) => ctl.setConfirm({ typedName: e.currentTarget.value })}
                        />
                      </div>
                    </Show>
                  </div>
                )}
              </Show>

              <JobView
                mode="restore"
                running={running()}
                plan={cur().plan}
                stale={!planCurrent()}
                general={errors().general}
                planError={cur().planError}
                startError={cur().startError}
                log={cur().log}
                startedAt={cur().startedAt}
                cancelRequested={cur().cancelRequested}
                singleTransaction={cur().options.singleTransaction}
                onCancel={() => ctl.cancel()}
              />
            </Show>
          </div>
        )}
      </Show>
    </Dialog>
  );
}
