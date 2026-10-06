// Dashboard sekmesinin gövdesi (tembel parça): üst çubuk (aralık, Duraklat/Sürdür, Yenile, durum), kayıp/silinmiş
// şeridi ve bölüm sekmeleri. Bölüm gövdeleri kendi dosyalarındadır ve ayrı tembel parçalar olarak yüklenir.
import { Pause, Play, RefreshCw, RotateCcw } from "lucide-solid";
import { For, Show, Suspense, lazy, type Component } from "solid-js";
import { td } from "../../app/strings-dashboard";
import { Button } from "../../ui/Button";
import { TabPanel, Tabs } from "../../ui/Tabs";
import SignalDialog from "./SignalDialog";
import type { DashController, DashModel, Section } from "./dash-controller";

export interface SectionProps {
  dash: DashController;
  model: DashModel;
}

/** Görünen bölümler, sekme sırasıyla. Hepsi tembel parça. */
const SECTIONS: { value: Section; view: Component<SectionProps> }[] = [
  { value: "overview", view: lazy(() => import("./OverviewView")) },
  { value: "sessions", view: lazy(() => import("./SessionsView")) },
  { value: "locks", view: lazy(() => import("./LocksView")) },
  { value: "settings", view: lazy(() => import("./SettingsView")) },
  { value: "statements", view: lazy(() => import("./StatementsView")) },
];

/** Yenileme aralığı seçenekleri (sn). */
const INTERVALS = [1, 2, 5, 10, 30];

const selectClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:opacity-50";

function StatusBadge(props: { model: DashModel }) {
  const s = td.status;
  const text = () => {
    const m = props.model;
    return m.status === "ready" && m.paused ? s.paused : s[m.status];
  };
  const danger = () => props.model.status === "lost" || props.model.status === "gone";
  const live = () => props.model.status === "ready" && !props.model.paused;
  return (
    <span
      role="status"
      class="inline-flex h-6 items-center gap-1.5 rounded-full border border-border px-2 text-[12px]"
      classList={{ "text-danger": danger(), "text-muted": !danger() && !live(), "text-fg": live() }}
    >
      <span
        aria-hidden="true"
        class="h-1.5 w-1.5 rounded-full"
        classList={{ "bg-accent": live(), "bg-danger": danger(), "bg-muted": !danger() && !live() }}
      />
      {text()}
    </span>
  );
}

export default function DashboardView(props: { dash: DashController; tabId: string; onReopen: (tabId: string) => void }) {
  const model = () => props.dash.model(props.tabId);
  let root: HTMLDivElement | undefined;
  return (
    <Show when={model()}>
      {(m) => {
        const ready = () => m().status === "ready";
        const where = () => (m().info?.database ? `${m().serverName}/${m().info?.database}` : m().serverName);
        return (
          <div ref={root} class="flex h-full min-h-0 flex-col">
            <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border bg-panel px-2">
              <span class="min-w-0 truncate text-[13px]" title={where()}>
                {where()}
              </span>
              <StatusBadge model={m()} />
              <div class="ml-auto flex items-center gap-1">
                <select
                  class={selectClass}
                  aria-label={td.bar.interval}
                  title={td.bar.interval}
                  value={String(m().intervalMs / 1000)}
                  onChange={(ev) => props.dash.setIntervalMs(props.tabId, Number(ev.currentTarget.value) * 1000)}
                >
                  <For each={INTERVALS}>{(n) => <option value={String(n)}>{td.bar.seconds(n)}</option>}</For>
                </select>
                <Button variant="ghost" onClick={() => props.dash.setPaused(props.tabId, !m().paused)}>
                  <Show when={m().paused} fallback={<Pause size={14} />}>
                    <Play size={14} />
                  </Show>
                  {m().paused ? td.bar.resume : td.bar.pause}
                </Button>
                <Button variant="ghost" disabled={!ready()} onClick={() => props.dash.refreshNow(props.tabId)}>
                  <RefreshCw size={14} /> {td.bar.refresh}
                </Button>
              </div>
            </div>
            <Show when={m().status === "lost"}>
              <div role="alert" class="m-2 flex shrink-0 items-center gap-3 rounded border border-danger px-3 py-2 text-[13px]">
                <div class="min-w-0 flex-1">
                  <p class="font-medium text-danger">{td.status.lost}</p>
                  <p class="text-muted">{m().error ?? td.lostNote}</p>
                </div>
                <Button onClick={() => props.onReopen(props.tabId)}>
                  <RotateCcw size={14} /> {td.reopen}
                </Button>
              </div>
            </Show>
            <Show when={m().status === "gone"}>
              <div role="alert" class="m-2 shrink-0 rounded border border-border px-3 py-2 text-[13px]">
                <p class="font-medium text-danger">{td.status.gone}</p>
                <p class="text-muted">{td.goneNote}</p>
              </div>
            </Show>
            <Tabs
              value={m().section}
              onChange={(v) => {
                const s = SECTIONS.find((x) => x.value === v);
                if (s) props.dash.setSection(props.tabId, s.value);
              }}
              class="min-h-0 flex-1"
              tabs={SECTIONS.map((s) => ({
                value: s.value,
                label: td.sections[s.value],
                // Eklenti yok/yüklü değil: sekme soluk ama tıklanabilir (neden metni görünsün).
                muted: s.value === "statements" && m().statements?.kind === "unavailable",
              }))}
            >
              <For each={SECTIONS}>
                {(s) => (
                  <TabPanel value={s.value} class="min-h-0 flex-1 overflow-auto">
                    <Suspense fallback={<p class="p-3 text-muted">{td.loading}</p>}>
                      <s.view dash={props.dash} model={m()} />
                    </Suspense>
                  </TabPanel>
                )}
              </For>
            </Tabs>
            <SignalDialog
              dash={props.dash}
              tabId={props.tabId}
              restoreFocus={() => root?.querySelector<HTMLElement>("[role=grid]")?.focus()}
            />
          </div>
        );
      }}
    </Show>
  );
}
