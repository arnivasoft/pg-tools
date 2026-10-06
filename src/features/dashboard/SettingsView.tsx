// Yapılandırma bölümü: pg_settings tablosu (arama, "yalnızca değiştirilmiş", "yeniden başlatma bekliyor" süzgeçleri) ve altında
// hazır transaction'lar. Otomatik yenilenmez: bölüme girişte ve **Yenile** ile denetleyici yükler (`loadSettings`/`loadPrepared`).
import { RefreshCw } from "lucide-solid";
import { Show, createMemo, createSignal } from "solid-js";
import { td } from "../../app/strings-dashboard";
import type { PreparedXact } from "../../ipc/bindings/PreparedXact";
import type { Setting } from "../../ipc/bindings/Setting";
import { Button } from "../../ui/Button";
import { DataTable, type DataColumn } from "./DataTable";
import type { SectionProps } from "./DashboardView";
import { formatDuration } from "./dash-model";
import { Banner } from "./parts";
import { filterSettings, formatSettingValue, nextSort, preparedAgeBadge, type SortState } from "./table-model";

const inputClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent";

const badgeClass = "inline-flex h-[18px] items-center rounded-full border px-1.5 text-[11px]";

export default function SettingsView(props: SectionProps) {
  const s = td.settings;
  const p = td.prepared;
  const [search, setSearch] = createSignal("");
  const [modifiedOnly, setModifiedOnly] = createSignal(false);
  const [pendingOnly, setPendingOnly] = createSignal(false);
  const [sort, setSort] = createSignal<SortState | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);

  const all = createMemo(() => props.model.settings ?? []);
  const rows = createMemo(() => {
    const f = filterSettings(all(), { search: search(), modifiedOnly: modifiedOnly(), pendingRestartOnly: pendingOnly() });
    const so = sort();
    if (!so) return f;
    const sign = so.dir === "asc" ? 1 : -1;
    const key = (r: Setting) => (so.key === "value" ? formatSettingValue(r) : r[so.key as "name" | "source" | "context"]);
    return [...f].sort((a, b) => sign * key(a).localeCompare(key(b), "tr", { numeric: true, sensitivity: "base" }));
  });

  const columns: DataColumn<Setting>[] = [
    { key: "name", header: s.cols.name, width: 230, sortable: true, cell: (r) => <span class="font-mono">{r.name}</span> },
    {
      key: "value", header: s.cols.value, width: 190, sortable: true,
      cell: (r) =>
        r.value == null ? (
          <span class="italic text-muted" title={td.noPrivilege}>
            —
          </span>
        ) : (
          <span class="font-mono">{formatSettingValue(r)}</span>
        ),
    },
    { key: "source", header: s.cols.source, width: 120, sortable: true, cell: (r) => r.source },
    { key: "context", header: s.cols.context, width: 110, sortable: true, cell: (r) => r.context },
    {
      key: "pending", header: s.cols.pending, width: 170,
      cell: (r) =>
        r.pendingRestart ? (
          <span class={`${badgeClass} border-[color:var(--srv-amber)] text-warn-text`} title={s.pendingRestartTitle}>
            {s.pendingRestart}
          </span>
        ) : (
          ""
        ),
    },
    { key: "description", header: s.cols.description, width: 560, cell: (r) => <span title={r.description}>{r.description}</span> },
  ];

  const preparedColumns: DataColumn<PreparedXact>[] = [
    { key: "gid", header: p.cols.gid, width: 260, cell: (r) => <span class="font-mono">{r.gid}</span> },
    { key: "transaction", header: p.cols.transaction, width: 110, align: "right", cell: (r) => r.transaction },
    { key: "owner", header: p.cols.owner, width: 120, cell: (r) => r.owner },
    { key: "database", header: p.cols.database, width: 140, cell: (r) => r.database },
    {
      key: "age", header: p.cols.age, width: 260,
      cell: (r) => (
        <span class="inline-flex items-center gap-2">
          <span>{formatDuration(r.ageSecs)}</span>
          <Show when={preparedAgeBadge(r.ageSecs) === "old"}>
            <span class={`${badgeClass} border-danger text-danger`} title={p.oldTitle}>
              {p.old}
            </span>
          </Show>
        </span>
      ),
    },
  ];

  const refresh = () => {
    void props.dash.loadSettings(props.model.id);
    void props.dash.loadPrepared(props.model.id);
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
          <input type="checkbox" checked={modifiedOnly()} onChange={(e) => setModifiedOnly(e.currentTarget.checked)} />
          {s.modifiedOnly}
        </label>
        <label class="flex items-center gap-1 text-[12px]">
          <input type="checkbox" checked={pendingOnly()} onChange={(e) => setPendingOnly(e.currentTarget.checked)} />
          {s.pendingRestartOnly}
        </label>
        <Button variant="ghost" title={s.refreshTitle} onClick={refresh}>
          <RefreshCw size={14} /> {s.refresh}
        </Button>
        <span class="ml-auto text-[12px] text-muted" aria-live="polite">
          {s.count(rows().length, all().length)}
        </span>
      </div>
      <Show when={props.model.sectionError.settings}>{(e) => <Banner kind="error">{e()}</Banner>}</Show>
      <Show
        when={props.model.settings}
        fallback={<p class="p-3 text-muted">{props.model.sectionError.settings ? "" : s.waiting}</p>}
      >
        <Show when={rows().length > 0} fallback={<p class="p-3 text-muted">{all().length === 0 ? s.empty : s.emptyFiltered}</p>}>
          <DataTable
            class="min-h-[120px] flex-1"
            ariaLabel={s.ariaLabel}
            columns={columns}
            rows={rows()}
            rowKey={(r) => r.name}
            selectedKey={selected()}
            onSelect={(k) => setSelected(k as string | null)}
            sort={sort()}
            onSort={(k) => setSort((cur) => nextSort(cur, k))}
          />
        </Show>
      </Show>
      <section aria-label={p.title} class="max-h-[40%] min-h-[96px] shrink-0 overflow-auto border-t border-border bg-panel">
        <div class="flex items-center gap-2 px-3 py-1.5 text-[12px]">
          <span class="font-semibold">{p.title}</span>
          <span class="text-muted">{p.note}</span>
        </div>
        <Show when={props.model.sectionError.prepared}>{(e) => <Banner kind="error">{e()}</Banner>}</Show>
        <Show
          when={props.model.prepared}
          fallback={<p class="px-3 pb-2 text-[12px] text-muted">{props.model.sectionError.prepared ? "" : p.waiting}</p>}
        >
          {(list) => (
            <Show when={list().length > 0} fallback={<p class="px-3 pb-2 text-[12px] text-muted">{p.empty}</p>}>
              <DataTable class="max-h-44" ariaLabel={p.ariaLabel} columns={preparedColumns} rows={list()} rowKey={(r) => r.gid} />
            </Show>
          )}
        </Show>
      </section>
    </div>
  );
}
