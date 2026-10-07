import { save } from "@tauri-apps/plugin-dialog";
import { For, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { texp } from "./strings-export";
import { toast } from "../../app/toast";
import type { CsvDelimiter } from "../../ipc/bindings/CsvDelimiter";
import type { CsvOptions } from "../../ipc/bindings/CsvOptions";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import { DEFAULT_CSV_OPTIONS, runExport, type ExportOutcome } from "./export";

const DELIMITERS: CsvDelimiter[] = ["comma", "semicolon", "tab"];

/**
 * Dışa aktarma arayüzü (söz tabanlı seçenek diyaloğu + ilerleme). `View` bir kez bağlanır; `start` akışı çalıştırır.
 * Üzerine yazma onayını Windows kaydet diyaloğu verir (FOS_OVERWRITEPROMPT, spec §8); ayrı onay yoktur.
 */
export function createExportUi() {
  const [optionsReq, setOptionsReq] = createSignal<((o: CsvOptions | null) => void) | null>(null);
  const [running, setRunning] = createSignal(false);
  const [rows, setRows] = createSignal(0);
  const [opts, setOpts] = createSignal<CsvOptions>(DEFAULT_CSV_OPTIONS);
  const e = texp;

  const [label, setLabel] = createSignal("");
  const askOptions = (stmtLabel: string) =>
    new Promise<CsvOptions | null>((resolve) => {
      optionsReq()?.(null);
      setLabel(stmtLabel);
      setOptionsReq(() => resolve);
    });
  const finishOptions = (o: CsvOptions | null) => {
    const r = optionsReq();
    setOptionsReq(null);
    r?.(o);
  };
  let session: string | null = null;

  /** `relation`: varsa varsayılan dosya adında tablo adı olarak kullanılır. */
  async function start(input: { session: string; sql: string; relation: string | null; stmtIndex: number }): Promise<ExportOutcome> {
    session = input.session;
    setRows(0);
    const outcome = await runExport(
      {
        askOptions,
        pickPath: (defaultName) => save({ defaultPath: defaultName, filters: [{ name: e.filterName, extensions: ["csv"] }], title: e.dialogTitle }),
        exportCsv: commands.qtExportCsv,
        onProgress: setRows,
        setRunning,
      },
      { ...input, now: new Date() },
    );
    session = null;
    if (outcome.kind === "done") {
      const s = outcome.summary;
      toast.show(e.done(s.rows, (s.bytes / 1024).toLocaleString("tr-TR", { maximumFractionDigits: 1 }), t.query.strip.ms(s.elapsedMs)), "success");
    } else if (outcome.kind === "cancelled") {
      toast.show(e.cancelled, "info");
    } else if (outcome.kind === "error") {
      toast.show([e.failed(outcome.error.message), outcome.hint].filter(Boolean).join(" "), "error");
    }
    return outcome;
  }

  const cancel = () => {
    if (session) void commands.qtCancel(session).catch((err) => toast.show(toPayload(err).message, "error"));
  };

  const View = () => (
    <>
      <Dialog
        open={optionsReq() !== null}
        onOpenChange={(open) => !open && finishOptions(null)}
        title={e.optionsTitle}
        width="360px"
        footer={
          <>
            <Button onClick={() => finishOptions(null)}>{t.server.cancel}</Button>
            <Button variant="primary" onClick={() => finishOptions(opts())}>
              {e.start}
            </Button>
          </>
        }
      >
        <div class="flex flex-col gap-3">
          <p class="break-all font-mono text-xs text-muted">{label()}</p>
          <Field label={e.delimiter}>
            <select
              class={inputClass}
              value={opts().delimiter}
              onChange={(ev) => setOpts({ ...opts(), delimiter: ev.currentTarget.value as CsvDelimiter })}
            >
              <For each={DELIMITERS}>{(d) => <option value={d}>{e.delimiters[d]}</option>}</For>
            </select>
          </Field>
          <label class="flex items-center gap-2 text-[13px]">
            <input type="checkbox" checked={opts().header} onChange={(ev) => setOpts({ ...opts(), header: ev.currentTarget.checked })} />
            {e.header}
          </label>
          <label class="flex items-center gap-2 text-[13px]">
            <input type="checkbox" checked={opts().bom} onChange={(ev) => setOpts({ ...opts(), bom: ev.currentTarget.checked })} />
            {e.bom}
          </label>
        </div>
      </Dialog>
      <Dialog
        open={running()}
        // Kapatma = iptal isteği; diyalog iş bitince (`setRunning(false)`) kapanır.
        onOpenChange={(open) => !open && cancel()}
        title={e.progressTitle}
        width="320px"
        footer={<Button onClick={cancel}>{e.cancel}</Button>}
      >
        <p role="status" aria-live="polite">{e.progress(rows())}</p>
      </Dialog>
    </>
  );

  return { start, View };
}
