import { For, Show, createResource, createSignal, onCleanup } from "solid-js";
import { t } from "../../app/strings";
import { toast } from "../../app/toast";
import type { HistoryEntry } from "../../ipc/bindings/HistoryEntry";
import { commands, type Api } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { ConfirmDialog } from "../../ui/ConfirmDialog";
import { inputClass } from "../../ui/Field";
import { Button } from "../../ui/Button";
import { displaySql, searchQuery } from "./history-model";

const LIST_LIMIT = 500;

/** Geçmiş listesi (arama + tıklayınca `onInsert(sql)`); `refreshKey` değişince yeniden yüklenir. */
export function HistoryView(props: {
  onInsert: (sql: string) => void;
  /** Her `scriptDone` sonrası çağıran artırır. */
  refreshKey?: number;
  api?: Pick<Api, "qtHistoryList">;
}) {
  const api = () => props.api ?? commands;
  const [input, setInput] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  const onInput = (v: string) => {
    setInput(v);
    clearTimeout(timer);
    timer = setTimeout(() => setDebounced(v), 200);
  };
  const [entries] = createResource(
    () => ({ q: searchQuery(debounced()), k: props.refreshKey ?? 0 }),
    async ({ q }): Promise<HistoryEntry[]> => {
      try {
        return await api().qtHistoryList(q, LIST_LIMIT);
      } catch (e) {
        toast.show(toPayload(e).message, "error");
        return [];
      }
    },
  );
  const h = t.query.history;
  return (
    <div role="region" class="flex h-full flex-col" aria-label={h.ariaLabel}>
      <div class="border-b border-border p-2">
        <input
          class={inputClass}
          type="search"
          placeholder={h.search}
          aria-label={h.search}
          value={input()}
          onInput={(e) => onInput(e.currentTarget.value)}
        />
      </div>
      <ul class="min-h-0 flex-1 overflow-auto">
        <Show
          when={(entries() ?? []).length > 0}
          fallback={<li class="p-3 text-xs text-muted">{entries.loading ? t.query.history.loading : h.empty}</li>}
        >
          <For each={entries()}>
            {(e) => (
              <li>
                <button
                  type="button"
                  class="block w-full border-b border-border px-2 py-1.5 text-left hover:bg-hover"
                  title={h.insertHint}
                  onClick={() => props.onInsert(e.sql)}
                >
                  <div class="flex gap-2 text-[11px] text-muted">
                    <span>{new Date(e.atMs).toLocaleString("tr-TR")}</span>
                    <span class="truncate">{e.serverName}{e.database ? ` / ${e.database}` : ""}</span>
                    <Show when={!e.ok}>
                      <span class="text-danger">{h.failed}</span>
                    </Show>
                    <Show when={e.ok}>
                      <span class="ml-auto">{t.query.strip.ms(e.elapsedMs)}</span>
                    </Show>
                  </div>
                  <pre class="mt-0.5 line-clamp-3 whitespace-pre-wrap break-all font-mono text-xs">{displaySql(e.sql)}</pre>
                </button>
              </li>
            )}
          </For>
        </Show>
      </ul>
    </div>
  );
}

/** Ayarlar bölümü: "Sorgu geçmişini kaydet" anahtarı ve onaylı "Geçmişi/taslakları temizle". */
export function HistorySettings(props: {
  enabled: boolean;
  onToggle: (enabled: boolean) => void;
  onCleared?: () => void;
  api?: Pick<Api, "qtHistoryClear" | "qtDraftsClear">;
}) {
  const api = () => props.api ?? commands;
  const [confirming, setConfirming] = createSignal(false);
  const h = t.query.history;
  const clear = async () => {
    setConfirming(false);
    try {
      await api().qtHistoryClear();
      await api().qtDraftsClear();
      props.onCleared?.();
    } catch (e) {
      toast.show(toPayload(e).message, "error");
    }
  };
  return (
    <div class="flex flex-col gap-2">
      <label class="flex items-center gap-2 text-[13px]">
        <input type="checkbox" checked={props.enabled} onChange={(e) => props.onToggle(e.currentTarget.checked)} />
        {h.saveToggle}
      </label>
      <div>
        <Button onClick={() => setConfirming(true)}>{h.clear}</Button>
      </div>
      <ConfirmDialog
        open={confirming()}
        title={h.clearTitle}
        message={h.clearMessage}
        confirmLabel={h.clearConfirm}
        danger
        onConfirm={clear}
        onCancel={() => setConfirming(false)}
      />
    </div>
  );
}
