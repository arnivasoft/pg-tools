// Explorer sağ tık menüsü (ana paket): ağacı saran div + olaylar + çapa. Menü içeriği (Kobalte) tembel parçadadır
// (ExplorerMenuPopup) ve çapaya sanal olarak bağlanır; ağaç asla yeniden bağlanmaz.
import { type Component, Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { toast } from "../../app/toast";
import { createLoader } from "../../ui/lazy-load";
import { actionsFor, disabledReason, executeAction, groupOf, type ActionDeps } from "./actions";
import type { ExplorerMenuPopupProps } from "./ExplorerMenuPopup";
import type { Explorer } from "./explorer-store";
import { anchorFromEvent, createMenuOpener, isKeyboardContextMenu, isMenuKey, separatorBefore, type MenuAnchor } from "./menu-anchor-model";
import { rowDomId } from "./TreeRow";
import { TreeView } from "./TreeView";

/** Satırın ham kimliğini `data-node-id`'den okur (TreeRow `encodeURIComponent` ile yazar). */
export function rowIdFromTarget(target: EventTarget | null): string | null {
  const row = target instanceof Element ? target.closest('[role="treeitem"]') : null;
  const raw = row?.getAttribute("data-node-id");
  return raw === null || raw === undefined ? null : decodeURIComponent(raw);
}

/** Yükleyici (tek tanım): menü açılışı ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadExplorerMenuPopup = () => import("./ExplorerMenuPopup");

// Parça modül düzeyinde bir kez yüklenir (ilk sağ tıkta). Ret önbelleğe alınmaz: bildirim ret başına bir kez, sonraki
// sağ tık yeniden dener.
const [Popup, setPopup] = createSignal<Component<ExplorerMenuPopupProps> | null>(null);
const ensurePopup = createLoader(
  () => loadExplorerMenuPopup().then((m) => void setPopup(() => m.default)),
  () => toast.show(t.app.chunkLoadFailed, "error"),
);

export function ExplorerMenu(props: { explorer: Explorer; deps: ActionDeps; treeRef?: (el: HTMLElement) => void }) {
  const ex = props.explorer;
  const [anchor, setAnchor] = createSignal<MenuAnchor | null>(null);
  let treeEl: HTMLElement | undefined;

  const open = createMenuOpener(ensurePopup, anchor, setAnchor);
  const rowRect = (id: string) => document.getElementById(rowDomId(id))?.getBoundingClientRect() ?? null;

  /** Satır yoksa (boş alan, ContextMenu / Shift+F10 tuşu) seçili düğüme düşer; düğüm yoksa menü açılmaz. */
  const request = (e: { clientX: number; clientY: number }, target: EventTarget | null, keyboard: boolean) => {
    const a = anchorFromEvent(e, rowIdFromTarget(target), ex.state().selected, rowRect, keyboard, () => treeEl?.getBoundingClientRect() ?? null);
    if (a === null || !ex.state().nodes.has(a.nodeId)) return;
    ex.controller.select(a.nodeId);
    open(a);
  };

  // Portal'daki menüden kabaran (Solid olay devri) olaylar ağaç olayı sayılmaz.
  const fromTree = (e: Event) => e.currentTarget instanceof Node && e.target instanceof Node && e.currentTarget.contains(e.target);

  return (
    <div
      class="h-full"
      style={{ "-webkit-touch-callout": "none" }}
      // Tarayıcı menüsü hiçbir zaman çıkmaz (eski Kobalte Trigger davranışı); menü yalnız çapa varsa açılır.
      onContextMenu={(e) => {
        e.preventDefault();
        if (!fromTree(e)) return;
        request(e, e.target, isKeyboardContextMenu(e as MouseEvent & { pointerType?: string; pointerId?: number }));
      }}
      onKeyDown={(e) => {
        // ContextMenu tuşu / Shift+F10 (IME koruması isMenuKey içinde). Tarayıcı ayrıca klavye `contextmenu` olayı
        // üretirse aynı çapa olduğundan menü yeniden açılmaz (createMenuOpener).
        if (!isMenuKey(e) || !fromTree(e)) return;
        e.preventDefault();
        request({ clientX: 0, clientY: 0 }, e.target, true);
      }}
    >
      <TreeView
        explorer={ex}
        treeRef={(el) => {
          treeEl = el;
          props.treeRef?.(el);
        }}
        onCopyName={(id) => void executeAction(props.deps, "copyName", id)}
        onOpenQuery={(id) => void executeAction(props.deps, "openQueryTool", id)}
      />
      <Show when={Popup()}>
        {(P) => (
          // Her yeni çapa menüyü yeniden bağlar (Kobalte konumu açıkken izlemez): açıkken ikinci sağ tık = kapan + yeni konumda aç.
          <Show when={anchor()} keyed>
            {(a) => {
              const View = P();
              const node = () => ex.state().nodes.get(a.nodeId) ?? null;
              const items = () => {
                const n = node();
                return n ? actionsFor(ex.state(), n) : [];
              };
              return (
                <View
                  anchor={a}
                  visible={node() !== null}
                  actions={items()}
                  label={(action) => t.menu[action]}
                  reason={(action) => disabledReason(action, props.deps, a.nodeId)}
                  shortcut={(action) => (action === "openQueryTool" ? t.menu.openQueryToolShortcut : null)}
                  separatorBefore={(i) => separatorBefore(items(), i, groupOf)}
                  onSelect={(action) => void executeAction(props.deps, action, a.nodeId)}
                  onClose={() => {
                    if (anchor() === a) setAnchor(null);
                  }}
                  onCloseAutoFocus={() => {
                    // Yerine yeni bir menü açıldıysa odak onda kalır.
                    if (anchor() === null) treeEl?.focus();
                  }}
                />
              );
            }}
          </Show>
        )}
      </Show>
    </div>
  );
}
