// Sağ paneldeki dashboard sekme başlıkları ve panelleri (tembel parça; denetleyiciyle birlikte iner). Başlık kabuğu
// (`Head`: sunucu işaretleri, etkinleştirme, kapatma) sorgu sekmeleriyle aynı olsun diye QueryTabs'tan gelir.
import { Activity } from "lucide-solid";
import { For, Show, Suspense, createEffect, lazy, type Component, type JSX } from "solid-js";
import { td } from "../../app/strings-dashboard";
import type { DashTab } from "../query/tabs-store";
import type { DashController } from "./dash-controller";

const DashboardView = lazy(() => import("./DashboardView"));

export type TabHeadComponent = Component<{ id: string; title: string; serverId: string | null; children: JSX.Element }>;

export function DashTabHeads(props: { dash: DashController; tabs: DashTab[]; Head: TabHeadComponent }) {
  return (
    <For each={props.tabs}>
      {(tab) => {
        const status = () => props.dash.model(tab.id)?.status;
        const down = () => (status() === "gone" ? td.status.gone : status() === "lost" ? td.status.lost : null);
        return (
          <props.Head id={tab.id} title={tab.title} serverId={tab.serverId}>
            <Activity size={12} class="shrink-0" aria-hidden="true" />
            <span class="truncate" classList={{ "opacity-60": !!down(), "line-through": status() === "gone" }}>
              {tab.title}
            </span>
            <Show when={down()}>
              {(text) => (
                <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-danger" title={text()}>
                  <span class="sr-only">{text()}</span>
                </span>
              )}
            </Show>
          </props.Head>
        );
      }}
    </For>
  );
}

/** Her dashboard paneli bağlı kalır (grafik geçmişi, kaydırma); gizlisi `invisible`, yenileme `setActive` ile yalnızca etkinde. */
export function DashPanels(props: { dash: DashController; tabs: DashTab[]; activeId: string; onReopen: (tabId: string) => void }) {
  // Yalnızca etkin dashboard yenilenir; arka plandakiler (ve "Nesne"/sorgu sekmesi etkinken hepsi) istek göndermez.
  createEffect(() => {
    const a = props.activeId;
    props.dash.setActive(props.tabs.some((x) => x.id === a) ? a : null);
  });
  return (
    <For each={props.tabs}>
      {(tab) => (
        <div
          class="absolute inset-0 bg-bg"
          classList={{ "invisible pointer-events-none": props.activeId !== tab.id }}
          aria-hidden={props.activeId !== tab.id}
        >
          <Suspense fallback={<p class="p-3 text-muted">{td.loading}</p>}>
            <DashboardView dash={props.dash} tabId={tab.id} onReopen={props.onReopen} />
          </Suspense>
        </div>
      )}
    </For>
  );
}
