// "Araç ayarları" bölümü (Plan 27, Kararlar 3): seçili araç özeti, bin klasörü ve bulunan kümeler. Yalnız çizer; olaylar denetleyiciye gider.
// Kapalı başlar; seçili araç yoksa ya da `problem` doluysa açık başlar.
import { createEffect, createSignal, For, on, Show } from "solid-js";
import { tb } from "../../app/strings-backup";
import type { SourceFormat } from "../../ipc/bindings/SourceFormat";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import { Button } from "../../ui/Button";
import { chosenTool, toolAdvice, toolFor } from "./backup-model";

export interface ToolsPanelProps {
  job: "backup" | "restore";
  tools: ToolDetection | null;
  toolsError: string | null;
  pgBinDir: string | null;
  binDirError: string | null;
  serverMajor: number | null;
  /** Geri yüklemede kaynak biçimi: düz SQL ise psql gösterilir. */
  sourceFormat?: SourceFormat | null;
  disabled: boolean;
  onChoose: () => Promise<void> | void;
  onClear: () => Promise<void> | void;
  onRescan: () => Promise<void> | void;
}

export function ToolsPanel(props: ToolsPanelProps) {
  const which = () => toolFor(props.job, props.sourceFormat ?? null).which;
  const toolName = () => toolFor(props.job, props.sourceFormat ?? null).name;
  const chosen = () => (props.tools ? chosenTool(props.tools, which()) : null);
  const advice = () => toolAdvice(chosen()?.version ?? null, props.serverMajor, which() === "psql" ? "psql" : props.job);
  const problem = () => props.toolsError ?? props.tools?.problem ?? null;
  const [open, setOpen] = createSignal(chosen() === null || problem() !== null);
  // Sorun sonradan ortaya çıkarsa (yeniden tarama) bölüm açılır; sorunsuz taramada kullanıcının seçimine dokunulmaz.
  createEffect(on(problem, (p) => p !== null && setOpen(true), { defer: true }));
  const [busy, setBusy] = createSignal(false);
  const run = async (fn: () => Promise<void> | void) => {
    setBusy(true);
    try {
      await fn();
    } finally {
      setBusy(false);
    }
  };
  const selectedDir = () => (props.tools ? (which() === "dump" ? props.tools.chosenDump : which() === "psql" ? props.tools.chosenPsql : props.tools.chosenRestore) : null);
  const bodyId = "tools-body-" + props.job;
  return (
    <section class="flex flex-col gap-1.5 rounded border border-border px-3 py-2">
      <button
        type="button"
        class="flex items-center gap-2 text-left"
        aria-expanded={open()}
        aria-controls={bodyId}
        onClick={() => setOpen(!open())}
      >
        <span aria-hidden="true" class="w-3 text-muted">{open() ? "▾" : "▸"}</span>
        <span class="text-[12px] text-muted">{tb.tools.title}</span>
        <span class="ml-auto truncate font-mono text-[12px]">
          <Show when={chosen()} fallback={<span class="text-danger">{tb.tools.none}</span>}>
            {(c) => tb.tools.summary(toolName(), c().version.text)}
          </Show>
        </span>
      </button>
      <Show when={advice()}>
        {(a) => <p class={`text-[12px] ${a().tone === "warning" ? "text-warn-text" : "text-muted"}`}>{a().text}</p>}
      </Show>
      <Show when={problem()}>{(p) => <p class="text-[12px] text-danger">{p()}</p>}</Show>
      <div id={bodyId} hidden={!open()} class="flex flex-col gap-2 pt-1">
        <div class="flex flex-col gap-1">
          <span class="text-[12px] text-muted">{tb.tools.binDir}</span>
          <span class="break-all font-mono text-[12px]">{props.pgBinDir ?? tb.tools.binDirAuto}</span>
          <div class="flex flex-wrap gap-2">
            <Button disabled={props.disabled || busy()} onClick={() => void run(props.onChoose)}>{tb.tools.choose}</Button>
            <Button disabled={props.disabled || busy() || props.pgBinDir === null} onClick={() => void run(props.onClear)}>
              {tb.tools.clear}
            </Button>
            <Button disabled={props.disabled || busy()} onClick={() => void run(props.onRescan)}>
              {busy() ? tb.tools.scanning : tb.tools.rescan}
            </Button>
          </div>
          <Show when={props.binDirError}>{(e) => <p class="text-[12px] text-danger">{e()}</p>}</Show>
        </div>
        <div>
          <span class="text-[12px] text-muted">{tb.tools.sets}</span>
          <Show when={(props.tools?.sets.length ?? 0) > 0} fallback={<p class="text-[12px] text-muted">{tb.tools.noSets}</p>}>
            <table class="mt-1 w-full text-left text-[12px]">
              <thead class="text-muted">
                <tr>
                  <th class="pr-2 font-normal">{tb.tools.colDir}</th>
                  <th class="pr-2 font-normal">{tb.tools.colVersion}</th>
                  <th class="pr-2 font-normal">pg_dump</th>
                  <th class="pr-2 font-normal">pg_restore</th>
                  <th class="pr-2 font-normal">psql</th>
                  <th class="font-normal"><span class="sr-only">{tb.tools.selected}</span></th>
                </tr>
              </thead>
              <tbody>
                <For each={props.tools?.sets ?? []}>
                  {(s) => (
                    <tr class="align-top">
                      <td class="break-all pr-2 font-mono">{s.dir}</td>
                      <td class="pr-2">{s.version.text}</td>
                      <td class="pr-2">{s.pgDump ? tb.tools.present : tb.tools.absent}</td>
                      <td class="pr-2">{s.pgRestore ? tb.tools.present : tb.tools.absent}</td>
                      <td class="pr-2">{s.psql ? tb.tools.present : tb.tools.absent}</td>
                      <td>{s.dir === selectedDir() ? tb.tools.selected : ""}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </div>
      </div>
    </section>
  );
}
