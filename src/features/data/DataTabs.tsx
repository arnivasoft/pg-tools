// Sağ paneldeki veri sekme başlıkları ve panelleri (tembel parça; denetleyiciyle birlikte iner). Başlık kabuğu (`Head`)
// QueryTabs'tan gelir. TASLAK (Plan 17 Task 1): ızgara, araç çubuğu, değişiklik paneli ve kapatma onayı sonraki görevlerde.
import { For, type Component, type JSX } from "solid-js";
import { t } from "../../app/strings";
import type { DataTab } from "../query/tabs-store";
import type { DataController } from "./data-open";

// Denetleyici ve açıcı bu parçayla iner (data-open.ts bağımlılıksız kalır; ana paket bütçesi, Plan 17 R1).
export { startData } from "./data-opener";

export type TabHeadComponent = Component<{ id: string; title: string; serverId: string | null; children: JSX.Element }>;

export function DataTabHeads(props: { data: DataController; tabs: DataTab[]; Head: TabHeadComponent }) {
  return (
    <For each={props.tabs}>
      {(tab) => (
        <props.Head id={tab.id} title={tab.title} serverId={tab.serverId}>
          <span class="truncate">{tab.title}</span>
        </props.Head>
      )}
    </For>
  );
}

/** Her veri paneli bağlı kalır (kaydırma, düzenleme); gizlisi `invisible` (display:none kaydırmayı sıfırlar). */
export function DataPanels(props: {
  data: DataController;
  tabs: DataTab[];
  activeId: string;
  onClose: (tabId: string) => void;
  /** Kapatma onayı bekleyen (kirli) veri sekmesi; `onCloseAnswer(true)` = değişiklikleri at ve kapat. */
  closing: string | null;
  onCloseAnswer: (discard: boolean) => void;
}) {
  return (
    <For each={props.tabs}>
      {(tab) => (
        <div
          class="absolute inset-0 bg-bg"
          classList={{ "invisible pointer-events-none": props.activeId !== tab.id }}
          aria-hidden={props.activeId !== tab.id}
        >
          <p class="p-3 text-muted">{t.query.tabs.loading}</p>
        </div>
      )}
    </For>
  );
}
