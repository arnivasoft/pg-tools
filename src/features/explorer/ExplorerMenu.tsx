import { ContextMenu } from "@kobalte/core/context-menu";
import { For, Show, createEffect, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { actionsFor, executeAction, groupOf, type ActionDeps, type ActionId } from "./actions";
import type { Explorer } from "./explorer-store";
import { TreeView } from "./TreeView";

const itemClass =
  "flex h-7 cursor-default items-center px-3 text-[13px] outline-none data-[highlighted]:bg-hover";

/** Satırın ham kimliğini `data-node-id`'den okur (TreeRow `encodeURIComponent` ile yazar). */
export function rowIdFromTarget(target: EventTarget | null): string | null {
  const row = target instanceof Element ? target.closest('[role="treeitem"]') : null;
  const raw = row?.getAttribute("data-node-id");
  return raw === null || raw === undefined ? null : decodeURIComponent(raw);
}

export function ExplorerMenu(props: { explorer: Explorer; deps: ActionDeps; treeRef?: (el: HTMLElement) => void }) {
  const ex = props.explorer;
  const [open, setOpen] = createSignal(false);
  const [target, setTarget] = createSignal<string | null>(null);
  let treeEl: HTMLElement | undefined;

  const targetNode = () => {
    const id = target();
    return id === null ? null : (ex.state().nodes.get(id) ?? null);
  };
  const items = () => {
    const node = targetNode();
    return node ? actionsFor(ex.state(), node) : [];
  };

  // Menü açıkken hedef ağaçtan kalkarsa (yenileme/silme) menüyü kapat.
  createEffect(() => {
    if (open() && targetNode() === null) setOpen(false);
  });

  const label = (a: ActionId) => t.menu[a];

  return (
    // `open` ContextMenuRootProps tiplerinde yok, ama Kobalte 0.13 kökü `{...others}`ı `open={...}`tan SONRA yayar,
    // yani verilen `open` kazanır (context-menu-root.tsx). Hedef kaybolunca menüyü kapatmak için gerekli.
    <ContextMenu onOpenChange={setOpen} {...({ open: open() } as object)}>
      <ContextMenu.Trigger
        class="h-full"
        // Kobalte kendi işleyicisinden ÖNCE bunu çağırır (callHandler); defaultPrevented ise menüyü açmaz.
        onContextMenu={(e: MouseEvent) => {
          // Satır yoksa (boş alan, ContextMenu / Shift+F10 tuşu) seçili düğüme düş.
          const id = rowIdFromTarget(e.target) ?? ex.state().selected;
          if (id === null || !ex.state().nodes.has(id)) {
            e.preventDefault();
            return;
          }
          ex.controller.select(id);
          setTarget(id);
        }}
      >
        <TreeView
          explorer={ex}
          treeRef={(el) => {
            treeEl = el;
            props.treeRef?.(el);
          }}
          onCopyName={(id) => void executeAction(props.deps, "copyName", id)}
        />
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content
          class="z-50 min-w-48 rounded-md border border-border bg-bg py-1 shadow-xl outline-none"
          onCloseAutoFocus={(e: Event) => {
            e.preventDefault();
            treeEl?.focus();
          }}
        >
          <For each={items()}>
            {(action, i) => (
              <>
                <Show when={i() > 0 && groupOf(items()[i() - 1]) !== groupOf(action)}>
                  <ContextMenu.Separator class="my-1 border-t border-border" />
                </Show>
                <ContextMenu.Item
                  class={itemClass}
                  onSelect={() => {
                    const id = target();
                    if (id !== null) void executeAction(props.deps, action, id);
                  }}
                >
                  {label(action)}
                </ContextMenu.Item>
              </>
            )}
          </For>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu>
  );
}
