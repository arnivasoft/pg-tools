// Sağ paneldeki şema karşılaştırma sekme başlıkları ve panelleri (tembel parça; denetleyiciyle birlikte iner). Başlık
// kabuğu (`Head`) QueryTabs'tan gelir. Paket bütçesi: yalnızca solid-js, strings-diff ve bu dizindeki özel modüller içe
// aktarılır (`src/ui/*`, lucide, CodeMirror yok; paylaşılan parça doğmasın).
import { For, Show, type Component, type JSX } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { DiffTab } from "../query/tabs-store";
import type { DiffController } from "./diff-open";

// Denetleyici ve açıcı bu parçayla iner (diff-open.ts bağımlılıksız kalır).
export { startDiff } from "./diff-opener";

export type TabHeadComponent = Component<{ id: string; title: string; serverId: string | null; children: JSX.Element }>;

export function DiffTabHeads(props: { diff: DiffController; tabs: DiffTab[]; Head: TabHeadComponent }) {
  return (
    <For each={props.tabs}>
      {(tab) => (
        <props.Head id={tab.id} title={tab.title} serverId={null}>
          <span class="truncate">{tab.title}</span>
        </props.Head>
      )}
    </For>
  );
}

export function DiffPanels(props: { diff: DiffController; tabs: DiffTab[]; activeId: string }) {
  return (
    <For each={props.tabs}>
      {(tab) => (
        <div class="absolute inset-0 bg-bg" classList={{ "invisible pointer-events-none": props.activeId !== tab.id }} aria-hidden={props.activeId !== tab.id}>
          <Show when={props.diff.model(tab.id) === null}>
            <p class="p-3 text-muted">{tsd.preparing}</p>
          </Show>
        </div>
      )}
    </For>
  );
}
