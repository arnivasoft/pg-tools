import { open as openFile } from "@tauri-apps/plugin-dialog";
import { For, Show, createEffect, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { timp } from "../../app/strings-import";
import { toast } from "../../app/toast";
import type { ImportItem } from "../../ipc/bindings/ImportItem";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { canImport, defaultSelection, groupLabel, summarize } from "./import-model";

export function ImportDialog(props: { open: boolean; onClose: () => void; onImported: () => void }) {
  const [path, setPath] = createSignal<string | null>(null);
  const [items, setItems] = createSignal<ImportItem[]>([]);
  const [selected, setSelected] = createSignal<Set<number>>(new Set());
  const [busy, setBusy] = createSignal(false);
  const [reading, setReading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [openWarnings, setOpenWarnings] = createSignal<Set<number>>(new Set());
  let generation = 0;

  createEffect(() => {
    void props.open; // her açılış/kapanışta sıfırla
    generation++;
    setPath(null);
    setItems([]);
    setSelected(new Set<number>());
    setBusy(false);
    setReading(false);
    setError(null);
    setOpenWarnings(new Set<number>());
  });

  const choose = async () => {
    const gen = generation;
    setError(null);
    try {
      const picked = await openFile({ multiple: false, filters: [{ name: timp.fileFilter, extensions: ["json"] }] });
      if (typeof picked !== "string" || gen !== generation) return;
      setReading(true);
      const list = await commands.previewPgadminImport(picked);
      if (gen !== generation) return;
      setPath(picked);
      setOpenWarnings(new Set<number>());
      setItems(list);
      setSelected(defaultSelection(list));
    } catch (e) {
      if (gen === generation) setError(toPayload(e).message);
    } finally {
      if (gen === generation) setReading(false);
    }
  };

  const toggleWarnings = (index: number) =>
    setOpenWarnings((s) => {
      const next = new Set(s);
      if (!next.delete(index)) next.add(index);
      return next;
    });

  const toggle = (index: number, on: boolean) =>
    setSelected((s) => {
      const next = new Set(s);
      if (on) next.add(index);
      else next.delete(index);
      return next;
    });

  const apply = async () => {
    const p = path();
    if (p === null || busy() || !canImport(items(), selected())) return;
    const gen = generation;
    setBusy(true);
    setError(null);
    try {
      const valid = new Set(items().filter((i) => i.error === null).map((i) => i.index));
      const count = await commands.applyPgadminImport(p, [...selected()].filter((i) => valid.has(i)));
      toast.show(timp.done(count), "success");
      props.onImported();
      props.onClose();
    } catch (e) {
      if (gen !== generation) return;
      setError(toPayload(e).message);
      // Kayıtlar tek tek yazılır: hata öncesindekiler aktarılmış olabilir. Ağacı ve önizlemeyi
      // yenile ki aktarılanlar "zaten var" olarak işaretlensin ve yeniden denemede kopya oluşmasın.
      props.onImported();
      try {
        const list = await commands.previewPgadminImport(p);
        if (gen === generation) {
          setItems(list);
          setSelected(defaultSelection(list));
        }
      } catch {
        // önizleme yenilenemedi; ilk hata iletisi gösterilmeye devam eder
      }
    } finally {
      if (gen === generation) setBusy(false);
    }
  };

  const count = () => items().filter((i) => i.error === null && selected().has(i.index)).length;

  return (
    <Dialog
      open={props.open}
      onOpenChange={(o) => !o && !busy() && props.onClose()}
      title={t.import.title}
      width="760px"
      footer={
        <>
          <Button disabled={busy()} onClick={props.onClose}>{timp.cancel}</Button>
          <Button variant="primary" disabled={busy() || reading() || !canImport(items(), selected())} onClick={() => void apply()}>
            {timp.doImport(count())}
          </Button>
        </>
      }
    >
      <div class="flex flex-col gap-3">
        <div class="flex items-center gap-2">
          <Button disabled={reading() || busy()} onClick={() => void choose()}>
            {reading() ? timp.reading : timp.choose}
          </Button>
          <Show when={path()}>
            <span class="truncate text-xs text-muted" title={path() ?? ""}>
              {path()}
            </span>
          </Show>
        </div>
        <Show when={error()}>
          <p class="text-[13px] text-danger">{error()}</p>
        </Show>
        <Show when={items().length > 0}>
          <div class="flex items-center gap-2 text-[13px]">
            <span class="text-muted">{timp.summary(summarize(items()))}</span>
            <div class="flex-1" />
            <Button variant="ghost" onClick={() => setSelected(defaultSelection(items()))}>
              {timp.selectRecommended}
            </Button>
            <Button variant="ghost" onClick={() => setSelected(new Set<number>())}>
              {timp.selectNone}
            </Button>
          </div>
          <table class="w-full text-left text-[13px]">
            <thead class="text-xs text-muted">
              <tr>
                <th class="w-8" />
                <th>{timp.colName}</th>
                <th>{timp.colGroup}</th>
                <th>{timp.colHost}</th>
                <th>{timp.colUser}</th>
                <th>{timp.colStatus}</th>
              </tr>
            </thead>
            <tbody>
              <For each={items()}>
                {(it) => (
                  <>
                    <tr class="border-t border-border">
                      <td>
                        <input
                          type="checkbox"
                          disabled={it.error !== null}
                          checked={selected().has(it.index)}
                          onChange={(e) => toggle(it.index, e.currentTarget.checked)}
                        />
                      </td>
                      <td>{it.name}</td>
                      <td>{groupLabel(it)}</td>
                      <td>
                        {it.host}:{it.port}
                      </td>
                      <td>{it.username}</td>
                      <td>
                        <Show
                          when={it.error}
                          fallback={
                            <>
                              <Show when={it.duplicate}>
                                <span class="text-muted">{timp.duplicate}</span>
                              </Show>
                              <Show when={it.warnings.length > 0}>
                                <button
                                  type="button"
                                  class="ml-1 rounded px-1 text-muted hover:bg-hover"
                                  aria-expanded={openWarnings().has(it.index)}
                                  aria-controls={openWarnings().has(it.index) ? `import-warnings-${it.index}` : undefined}
                                  aria-label={(openWarnings().has(it.index) ? timp.hideWarnings : timp.showWarnings)(it.warnings.length)}
                                  onClick={() => toggleWarnings(it.index)}
                                >
                                  {timp.warnBadge(it.warnings.length)}
                                </button>
                              </Show>
                            </>
                          }
                        >
                          <span class="text-danger">{it.error}</span>
                        </Show>
                      </td>
                    </tr>
                    <Show when={it.error === null && it.warnings.length > 0 && openWarnings().has(it.index)}>
                      <tr id={`import-warnings-${it.index}`}>
                        <td />
                        <td colSpan={5} class="pb-1">
                          <p class="text-xs text-muted">{timp.warnings}:</p>
                          <ul class="list-disc pl-5 text-xs text-muted">
                            <For each={it.warnings}>{(w) => <li>{w}</li>}</For>
                          </ul>
                        </td>
                      </tr>
                    </Show>
                  </>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </div>
    </Dialog>
  );
}
