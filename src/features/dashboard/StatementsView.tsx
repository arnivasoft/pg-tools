// Sorgu istatistikleri bölümü (pg_stat_statements): sıralama seçici, sanal tablo ve ayrıntı paneli. Eklenti yok/yüklü değilse
// `reason` arka uçtan geldiği gibi boş durum olarak gösterilir (arayüz yorumlamaz). Veri liste aralığıyla yenilenir.
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { copyText } from "../../app/clipboard";
import { td } from "../../app/strings-dashboard";
import { toast } from "../../app/toast";
import type { StatementRow } from "../../ipc/bindings/StatementRow";
import type { StatementsOrder } from "../../ipc/bindings/StatementsOrder";
import { DataTable, type DataColumn } from "./DataTable";
import type { SectionProps } from "./DashboardView";
import { formatCount } from "./dash-model";
import { Banner } from "./parts";
import { formatMs, formatRatio, hitRatio, nextSort, oneLine, type SortState } from "./table-model";

const ORDERS: StatementsOrder[] = ["total_time", "mean_time", "calls"];

const inputClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent";

interface Item {
  row: StatementRow;
  key: string;
}

/** Satır kimliği: `queryId`; yoksa (yetkisiz satır) dizin. Seçim yenilemeler arasında bununla izlenir. */
const keyOf = (r: StatementRow, i: number) => r.queryId ?? `#${i}`;

function numOf(r: StatementRow, k: string): number {
  switch (k) {
    case "calls": return r.calls;
    case "totalMs": return r.totalMs;
    case "meanMs": return r.meanMs;
    case "rows": return r.rows;
    case "hit": return r.sharedBlksHit;
    case "read": return r.sharedBlksRead;
    default: return hitRatio(r.sharedBlksHit, r.sharedBlksRead) ?? -1;
  }
}

export default function StatementsView(props: SectionProps) {
  const s = td.statements;
  const [sort, setSort] = createSignal<SortState | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);
  let detailEl: HTMLDivElement | undefined;

  // Başvuru önce: model her yamada yeni nesnedir; liste yalnızca yeni `statements` gelince yeniden türetilir.
  const result = createMemo(() => props.model.statements);
  const keyed = createMemo<Item[]>(() => {
    const r = result();
    return r?.kind === "available" ? r.rows.map((row, i) => ({ row, key: keyOf(row, i) })) : [];
  });
  // Sunucu sıralaması varsayılan; başlığa tıklayınca yalnızca görünen liste yerelde yeniden sıralanır.
  const items = createMemo(() => {
    const so = sort();
    if (!so) return keyed();
    const sign = so.dir === "asc" ? 1 : -1;
    return [...keyed()].sort((a, b) => sign * (numOf(a.row, so.key) - numOf(b.row, so.key)));
  });
  const current = createMemo(() => items().find((it) => it.key === selected()) ?? null);
  createEffect(() => {
    if (selected() !== null && current() === null) setSelected(null);
  });

  const columns: DataColumn<Item>[] = [
    {
      key: "query", header: s.cols.query, width: 420,
      cell: (it) =>
        it.row.query == null ? <span class="italic text-muted">{td.noPrivilege}</span> : <span class="font-mono">{oneLine(it.row.query)}</span>,
    },
    { key: "calls", header: s.cols.calls, width: 90, align: "right", sortable: true, cell: (it) => formatCount(it.row.calls) },
    { key: "totalMs", header: s.cols.totalMs, width: 110, align: "right", sortable: true, cell: (it) => formatMs(it.row.totalMs) },
    { key: "meanMs", header: s.cols.meanMs, width: 110, align: "right", sortable: true, cell: (it) => formatMs(it.row.meanMs) },
    { key: "rows", header: s.cols.rows, width: 90, align: "right", sortable: true, cell: (it) => formatCount(it.row.rows) },
    { key: "hit", header: s.cols.hit, width: 90, align: "right", sortable: true, cell: (it) => formatCount(it.row.sharedBlksHit) },
    { key: "read", header: s.cols.read, width: 90, align: "right", sortable: true, cell: (it) => formatCount(it.row.sharedBlksRead) },
    {
      key: "ratio", header: s.cols.ratio, width: 90, align: "right", sortable: true,
      cell: (it) => formatRatio(hitRatio(it.row.sharedBlksHit, it.row.sharedBlksRead)),
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
        <label class="flex items-center gap-1 text-[12px]">
          {s.order}
          <select
            class={inputClass}
            value={props.model.statementsOrder}
            onChange={(e) => {
              setSort(null);
              props.dash.setStatementsOrder(props.model.id, e.currentTarget.value as StatementsOrder);
            }}
          >
            <For each={ORDERS}>{(o) => <option value={o}>{s.orders[o]}</option>}</For>
          </select>
        </label>
        <Show when={result()?.kind === "available"}>
          <span class="ml-auto text-[12px] text-muted" aria-live="polite">
            {s.count(keyed().length)}
          </span>
        </Show>
      </div>
      <Show when={props.model.sectionError.statements}>{(e) => <Banner kind="error">{e()}</Banner>}</Show>
      <Show when={result()} fallback={<p class="p-3 text-muted">{props.model.sectionError.statements ? "" : s.waiting}</p>}>
        {(r) => {
          const unavailable = () => {
            const v = r();
            return v.kind === "unavailable" ? v.reason : null;
          };
          return (
            <Show
              when={unavailable() === null}
              fallback={
                <div class="p-3 text-[13px]">
                  <p class="font-medium">{s.unavailable}</p>
                  <p class="mt-1 text-muted">{unavailable()}</p>
                </div>
              }
            >
              <Show when={keyed().length > 0} fallback={<p class="p-3 text-muted">{s.empty}</p>}>
                <DataTable
                  class="min-h-[120px] flex-1"
                  ariaLabel={s.ariaLabel}
                  columns={columns}
                  rows={items()}
                  rowKey={(it) => it.key}
                  selectedKey={selected()}
                  onSelect={(k) => setSelected(k as string | null)}
                  onActivate={() => detailEl?.focus()}
                  sort={sort()}
                  onSort={(k) => setSort((cur) => nextSort(cur, k))}
                />
              </Show>
              <div
                ref={detailEl}
                tabIndex={-1}
                class="max-h-[34%] min-h-[64px] shrink-0 overflow-auto border-t border-border bg-panel px-3 py-2 outline-none"
              >
                <Show when={current()} fallback={<p class="text-[12px] text-muted">{s.selectHint}</p>}>
                  {(it) => (
                    <div class="flex flex-col gap-1.5">
                      <div class="flex items-center gap-2 text-[12px]">
                        <span class="font-semibold">{s.detail}</span>
                        <Show when={it().row.query != null}>
                          <button
                            type="button"
                            class="ml-auto h-5 rounded border border-border bg-bg px-1.5 text-[11px] hover:bg-hover"
                            onClick={() => copy(it().row.query ?? "")}
                          >
                            {s.copy}
                          </button>
                        </Show>
                      </div>
                      <Show when={it().row.query != null} fallback={<p class="text-[12px] italic text-muted">{td.sessions.noQuery}</p>}>
                        <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px]">
                          {it().row.query || s.emptyQuery}
                        </pre>
                      </Show>
                    </div>
                  )}
                </Show>
              </div>
            </Show>
          );
        }}
      </Show>
    </div>
  );
}
