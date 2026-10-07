// Veri sekmesi araç çubuğu (tembel DataTabs parçası): yeniden yükle, satır ekle/sil, NULL/DEFAULT, tüm düzenlemeleri at,
// kaydet/iptal, filtre ve sıralama özeti. `src/ui/*` bileşenleri içe aktarılmaz (paylaşılan parça doğmasın; paket bütçesi):
// düğme sınıfları `ui/Button`'dan kopyadır. Filtre metni yalnızca `data_select`'e gider (günlük/depo yok).
import { For, Show, createEffect, createSignal, createUniqueId, on } from "solid-js";
import { tdata } from "../../app/strings-data";
import type { DataModel } from "./data-controller";
import type { GridCommands } from "./DataGrid";

/** `ui/Button` ikincil/birincil sınıflarının kopyası. */
export const BTN =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded border border-border bg-bg px-2.5 text-[13px] hover:bg-hover disabled:opacity-50 disabled:hover:bg-bg";
export const BTN_PRIMARY =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded bg-accent px-2.5 text-[13px] text-accent-fg hover:brightness-110 disabled:opacity-50 disabled:hover:brightness-100";
export const BTN_DANGER =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded bg-danger px-2.5 text-[13px] text-danger-fg hover:brightness-110";

export function DataToolbar(props: {
  m: DataModel;
  /** Düzenleme komutları kapalıysa gerekçesi. */
  blocked: string | null;
  /** Etkin hücrenin satırı silinecek mi (düğme metni). */
  activeDeleted: boolean;
  hasActive: boolean;
  dirty: boolean;
  /** Kaydet kapalıysa gerekçesi. */
  saveReason: string | null;
  cmd: GridCommands;
  onReload: () => void;
  onDiscard: () => void;
  onSave: () => void;
  onCancelApply: () => void;
  onFilter: (filter: string) => void;
  onClearSort: () => void;
}) {
  const uid = createUniqueId();
  const [draft, setDraft] = createSignal(props.m.select.filter);
  // Uygulanan filtre değişince (başarısız yüklemede geri dönüş dahil) taslak ona eşitlenir.
  createEffect(on(() => props.m.select.filter, (f) => setDraft(f), { defer: true }));

  const busy = () => props.m.applying || props.m.loadingMore || props.m.status === "loading" || props.m.status === "gone";
  const cellReason = () => props.blocked ?? (props.hasActive ? null : tdata.blocked.noCell);
  const insertReason = () => props.blocked ?? (props.m.loaded?.source.canInsert ? null : tdata.blocked.readOnly);
  const deleteReason = () => cellReason() ?? (props.m.loaded?.source.canDelete ? null : tdata.blocked.readOnly);
  const filterErr = () => props.m.selectErrors.filter;
  const orderErr = () => [...props.m.selectErrors.order, ...props.m.selectErrors.general];

  const sortText = () =>
    props.m.select.order.map((o) => `${o.column} ${o.desc ? tdata.sort.desc : tdata.sort.asc}`).join(", ");

  return (
    <div class="flex shrink-0 flex-col gap-1.5 border-b border-border bg-panel px-2 py-1.5 text-[13px]">
      <div class="flex flex-wrap items-center gap-1.5">
        <button type="button" class={BTN} title={tdata.toolbar.reloadTitle} disabled={busy()} onClick={() => props.onReload()}>
          {tdata.toolbar.reload}
        </button>
        <span class="mx-1 h-5 w-px bg-border" aria-hidden="true" />
        <button
          type="button"
          class={BTN}
          title={insertReason() ?? tdata.toolbar.addRowTitle}
          disabled={insertReason() !== null}
          onClick={() => props.cmd.addRow()}
        >
          {tdata.toolbar.addRow}
        </button>
        <button
          type="button"
          class={BTN}
          title={deleteReason() ?? tdata.toolbar.deleteRowTitle}
          disabled={deleteReason() !== null}
          onClick={() => props.cmd.toggleDelete()}
        >
          {props.activeDeleted ? tdata.toolbar.undeleteRow : tdata.toolbar.deleteRow}
        </button>
        <button
          type="button"
          class={BTN}
          title={cellReason() ?? tdata.toolbar.setNullTitle}
          disabled={cellReason() !== null}
          onClick={() => props.cmd.setNull()}
        >
          {tdata.toolbar.setNull}
        </button>
        <button
          type="button"
          class={BTN}
          title={cellReason() ?? tdata.toolbar.setDefaultTitle}
          disabled={cellReason() !== null}
          onClick={() => props.cmd.setDefault()}
        >
          {tdata.toolbar.setDefault}
        </button>
        <span class="mx-1 h-5 w-px bg-border" aria-hidden="true" />
        <button
          type="button"
          class={BTN}
          title={tdata.toolbar.discardTitle}
          disabled={!props.dirty || props.m.applying || props.m.status !== "ready"}
          onClick={() => props.onDiscard()}
        >
          {tdata.toolbar.discard}
        </button>
        <Show
          when={props.m.applying}
          fallback={
            <button
              type="button"
              class={BTN_PRIMARY}
              title={props.saveReason ?? tdata.toolbar.save}
              disabled={props.saveReason !== null}
              onClick={() => props.onSave()}
            >
              {tdata.toolbar.save}
            </button>
          }
        >
          <span role="status" class="text-muted">
            {tdata.toolbar.saving}
          </span>
          <button type="button" class={BTN} title={tdata.toolbar.cancelTitle} onClick={() => props.onCancelApply()}>
            {tdata.toolbar.cancel}
          </button>
        </Show>
      </div>
      <div class="flex flex-wrap items-center gap-1.5">
        <label for={`${uid}-f`} class="shrink-0 text-muted">
          {tdata.filter.label}
        </label>
        <input
          id={`${uid}-f`}
          type="text"
          spellcheck={false}
          class="h-7 min-w-40 flex-1 rounded border bg-bg px-2 font-mono text-[12px] outline-none focus:border-accent"
          classList={{ "border-danger": filterErr().length > 0, "border-border": filterErr().length === 0 }}
          placeholder={tdata.filter.placeholder}
          title={tdata.filter.example}
          aria-invalid={filterErr().length > 0 ? true : undefined}
          aria-describedby={filterErr().length > 0 ? `${uid}-fe` : undefined}
          value={draft()}
          disabled={busy()}
          onInput={(ev) => setDraft(ev.currentTarget.value)}
          onKeyDown={(ev) => {
            if (ev.key === "Enter") {
              ev.preventDefault();
              props.onFilter(draft());
            } else if (ev.key === "Escape" && draft() !== props.m.select.filter) {
              ev.preventDefault();
              setDraft(props.m.select.filter);
            }
          }}
        />
        <button type="button" class={BTN} disabled={busy()} onClick={() => props.onFilter(draft())}>
          {tdata.filter.apply}
        </button>
        <button
          type="button"
          class={BTN}
          disabled={busy() || (props.m.select.filter === "" && draft() === "")}
          onClick={() => {
            setDraft("");
            props.onFilter("");
          }}
        >
          {tdata.filter.clear}
        </button>
        <span class="ml-2 min-w-0 truncate text-muted" title={tdata.sort.hint}>
          {`${tdata.sort.label}: ${sortText() || tdata.sort.none}`}
        </span>
        <Show when={props.m.select.order.length > 0}>
          <button type="button" class={BTN} disabled={busy()} onClick={() => props.onClearSort()}>
            {tdata.sort.clear}
          </button>
        </Show>
      </div>
      <Show when={filterErr().length > 0}>
        <ul id={`${uid}-fe`} class="text-[12px] text-danger">
          <For each={filterErr()}>{(msg) => <li>{msg}</li>}</For>
        </ul>
      </Show>
      <Show when={orderErr().length > 0}>
        <ul role="alert" class="text-[12px] text-danger">
          <For each={orderErr()}>{(msg) => <li>{msg}</li>}</For>
        </ul>
      </Show>
    </div>
  );
}
