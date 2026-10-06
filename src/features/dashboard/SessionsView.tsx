// Oturumlar bölümü: araç çubuğu (arama, durum/etkin/boşta-tx/veritabanı süzgeçleri), sanal tablo ve ayrıntı paneli.
// Süzgeç ve sıralama görünümde durur; veri `model.activity` alanından gelir (yenilemeyi denetleyici yapar, bölüm seçiliyken
// liste 5 sn'de bir çekilir). İptal/Sonlandır doğrudan göndermez: `confirmSignal` onay diyaloğunu açar.
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { copyText } from "../../app/clipboard";
import { td } from "../../app/strings-dashboard";
import { toast } from "../../app/toast";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import { DataTable, type DataColumn } from "./DataTable";
import type { SectionProps } from "./DashboardView";
import { Banner, Duration, SignalButtons, StateBadge } from "./parts";
import {
  filterActivity, nextSort, oneLine, sortActivity, waitLabel, type ActivityFilter, type ActivityKey, type SortState,
} from "./table-model";

/** "Boşta-tx ≥" seçenekleri (sn). */
const IDLE_TX_OPTIONS = [1, 5, 10, 30, 60, 300];

const inputClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent";

export default function SessionsView(props: SectionProps) {
  const s = td.sessions;
  const [search, setSearch] = createSignal("");
  const [states, setStates] = createSignal<Set<string>>(new Set());
  const [onlyActive, setOnlyActive] = createSignal(false);
  const [idleTx, setIdleTx] = createSignal<number | null>(null);
  const [database, setDatabase] = createSignal<string | null>(null);
  const [sort, setSort] = createSignal<SortState | null>(null);
  const [selected, setSelected] = createSignal<number | null>(null);
  let detailEl: HTMLDivElement | undefined;

  const all = createMemo(() => props.model.activity?.rows ?? []);
  const filter = (): ActivityFilter => ({
    search: search(), states: states(), onlyActive: onlyActive(), idleInTxOverSecs: idleTx(), database: database(),
  });
  const rows = createMemo(() => {
    const f = filterActivity(all(), filter());
    const so = sort();
    return so ? sortActivity(f, so.key as ActivityKey, so.dir) : f;
  });
  const stateNames = createMemo(() => [...new Set(all().map((r) => r.state).filter((x): x is string => x != null))].sort());
  const dbNames = createMemo(() => [...new Set(all().map((r) => r.database).filter((x): x is string => x != null))].sort());
  // Ayrıntı, Delete ve vurgu aynı (süzülmüş) listeden gelir: seçili satır listeden çıkarsa seçim temizlenir.
  const current = createMemo(() => rows().find((r) => r.pid === selected()) ?? null);
  createEffect(() => {
    if (selected() !== null && current() === null) setSelected(null);
  });

  const toggleState = (name: string) =>
    setStates((cur) => {
      const next = new Set(cur);
      if (!next.delete(name)) next.add(name);
      return next;
    });

  const ask = (row: ActivityRow, kind: "cancel" | "terminate") => {
    if (row.isSelf || props.model.signaling) return;
    void props.dash.confirmSignal(props.model.id, row, kind);
  };

  const columns: DataColumn<ActivityRow>[] = [
    { key: "pid", header: s.cols.pid, width: 64, align: "right", sortable: true, cell: (r) => r.pid },
    { key: "database", header: s.cols.database, width: 110, sortable: true, cell: (r) => r.database ?? "" },
    { key: "user", header: s.cols.user, width: 100, sortable: true, cell: (r) => r.user ?? "" },
    { key: "application", header: s.cols.application, width: 130, sortable: true, cell: (r) => r.application },
    { key: "client", header: s.cols.client, width: 110, sortable: true, cell: (r) => r.clientAddr ?? "" },
    { key: "state", header: s.cols.state, width: 150, sortable: true, cell: (r) => <StateBadge state={r.state} /> },
    { key: "wait", header: s.cols.wait, width: 140, sortable: true, cell: (r) => waitLabel(r) },
    { key: "xactAge", header: s.cols.xactAge, width: 90, align: "right", sortable: true, cell: (r) => <Duration secs={r.xactAgeSecs} /> },
    { key: "queryAge", header: s.cols.queryAge, width: 95, align: "right", sortable: true, cell: (r) => <Duration secs={r.queryAgeSecs} /> },
    { key: "stateAge", header: s.cols.stateAge, width: 95, align: "right", sortable: true, cell: (r) => <Duration secs={r.stateAgeSecs} /> },
    {
      key: "query", header: s.cols.query, width: 320,
      cell: (r) =>
        r.query == null ? <span class="italic text-muted">{td.noPrivilege}</span> : <span class="font-mono">{oneLine(r.query)}</span>,
    },
    {
      key: "actions", header: s.cols.actions, width: 130,
      cell: (r) => <SignalButtons dash={props.dash} tabId={props.model.id} row={r} signaling={props.model.signaling} />,
    },
  ];

  const copy = (q: string) => {
    void copyText(q).then(
      () => toast.show(s.copied, "success"),
      () => toast.show(s.copyFailed, "error"),
    );
  };

  return (
    <div class="flex h-full min-h-0 flex-col">
      <div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <input
          type="search"
          class={`${inputClass} w-64`}
          placeholder={s.search}
          aria-label={s.search}
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
        />
        <label class="flex items-center gap-1 text-[12px]">
          <input type="checkbox" checked={onlyActive()} onChange={(e) => setOnlyActive(e.currentTarget.checked)} />
          {s.onlyActive}
        </label>
        <select
          class={inputClass}
          aria-label={s.idleInTx}
          title={s.idleInTx}
          value={idleTx() == null ? "" : String(idleTx())}
          onChange={(e) => setIdleTx(e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}
        >
          <option value="">{s.idleInTxAny}</option>
          <For each={IDLE_TX_OPTIONS}>{(n) => <option value={String(n)}>{s.idleInTxOver(n)}</option>}</For>
        </select>
        <select
          class={inputClass}
          aria-label={s.database}
          title={s.database}
          value={database() ?? ""}
          onChange={(e) => setDatabase(e.currentTarget.value === "" ? null : e.currentTarget.value)}
        >
          <option value="">{s.allDatabases}</option>
          <For each={dbNames()}>{(n) => <option value={n}>{n}</option>}</For>
        </select>
        <div role="group" aria-label={s.states} class="flex flex-wrap items-center gap-1">
          <For each={stateNames()}>
            {(name) => (
              <button
                type="button"
                aria-pressed={states().has(name)}
                class="h-6 rounded-full border px-2 text-[11px] hover:bg-hover"
                classList={{ "border-accent bg-selected": states().has(name), "border-border": !states().has(name) }}
                onClick={() => toggleState(name)}
              >
                {name}
              </button>
            )}
          </For>
        </div>
        <span class="ml-auto text-[12px] text-muted" aria-live="polite">
          {s.count(rows().length, all().length)}
        </span>
      </div>
      <Show when={props.model.sectionError.sessions}>{(e) => <Banner kind="error">{e()}</Banner>}</Show>
      <Show when={props.model.activity?.truncated}>
        <Banner kind="info">{s.truncated}</Banner>
      </Show>
      <Show
        when={props.model.activity}
        fallback={<p class="p-3 text-muted">{props.model.sectionError.sessions ? "" : s.waiting}</p>}
      >
        <Show
          when={rows().length > 0}
          fallback={<p class="p-3 text-muted">{all().length === 0 ? s.empty : s.emptyFiltered}</p>}
        >
          <DataTable
            class="min-h-[120px] flex-1"
            ariaLabel={s.ariaLabel}
            columns={columns}
            rows={rows()}
            rowKey={(r) => r.pid}
            selectedKey={selected()}
            onSelect={(k) => setSelected(k as number | null)}
            onActivate={() => detailEl?.focus()}
            onDelete={(r) => ask(r, "cancel")}
            sort={sort()}
            onSort={(k) => setSort((cur) => nextSort(cur, k))}
          />
        </Show>
      </Show>
      <div
        ref={detailEl}
        tabIndex={-1}
        class="max-h-[34%] min-h-[64px] shrink-0 overflow-auto border-t border-border bg-panel px-3 py-2 outline-none"
      >
        <Show when={current()} fallback={<p class="text-[12px] text-muted">{s.selectHint}</p>}>
          {(r) => (
            <div class="flex flex-col gap-1.5">
              <div class="flex items-center gap-2 text-[12px]">
                <span class="font-semibold">{s.detail}</span>
                <span class="text-muted">
                  pid {r().pid}
                  {r().user ? ` · ${r().user}` : ""}
                </span>
                <div class="ml-auto flex items-center gap-2">
                  <Show when={r().query != null}>
                    <button
                      type="button"
                      class="h-5 rounded border border-border bg-bg px-1.5 text-[11px] hover:bg-hover"
                      onClick={() => copy(r().query ?? "")}
                    >
                      {s.copy}
                    </button>
                  </Show>
                  <SignalButtons dash={props.dash} tabId={props.model.id} row={r()} signaling={props.model.signaling} />
                </div>
              </div>
              <Show when={r().query != null} fallback={<p class="text-[12px] italic text-muted">{s.noQuery}</p>}>
                <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px]">{r().query || s.emptyQuery}</pre>
              </Show>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
