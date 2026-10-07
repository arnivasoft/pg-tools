// Table formunun durumu (tembel parça; yalnızca TableForm içe aktarır): seçili bölüm, odak dağıtıcı (Explorer'dan gelen
// kolon/kısıt/ad odak isteği → bölüm + satır vurgusu) ve kendine-başvuran FK anlık görüntüsünü (SelfSnap) koruyan
// düzenleme sarmalayıcıları. Tüm Table düzenlemeleri buradan geçer; aksi halde anlık görüntü dış değişiklik sanılıp
// yeniden alınırdı.
import { createEffect, createSignal, type Accessor } from "solid-js";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TableDef } from "../../ipc/bindings/TableDef";
import type { DesignerController, DesignerModel } from "./designer-controller";
import { createSnapTracker, type SelfSnap, type WithSnap } from "./table-constraints";
import { asTable, rowOfFocus, type TableSection } from "./table-model";

/** Kolon/kısıt bölümüne odak: `index === null` → satır bulunamadı (şerit gösterilir). `seq` her istekte değişir. */
export interface TableHighlight {
  section: "columns" | "constraints";
  index: number | null;
  seq: number;
}

export interface TableEditor {
  /** Anlık görüntüye dokunmayan düzenleme (alanlar, kolonlar, bölümleme). */
  edit: (fn: (d: TableDef) => TableDef) => void;
  /** Anlık görüntülü düzenleme: tablo adı/şeması, kısıt ekle/sil/geri al, FK hedefi (`*Snap` sarmalayıcıları). */
  editSnap: (fn: (d: TableDef, snap: SelfSnap) => WithSnap) => void;
}

export interface TableFormState {
  section: Accessor<TableSection>;
  setSection: (s: TableSection) => void;
  highlight: Accessor<TableHighlight | null>;
  editor: TableEditor;
}

type StateProps = {
  designer: Pick<DesignerController, "focusRequest" | "consumeFocus" | "edit" | "model">;
  tabId: string;
  model: Pick<DesignerModel, "status" | "draft">;
};

export function createTableFormState(props: StateProps): TableFormState {
  const [section, setSection] = createSignal<TableSection>("general");
  const [highlight, setHighlight] = createSignal<TableHighlight | null>(null);

  // Denetleyicinin güncel taslağı (memo zinciri değil: düzenlemeden hemen sonra okunur).
  const current = () => asTable(props.designer.model(props.tabId)?.draft ?? null);
  const tracker = createSnapTracker(current);

  const editSnap = (fn: (d: TableDef, snap: SelfSnap) => WithSnap) => {
    let next: SelfSnap | null = null;
    props.designer.edit(props.tabId, (d: ObjectDef) => {
      if (d.kind !== "table") return d;
      const r = fn(d, tracker.snap());
      next = r.snap;
      return { ...r.d, kind: "table" };
    });
    // Düzenleme reddedildiyse (salt okunur) geri çağrı çalışmaz: anlık görüntü değişmez.
    if (next !== null) tracker.commit(next, current());
  };
  const edit = (fn: (d: TableDef) => TableDef) => editSnap((d, snap) => ({ d: fn(d), snap }));

  // Odak dağıtıcı (tek yer): ad → Genel (alanı TableGeneral'daki createFocusOnRequest odaklar ve tüketir; salt okunurda
  // bekler); kolon/kısıt → bölüm + satır vurgusu, istek hemen tüketilir (salt okunurda da: bölüm/kalıtımlı tablo).
  createEffect(() => {
    const req = props.designer.focusRequest(props.tabId);
    const d = asTable(props.model.draft);
    if (!req || !d || props.model.status !== "ready") return;
    if (req.field === "name") {
      setSection("general");
      return;
    }
    const row = rowOfFocus(d, req.field);
    if (!row) return;
    setSection(row.section);
    setHighlight({ ...row, seq: req.seq });
    props.designer.consumeFocus(props.tabId, req.seq);
  });

  return { section, setSection, highlight, editor: { edit, editSnap } };
}
