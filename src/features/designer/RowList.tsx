// Genel satır listesi kabı (Plan 19; tembel parça): argümanlar, TABLE kolonları, enum değerleri, composite öznitelikleri,
// domain kısıtları, trigger argümanları. Satır alanlarını çağıran çizer (`children`); bu bileşen yalnızca ekle / sil /
// yukarı / aşağı düğmelerini ve odak davranışını taşır. İşlemler saf `row-list-model`'dendir (yeni dizi → `onChange`).
//
// Klavye/erişilebilirlik: tüm düğmeler Tab ile erişilir; taşıma sonrası odak taşınan satırın aynı düğmesinde kalır (o
// düğme artık kapalıysa satırın diğer taşıma düğmesine, o da yoksa satıra); silmede sonraki satırın Sil düğmesine (yoksa
// önceki, liste boşsa Ekle'ye); eklemede yeni satırın ilk alanına. `aria-label`'lar satır adını içerir.
// Simge yalnızca `Trash2` (lucide); taşıma düğmeleri metindir.
import { Trash2 } from "lucide-solid";
import { Index, Show, createUniqueId, type Accessor, type JSX } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { Button } from "../../ui/Button";
import { canMove, insertRow, moveRow, removeRow, type MoveDir } from "./row-list-model";

export interface RowListProps<T> {
  /** Grup başlığı (görünür; `aria-labelledby`). */
  label: string;
  items: T[];
  onChange: (list: T[]) => void;
  /** Satırın görünen adı (boş olabilir): düğme etiketleri `tdes.rows.rowName(i, ad)`. */
  name: (item: T, i: number) => string;
  /** Ekle düğmesinin ürettiği satır; verilmezse Ekle düğmesi yok. */
  newItem?: () => T;
  /** Yeni satırın konumu (varsayılan sona). */
  addIndex?: (list: T[]) => number;
  addLabel?: string;
  /** Satırlar yeniden sıralanabilir mi (varsayılan evet). */
  reorderable?: boolean;
  /** Taşınamayan satır (ör. yüklü enum değeri); kilitli satırla yer de değiştirilemez. */
  moveLocked?: (i: number) => boolean;
  /** Ek taşıma kuralı (satır çifti; ör. enum: iki yüklü değer yer değiştiremez ama yeni değer yüklülerin arasına girer). */
  moveAllowed?: (i: number, dir: MoveDir) => boolean;
  /** Silinemeyen satır. */
  removeLocked?: (i: number) => boolean;
  readOnly: boolean;
  emptyText?: string;
  /** Satır alanları; `i` satırın güncel indeksi. */
  children: (item: Accessor<T>, i: number) => JSX.Element;
}

const FOCUSABLE = "input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [contenteditable='true']";

export function RowList<T>(props: RowListProps<T>) {
  const headingId = createUniqueId();
  let root: HTMLDivElement | undefined;
  let addEl: HTMLButtonElement | undefined;
  const editable = () => !props.readOnly;
  const rows = () => (root ? [...root.querySelectorAll<HTMLElement>(":scope > ul > li[data-row]")] : []);
  const later = (f: () => void) => queueMicrotask(f);
  // Silme sırasında kapanan satır bir kez daha okunabilir: satır yoksa yalnızca sıra numarası.
  const rowName = (i: number) => {
    const it = props.items[i];
    return tdes.rows.rowName(i, it === undefined ? "" : props.name(it, i));
  };
  const movable = (i: number, dir: MoveDir) =>
    editable() &&
    (props.reorderable ?? true) &&
    canMove(i, dir, props.items.length, props.moveLocked) &&
    (props.moveAllowed?.(i, dir) ?? true);
  const removable = (i: number) => editable() && !(props.removeLocked?.(i) ?? false);

  const move = (i: number, dir: MoveDir) => {
    if (!movable(i, dir)) return;
    const j = i + dir;
    props.onChange(moveRow(props.items, i, dir, props.moveLocked));
    const act = dir < 0 ? "up" : "down";
    later(() => {
      const row = rows()[j];
      const btn =
        row?.querySelector<HTMLButtonElement>(`[data-act="${act}"]:not(:disabled)`) ??
        row?.querySelector<HTMLButtonElement>(`[data-act="${act === "up" ? "down" : "up"}"]:not(:disabled)`);
      (btn ?? row)?.focus();
    });
  };
  const remove = (i: number) => {
    if (!removable(i)) return;
    props.onChange(removeRow(props.items, i));
    later(() => {
      const list = rows();
      const row = list[Math.min(i, list.length - 1)];
      const btn = row?.querySelector<HTMLButtonElement>(`[data-act="remove"]:not(:disabled)`);
      (btn ?? row ?? addEl)?.focus();
    });
  };
  const add = () => {
    if (!editable() || !props.newItem) return;
    const at = Math.min(Math.max(props.addIndex?.(props.items) ?? props.items.length, 0), props.items.length);
    props.onChange(insertRow(props.items, at, props.newItem()));
    later(() => {
      const row = rows()[at];
      (row?.querySelector<HTMLElement>(FOCUSABLE) ?? row)?.focus();
    });
  };

  return (
    <div ref={root} role="group" aria-labelledby={headingId} class="flex flex-col gap-2">
      <h3 id={headingId} class="text-[13px] font-medium">
        {props.label}
      </h3>
      <Show when={props.items.length === 0}>
        <p class="text-xs text-muted">{props.emptyText ?? tdes.rows.empty}</p>
      </Show>
      <ul role="list" class="flex flex-col gap-2">
        <Index each={props.items}>
          {(item, i) => (
            <li
              data-row
              tabIndex={-1}
              aria-label={rowName(i)}
              class="flex flex-col gap-1 rounded border border-border px-2 py-2 outline-none focus:border-accent"
            >
              <div class="flex items-start gap-2">
                <div class="min-w-0 flex-1">{props.children(item, i)}</div>
                <Show when={editable()}>
                  <div class="flex shrink-0 items-center gap-1">
                    <Show when={props.reorderable ?? true}>
                      <Button
                        variant="ghost"
                        data-act="up"
                        class="!px-1.5 text-[12px]"
                        aria-label={tdes.rows.moveUp(rowName(i))}
                        title={tdes.rows.moveUp(rowName(i))}
                        disabled={!movable(i, -1)}
                        onClick={() => move(i, -1)}
                      >
                        {tdes.rows.up}
                      </Button>
                      <Button
                        variant="ghost"
                        data-act="down"
                        class="!px-1.5 text-[12px]"
                        aria-label={tdes.rows.moveDown(rowName(i))}
                        title={tdes.rows.moveDown(rowName(i))}
                        disabled={!movable(i, 1)}
                        onClick={() => move(i, 1)}
                      >
                        {tdes.rows.down}
                      </Button>
                    </Show>
                    <Button
                      variant="ghost"
                      data-act="remove"
                      aria-label={tdes.rows.remove(rowName(i))}
                      title={tdes.rows.remove(rowName(i))}
                      disabled={!removable(i)}
                      onClick={() => remove(i)}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                    </Button>
                  </div>
                </Show>
              </div>
            </li>
          )}
        </Index>
      </ul>
      <Show when={props.newItem && editable()}>
        <div>
          <Button ref={(el: HTMLButtonElement) => (addEl = el)} onClick={add}>
            {props.addLabel ?? tdes.rows.add}
          </Button>
        </div>
      </Show>
    </div>
  );
}
