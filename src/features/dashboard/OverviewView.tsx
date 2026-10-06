// Genel (Overview) bölümü: özet kartları ve 5 Canvas grafiği. Veri `DashModel`'den gelir; yenilemeyi denetleyici yapar
// (sekme etkin değilken istek yok), grafik yalnızca veri değişince çizer.
import { For, Show, createMemo, type JSX } from "solid-js";
import { td } from "../../app/strings-dashboard";
import Chart from "./Chart";
import type { Series } from "./chart-model";
import { formatDuration } from "./dash-model";
import type { SectionProps } from "./DashboardView";

/** Bağlantı kullanımı bu oranın üstünde ise rozet gösterilir. */
const HIGH_USAGE = 0.8;

function Card(props: { label: string; title?: string; children: JSX.Element; badge?: JSX.Element; note?: JSX.Element }) {
  return (
    <div class="min-w-0 rounded border border-border bg-panel px-3 py-2" title={props.title}>
      <div class="text-[11px] text-muted">{props.label}</div>
      <div class="mt-0.5 flex items-center gap-2 text-[14px] font-medium">
        <span class="min-w-0 truncate">{props.children}</span>
        {props.badge}
      </div>
      {props.note}
    </div>
  );
}

export default function OverviewView(props: SectionProps) {
  const o = () => props.model.overview;
  const rates = () => props.model.rates;
  const last = () => {
    const s = props.model.ring.samples();
    return s.length ? s[s.length - 1] : null;
  };
  const sc = td.overview.series;
  const rate = (name: string, color: string, pts: { t: number; v: number | null }[]): Series => ({ name, color, points: pts });

  const sessions = createMemo<Series[]>(() => {
    const s = props.model.ring.samples();
    const pts = (f: (x: (typeof s)[number]) => number) => s.map((x) => ({ t: x.at, v: f(x) }));
    return [
      { name: sc.active, color: "--accent", stacked: true, points: pts((x) => x.sessionsActive) },
      { name: sc.idle, color: "--muted", stacked: true, points: pts((x) => x.sessionsIdle) },
      { name: sc.idleInTx, color: "--srv-amber", stacked: true, points: pts((x) => x.sessionsIdleInTx) },
    ];
  });
  const xacts = createMemo(() => [
    rate(sc.commit, "--srv-green", rates().commit),
    rate(sc.rollback, "--danger", rates().rollback),
  ]);
  const tupIn = createMemo(() => [
    rate(sc.insert, "--srv-green", rates().ins),
    rate(sc.update, "--srv-amber", rates().upd),
    rate(sc.delete, "--danger", rates().del),
  ]);
  const tupOut = createMemo(() => [
    rate(sc.returned, "--accent", rates().ret),
    rate(sc.fetched, "--srv-purple", rates().fetched),
  ]);
  const blocks = createMemo(() => [
    rate(sc.read, "--srv-orange", rates().read),
    rate(sc.hit, "--srv-teal", rates().hit),
  ]);

  const high = () => {
    const v = o();
    return !!v && v.maxConnections > 0 && v.connections / v.maxConnections > HIGH_USAGE;
  };
  const total = () => {
    const s = last();
    return s ? s.sessionsActive + s.sessionsIdle + s.sessionsIdleInTx : null;
  };

  return (
    <div class="flex flex-col gap-3 p-3">
      <Show when={props.model.sectionError.overview}>
        {(e) => (
          <p role="alert" class="rounded border border-border px-3 py-2 text-[13px] text-danger">
            {e()}
          </p>
        )}
      </Show>
      <div class="grid grid-cols-[repeat(auto-fit,minmax(170px,1fr))] gap-2">
        <Card label={td.overview.version} title={o()?.version}>
          {o()?.version.split(" on ")[0] ?? "—"}
        </Card>
        <Card label={td.overview.uptime}>{formatDuration(o()?.uptimeSecs ?? null)}</Card>
        <Card
          label={td.overview.connections}
          badge={
            <Show when={high()}>
              <span class="rounded-full border border-danger px-1.5 text-[11px] font-normal text-danger">{td.overview.highUsage}</span>
            </Show>
          }
        >
          {o() ? td.overview.connectionsOf(o()!.connections, o()!.maxConnections) : "—"}
        </Card>
        <Card label={td.overview.oldestXact}>
          {o() ? (o()!.oldestXactSecs == null ? td.overview.none : formatDuration(o()!.oldestXactSecs)) : "—"}
        </Card>
        <Card
          label={td.overview.sessions}
          title={last() && last()!.sessionsOther > 0 ? td.overview.otherTitle(last()!.sessionsOther) : undefined}
          note={
            <Show when={last() && last()!.sessionsOther > 0}>
              <div class="mt-0.5 text-[11px] text-muted">{td.partial}</div>
            </Show>
          }
        >
          {total() ?? "—"}
          <Show when={last() && last()!.sessionsOther > 0}>
            <span class="ml-1 text-[12px] font-normal text-muted">
              (+{last()!.sessionsOther} {td.overview.other.toLowerCase()})
            </span>
          </Show>
        </Card>
      </div>
      <div class="grid grid-cols-[repeat(auto-fit,minmax(320px,1fr))] gap-3">
        <For each={[
          { title: td.overview.charts.sessions, series: sessions, integer: true },
          { title: td.overview.charts.xacts, series: xacts },
          { title: td.overview.charts.tupIn, series: tupIn },
          { title: td.overview.charts.tupOut, series: tupOut },
          { title: td.overview.charts.blocks, series: blocks },
        ]}>
          {(c) => <Chart title={c.title} series={c.series()} integer={c.integer} />}
        </For>
      </div>
    </div>
  );
}
