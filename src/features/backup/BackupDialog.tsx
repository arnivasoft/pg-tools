// "Yedekle…" diyaloğu (Plan 27). Durumun tek kaynağı `BackupController`dır; bu dosya yalnız çizer ve olayları ona iletir.
// Klavye: Enter başlatır (kapı tamamsa; IME sırasında, `textarea`/`select`'te değil), Esc kapatır (çalışırken onay sorar).
import { createUniqueId, For, Show } from "solid-js";
import { tb } from "../../app/strings-backup";
import type { DumpFormat } from "../../ipc/bindings/DumpFormat";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { inputClass } from "../../ui/Field";
import { backupOptionStates, splitFieldErrors, type BackupKey } from "./backup-model";
import type { BackupController, BackupState } from "./backup-controller";
import { CheckOption, Group, NumberOption, SelectOption } from "./controls";
import { liveTexts } from "./job-live";
import { CloseAsk, enterToStart, JobView } from "./JobView";
import { ToolsPanel } from "./ToolsPanel";

const FORMATS: DumpFormat[] = ["custom", "plain", "directory", "tar"];
const OPTION_KEYS: BackupKey[] = [
  "schemaOnly", "dataOnly", "noOwner", "noPrivileges", "create", "clean", "ifExists", "inserts", "columnInserts", "compress", "jobs", "encoding",
];
const GENERAL: BackupKey[] = ["schemaOnly", "dataOnly", "noOwner", "noPrivileges", "create"];
const PLAIN: BackupKey[] = ["clean", "ifExists", "inserts", "columnInserts"];
const BOOL_LABEL: Record<string, string> = {
  schemaOnly: tb.optionLabels.schemaOnly,
  dataOnly: tb.optionLabels.dataOnly,
  noOwner: tb.optionLabels.noOwner,
  noPrivileges: tb.optionLabels.noPrivileges,
  create: tb.optionLabels.create,
  clean: tb.optionLabels.clean,
  ifExists: tb.optionLabels.ifExists,
  inserts: tb.optionLabels.inserts,
  columnInserts: tb.optionLabels.columnInserts,
};

function scopeTitle(s: BackupState): string {
  const sc = s.scope;
  if (!sc) return "";
  if (sc.kind === "database") return tb.scopeDatabase(s.dbName);
  if (sc.kind === "schema") return tb.scopeSchema(s.dbName, sc.name);
  return tb.scopeTable(s.dbName, sc.schema, sc.name);
}

