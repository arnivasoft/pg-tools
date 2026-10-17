// Explorer sağ tık menüsünün içeriği (tembel parça): Kobalte DropdownMenu, ana paketteki çapaya sanal olarak bağlanır.
// Ana paket (ExplorerMenu) ağacı saran div'i, olayları ve çapayı tutar; metinler, gerekçeler ve eylemler prop olarak gelir.
// Bu parça ana paket modüllerini (strings, actions …) içe aktarmaz: aktarırsa Rolldown onları ana paketten ayrı, açılışta
// önyüklenen parçalara böler (Task 4–5 ölçümü).
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show } from "solid-js";
import type { ActionId } from "./actions";
import type { MenuAnchor } from "./menu-anchor-model";

const itemClass =
  "flex h-7 cursor-default items-center px-3 text-[13px] outline-none data-[highlighted]:bg-hover data-[disabled]:opacity-50";

export type ExplorerMenuPopupProps = {
  anchor: MenuAnchor;
  /** Hedef düğüm hâlâ ağaçta mı (değilse içerik çizilmez). */
  visible: boolean;
  actions: ActionId[];
  label: (a: ActionId) => string;
  /** Kapalı öğenin gerekçesi (`title`); null ise etkin. Tepkisel okunur. */
  reason: (a: ActionId) => string | null;
  /** Öğenin sağındaki kısayol metni. */
  shortcut: (a: ActionId) => string | null;
  separatorBefore: (i: number) => boolean;
  onSelect: (a: ActionId) => void;
  /** Kobalte menüyü kapattı (Esc, dışarı tık, öğe seçimi). */
  onClose: () => void;
  /** Kapanış odağı: Kobalte'nin varsayılanı engellenir, odağın nereye döneceğine ana paket karar verir. */
  onCloseAutoFocus: () => void;
};

export default function ExplorerMenuPopup(props: ExplorerMenuPopupProps) {
  return (
    // ContextMenu kökünün varsayılanları (sağ-başlangıç, gutter/shift 2) korunur; çapa sıfır boyutlu bir noktadır.
    <DropdownMenu
      open
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
      getAnchorRect={() => ({ x: props.anchor.x, y: props.anchor.y, width: 0, height: 0 })}
      placement="right-start"
      gutter={2}
      shift={2}
    >
      <DropdownMenu.Portal>
        {/* Hedef menü açıkken ağaçtan kalkarsa içerik hiçbir şey çizmez; Kobalte durumu zorlanmaz (sonraki sağ tık hedefi yeniler). */}
        <Show when={props.visible}>
        <DropdownMenu.Content
          class="z-50 min-w-48 rounded-md border border-border bg-bg py-1 shadow-xl outline-none"
          // Menü üstünde sağ tık / menü tuşu tarayıcı menüsünü açmasın.
          onContextMenu={(e: MouseEvent) => e.preventDefault()}
          onCloseAutoFocus={(e: Event) => {
            e.preventDefault();
            props.onCloseAutoFocus();
          }}
        >
          <For each={props.actions}>
            {(action, i) => {
              // Kapalı öğe görünür kalır (gerekçe `title`); Kobalte seçtirmez.
              const off = () => props.reason(action);
              return (
              <>
                <Show when={props.separatorBefore(i())}>
                  <DropdownMenu.Separator class="my-1 border-t border-border" />
                </Show>
                <DropdownMenu.Item
                  class={itemClass}
                  disabled={!!off()}
                  title={off() ?? undefined}
                  onSelect={() => props.onSelect(action)}
                >
                  {props.label(action)}
                  <Show when={props.shortcut(action)}>
                    {(s) => <span class="ml-auto pl-6 text-xs text-muted">{s()}</span>}
                  </Show>
                </DropdownMenu.Item>
              </>
              );
            }}
          </For>
        </DropdownMenu.Content>
        </Show>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}
