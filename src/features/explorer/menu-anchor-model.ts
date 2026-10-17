// Explorer sağ tık menüsünün saf konum/klavye modeli (ana paket): tembel menü bu çapaya sanal olarak bağlanır.

export type MenuAnchor = { x: number; y: number; nodeId: string; source: "pointer" | "keyboard" };

/** Çapa hesabı için gereken dikdörtgen alanları (`DOMRect` uyar). */
type RectLike = Pick<DOMRect, "left" | "top" | "bottom">;

/**
 * Olaydan çapa: fare → istemci koordinatı; klavye (ContextMenu tuşu / Shift+F10) → satır öğesinin sol-alt köşesi.
 * Hedef düğüm tıklanan satırdır, satır yoksa seçili düğüme düşülür; ikisi de yoksa null (menü açılmaz).
 * Klavyede satır DOM'da yoksa (sanal kaydırma dışında) ağaç kapsayıcısının sol üstü kullanılır; o da yoksa null.
 */
export function anchorFromEvent(
  e: { clientX: number; clientY: number; button?: number; detail?: number },
  rowId: string | null,
  selected: string | null,
  rowRect: (id: string) => RectLike | null,
  keyboard: boolean,
  containerRect?: () => RectLike | null,
): MenuAnchor | null {
  const nodeId = rowId ?? selected;
  if (nodeId === null) return null;
  if (!keyboard) return { x: e.clientX, y: e.clientY, nodeId, source: "pointer" };
  const row = rowRect(nodeId);
  if (row) return { x: row.left, y: row.bottom, nodeId, source: "keyboard" };
  const box = containerRect?.() ?? null;
  return box ? { x: box.left, y: box.top, nodeId, source: "keyboard" } : null;
}

/**
 * Tarayıcının klavyeden ürettiği `contextmenu` olayı mı? Kobalte ContextMenu ayırt etmez (her zaman clientX/Y kullanır);
 * burada Pointer Events kuralı esas: işaretçi dışı üretilen olayda `pointerType === ""` / `pointerId === -1`.
 * İşaretçi bilgisi yoksa (eski MouseEvent) sağ düğme dışı her `contextmenu` klavye sayılır.
 */
export function isKeyboardContextMenu(e: { button?: number; pointerType?: string; pointerId?: number }): boolean {
  if (e.pointerType === "" || e.pointerId === -1) return true;
  if (e.pointerType !== undefined) return false;
  return e.button !== 2;
}

/** Menü tuşu: ContextMenu ya da (yalnız) Shift+F10. IME bileşimi sırasında yok sayılır. */
export function isMenuKey(e: {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
  keyCode: number;
}): boolean {
  if (e.isComposing || e.keyCode === 229) return false;
  if (e.key === "ContextMenu") return true;
  return e.key === "F10" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey;
}

/** Aynı düğüm ve konum: keydown'dan sonra gelen klavye `contextmenu` olayı menüyü yeniden açmasın. */
export function sameAnchor(a: MenuAnchor | null, b: MenuAnchor | null): boolean {
  return a !== null && b !== null && a.nodeId === b.nodeId && a.x === b.x && a.y === b.y;
}

/**
 * Menü açıcı: çapa hemen kaydedilir (parça yüklenirken olay kaybolmaz; yükleme bitince menü son çapada açılır).
 * `ensure` paylaşılan, reddi önbelleğe almayan yükleyicidir (bildirimi o verir); ret gelince çapa hâlâ aynıysa temizlenir.
 * Aynı çapa yeniden gelirse (keydown ardından tarayıcının klavye `contextmenu` olayı) hiçbir şey yapılmaz.
 */
export function createMenuOpener(
  ensure: () => Promise<boolean>,
  get: () => MenuAnchor | null,
  set: (a: MenuAnchor | null) => void,
): (next: MenuAnchor) => void {
  return (next) => {
    if (sameAnchor(get(), next)) return;
    set(next);
    void ensure().then((ok) => {
      if (!ok && get() === next) set(null);
    });
  };
}

/** Öğe grubu değişince önüne ayırıcı gelir (ilk öğe hariç). */
export function separatorBefore<A>(list: readonly A[], i: number, groupOf: (a: A) => number): boolean {
  return i > 0 && groupOf(list[i - 1]) !== groupOf(list[i]);
}