export function BackupDialog(props: { ctl: BackupController }) {
  const ctl = props.ctl;
  const s = () => ctl.state();
  const reasonId = createUniqueId();
  const pathId = createUniqueId();
  const pathErrId = createUniqueId();
  const running = () => s()?.phase === "running";
  const ready = () => s()?.phase === "ready";
  const gateReason = (): string | undefined => {
    const g = ctl.gate();
    return g.kind === "blocked" ? tb.blocked[g.reason] : undefined;
  };
  const canStart = () => ctl.gate().kind === "ready" && ready();
  const states = () => {
    const cur = s();
    return cur ? backupOptionStates(cur.format, cur.options) : null;
  };
  const errors = () => {
    const cur = s();
    const st = states();
    if (!cur || !st) return { byField: {} as Record<string, string>, general: [] as string[] };
    const fields = ["outputPath", "scope.name", "scope.schema", ...OPTION_KEYS.filter((k) => st[k].visible).map((k) => `options.${k}`)];
    // Boş yolda `outputPath` hatası ("Yol boş olamaz.") alan altında gösterilmez: kapı zaten "hedef seçilmedi" der.
    const list = (cur.plan?.errors ?? []).filter((e) => !(e.path === "outputPath" && cur.outputPath === ""));
    return splitFieldErrors(list, fields);
  };
  const live = () => {
    const cur = s();
    return cur
      ? liveTexts({
          phase: cur.phase, loadError: cur.loadError, closing: cur.closing, closeAsk: cur.closeAsk, summary: cur.log.summary,
          mode: "backup", singleTransaction: false, startError: cur.startError,
        })
      : { status: "", alert: "" };
  };
  const stale = () => {
    const cur = s();
    return !!cur && (cur.pending || cur.planFor !== cur.version);
  };
  const vis = (keys: BackupKey[]) => keys.some((k) => states()?.[k].visible);
  const err = (k: BackupKey) => errors().byField[`options.${k}`];
  const title = () => {
    const cur = s();
    return cur && cur.scopeLabel !== "" ? tb.titleBackup(cur.scopeLabel) : tb.backupTitle;
  };

  const bool = (k: BackupKey) => {
    const st = () => states()![k];
    return (
      <Show when={st().visible}>
        <CheckOption
          label={BOOL_LABEL[k]}
          checked={s()?.options[k] === true}
          state={st()}
          error={err(k)}
          disabled={!ready()}
          onChange={(v) => ctl.setOption(k, v as never)}
        />
      </Show>
    );
  };

  const encodingOptions = () => {
    const cur = s();
    if (!cur) return [];
    const list = cur.encodings.includes(cur.options.encoding ?? "") || cur.options.encoding === null ? cur.encodings : [cur.options.encoding!, ...cur.encodings];
    return [{ value: "", label: tb.encodingDefault }, ...list.map((e) => ({ value: e, label: e }))];
  };
  const compressOptions = [
    { value: "", label: tb.compressDefault },
    ...Array.from({ length: 10 }, (_, i) => ({ value: String(i), label: i === 0 ? `0 (${tb.compressNone})` : String(i) })),
  ];

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
            disabled={!canStart()}
            title={gateReason()}
            aria-describedby={gateReason() ? reasonId : undefined}
            onClick={() => canStart() && void ctl.start()}
          >
            {s()?.log.summary ? tb.again : tb.backupTitle}
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
            <Show when={cur().closeAsk}>
              <CloseAsk onCancelAndClose={() => ctl.answerClose(true)} onContinue={() => ctl.answerClose(false)} />
            </Show>
            <Show when={cur().closing}>
              <p class="text-muted">{tb.closeWaiting}</p>
            </Show>
            <Show when={cur().phase === "ready" || cur().phase === "running"}>
              <div class="flex flex-col gap-0.5">
                <p class="text-[12px] text-muted">{tb.scope}</p>
                <p class="break-all font-medium">{scopeTitle(cur())}</p>
                <For each={[errors().byField["scope.name"], errors().byField["scope.schema"]].filter(Boolean)}>
                  {(m) => <p class="text-[12px] text-danger">{m}</p>}
                </For>
              </div>
              <ToolsPanel
                job="backup"
                tools={cur().tools}
                toolsError={cur().toolsError}
                pgBinDir={cur().pgBinDir}
                binDirError={cur().binDirError}
                serverMajor={cur().serverMajor}
                disabled={!ready()}
                onChoose={() => ctl.chooseBinDir()}
                onClear={() => ctl.clearBinDir()}
                onRescan={() => ctl.rescanTools()}
              />
              <fieldset class="flex flex-col gap-1" disabled={!ready()}>
                <legend class="mb-1 text-[12px] text-muted">{tb.format}</legend>
                <div class="flex flex-wrap gap-x-4 gap-y-1">
                  <For each={FORMATS}>
                    {(f) => {
                      const hint = createUniqueId();
                      return (
                        <div class="flex flex-col">
                          <label class="flex items-center gap-1.5">
                            <input
                              type="radio"
                              name={pathId + "-format"}
                              value={f}
                              checked={cur().format === f}
                              aria-describedby={hint}
                              onChange={() => ctl.setFormat(f)}
                            />
                            {tb.formats[f]}
                          </label>
                          <span id={hint} class="pl-6 text-[11px] text-muted">{tb.formatHints[f]}</span>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </fieldset>
              <div class="flex flex-col gap-1">
                <label for={pathId} class="text-[12px] text-muted">{tb.outputPath}</label>
                <div class="flex gap-2">
                  <input
                    id={pathId}
                    readOnly
                    class={`${inputClass} font-mono`}
                    value={cur().outputPath}
                    placeholder={tb.outputPathEmpty}
                    aria-describedby={errors().byField.outputPath ? pathErrId : undefined}
                    aria-invalid={errors().byField.outputPath ? true : undefined}
                  />
                  <Button class="shrink-0 whitespace-nowrap" disabled={!ready()} onClick={() => void ctl.chooseOutput()}>
                    {cur().outputPath === "" ? tb.choosePath : tb.changePath}
                  </Button>
                </div>
                <Show when={errors().byField.outputPath}>
                  {(m) => <p id={pathErrId} class="text-[12px] text-danger">{m()}</p>}
                </Show>
              </div>
              <div class="flex flex-col gap-3">
                <p class="text-[12px] text-muted">{tb.options}</p>
                <Show when={vis(GENERAL)}>
                  <Group legend={tb.groups.general} disabled={!ready()}>
                    <For each={GENERAL}>{(k) => bool(k)}</For>
                  </Group>
                </Show>
                <Show when={vis(PLAIN)}>
                  <Group legend={tb.groups.plain} disabled={!ready()}>
                    <For each={PLAIN}>{(k) => bool(k)}</For>
                  </Group>
                </Show>
                <Show when={vis(["compress", "jobs"])}>
                  <Group legend={tb.groups.compression} disabled={!ready()}>
                    <Show when={states()!.compress.visible}>
                      <SelectOption
                        label={tb.optionLabels.compress}
                        value={cur().options.compress === null ? "" : String(cur().options.compress)}
                        options={compressOptions}
                        state={states()!.compress}
                        error={err("compress")}
                        disabled={!ready()}
                        onChange={(v) => ctl.setOption("compress", v === "" ? null : Number(v))}
                      />
                    </Show>
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
                <Group legend={tb.groups.encoding} disabled={!ready()}>
                  <SelectOption
                    label={tb.optionLabels.encoding}
                    value={cur().options.encoding ?? ""}
                    options={encodingOptions()}
                    state={states()!.encoding}
                    error={err("encoding")}
                    disabled={!ready()}
                    onChange={(v) => ctl.setOption("encoding", v === "" ? null : v)}
                  />
                </Group>
              </div>
              <JobView
                mode="backup"
                running={running()}
                plan={cur().plan}
                stale={stale()}
                general={errors().general}
                planError={cur().planError}
                startError={cur().startError}
                log={cur().log}
                startedAt={cur().startedAt}
                cancelRequested={cur().cancelRequested}
                singleTransaction={false}
                onCancel={() => ctl.cancel()}
                onReveal={cur().log.summary?.outcome === "success" ? () => void ctl.reveal() : undefined}
              />
            </Show>
          </div>
        )}
      </Show>
    </Dialog>
  );
}
