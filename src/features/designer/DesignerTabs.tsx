// Sağ paneldeki designer sekme başlıkları, panelleri ve kirli sekme kapatma onayı (tembel parça; denetleyiciyle birlikte
// iner). Başlık kabuğu (`Head`: sunucu işaretleri, etkinleştirme, kapatma) sorgu sekmeleriyle aynı olsun diye QueryTabs'tan
// gelir.
import { PencilRuler } from "lucide-solid";
import { For, Show, Suspense, lazy, type Component, type JSX } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { ConfirmDialog } from "../../ui/ConfirmDialog";
import type { DesignerTab } from "../query/tabs-store";
import type { DesignerController } from "./designer-controller";

const DesignerView = lazy(() => import("./DesignerView"));

export type TabHeadComponent = Component<{ id: string; title: string; serverId: string | null; children: JSX.Element }>;

export function DesignerTabHeads(props: { designer: DesignerController; tabs: DesignerTab[]; Head: TabHeadComponent }) {
  return (
    <For each={props.tabs}>
      {(tab) => {
        const m = () => props.designer.model(tab.id);
        const down = () => m()?.status === "disconnected" || m()?.status === "gone";
        return (
          <props.Head id={tab.id} title={tab.title} serverId={tab.serverId}>
            <PencilRuler size={12} class="shrink-0" aria-hidden="true" />
            <span class="truncate" classList={{ "opacity-60": down(), "line-through": m()?.status === "gone" }}>
              {tab.title}
            </span>
            <Show when={m()?.dirty}>
              <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" title={tdes.tab.dirty}>
                <span class="sr-only">{tdes.tab.dirty}</span>
              </span>
            </Show>
          </props.Head>
        );
      }}
    </For>
  );
}

/** Her designer paneli bağlı kalır (form durumu, kaydırma, editör); gizlisi `invisible`. Kapatma onayı da buradadır. */
export function DesignerPanels(props: {
  designer: DesignerController;
  tabs: DesignerTab[];
  activeId: string;
  onOpenScript: (input: { serverId: string; databaseOid: number; text: string }) => void;
  onClose: (tabId: string) => void;
  /** Kapatma onayı bekleyen (kirli) sekme; `onCloseAnswer(true)` = değişiklikleri at ve kapat. */
  closing: string | null;
  onCloseAnswer: (discard: boolean) => void;
}) {
  const closingTitle = () => props.tabs.find((x) => x.id === props.closing)?.title ?? null;
  return (
    <>
      <For each={props.tabs}>
        {(tab) => (
          <div
            class="absolute inset-0 bg-bg"
            classList={{ "invisible pointer-events-none": props.activeId !== tab.id }}
            aria-hidden={props.activeId !== tab.id}
          >
            <Suspense fallback={<p class="p-3 text-muted">{tdes.loading}</p>}>
              <DesignerView
                designer={props.designer}
                tabId={tab.id}
                where={tab.where}
                onOpenScript={props.onOpenScript}
                onClose={props.onClose}
              />
            </Suspense>
          </div>
        )}
      </For>
      {/* Kirli sekmeyi kapatma onayı; odak Vazgeç'te (Enter atmayı onaylamaz). */}
      <ConfirmDialog
        open={props.closing !== null}
        title={tdes.discard.title}
        message={tdes.discard.closeMessage(closingTitle() ?? "")}
        confirmLabel={tdes.discard.close}
        danger
        onCancel={() => props.onCloseAnswer(false)}
        onConfirm={() => props.onCloseAnswer(true)}
      />
    </>
  );
}
