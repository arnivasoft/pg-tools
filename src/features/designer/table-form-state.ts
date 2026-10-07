// Table formunun durumu (tembel parça; yalnızca TableForm içe aktarır): seçili bölüm, odak dağıtıcı (Explorer'dan gelen
// kolon/kısıt/ad odak isteği → bölüm + satır vurgusu) ve kendine-başvuran FK anlık görüntüsünü (SelfSnap) koruyan
// düzenleme sarmalayıcıları. Tüm Table düzenlemeleri buradan geçer; aksi halde anlık görüntü dış değişiklik sanılıp
// yeniden alınırdı.
import { batch, createEffect, createSignal, type Accessor } from "solid-js";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TableDef } from "../../ipc/bindings/TableDef";
import type { DesignerController, DesignerModel } from "./designer-controller";
import type { FormProps } from "./FormParts";
import { NO_PK_FORCED, createSnapTracker, reconcilePrimaryKeyNotNull, type PkForced, type SelfSnap, type WithSnap } from "./table-constraints";
import { asTable, columnRefs, removeColumnRestoringName, renameTableColumn, rowOfFocus, type ColumnRef, type TableDraft, type TableSection } from "./table-model";

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
  /** Taslağın dışarıdan (yükleme/yeniden yükleme) kaç kez değiştiği; formun kendi düzenlemeleri artırmaz. */
  epoch: () => number;
  /** Güncel SelfSnap (taslağa bağlı okunur: dış değişimde yeniden alınır). Yalnızca gösterim (ör. sarkan referans). */
  snap: () => SelfSnap;
}

/**
 * Her bölüm gövdesinin aldığı prop'lar (TableForm `PANELS` tablosu). Bölümler hep bağlı kalır (gizli = `hidden`);
 * `active` yalnızca seçili bölümde true — odak/kaydırma etkileri `active` iken çalışmalı (gizli öğe odaklanamaz).
 */
export interface TableSectionProps extends FormProps {
  draft: TableDraft;
  original: TableDraft | null;
  editor: TableEditor;
  highlight: TableHighlight | null;
  active: boolean;
  /** Kısıtlar bölümündeki "Düzenlenmeyenler" listesinin id'si (TableForm'un "kısıt bulunamadı" şeridi buna bağlanır). */
  unsupportedId?: string;
}

/** Geliştirme koruması metni (yalnızca DEV yapısında atılır). */
export const SNAP_GUARD = "Table editor.edit: kısıt sayısı değişti; editSnap ve *Snap sarmalayıcılarını kullanın.";

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
  let epochCount = 0;
  let epochSeen: TableDef | null | undefined;
  // Her düzenlemeden önce ve dağıtıcıda örneklenir (tembel örnekleme dış değişimi kaçırmasın).
  const epoch = () => {
    const d = current();
    if (d !== epochSeen) {
      if (!tracker.isOwn(d)) epochCount++;
      epochSeen = d;
    }
    return epochCount;
  };

  // PRIMARY KEY'in zorladığı NOT NULL kaydı: her düzenlemeden sonra uzlaştırılır (PK'dan çıkan kolon eski değerine
  // döner); taslak dışarıdan değiştiyse (yükleme/yeniden yükleme) sıfırlanır.
  let forced: PkForced = NO_PK_FORCED;
  const editSnap = (fn: (d: TableDef, snap: SelfSnap) => WithSnap) => {
    let next: SelfSnap | null = null;
    let nextForced: PkForced = NO_PK_FORCED;
    epoch();
    if (!tracker.isOwn(current())) forced = NO_PK_FORCED;
    // Toplu: taslak ve anlık görüntü birlikte kaydedilmeden etkiler (odak dağıtıcının "dış değişim" denetimi) çalışmasın.
    batch(() => {
      props.designer.edit(props.tabId, (d: ObjectDef) => {
        if (d.kind !== "table") return d;
        const r = fn(d, tracker.snap());
        const pk = reconcilePrimaryKeyNotNull(d, r.d, forced);
        next = r.snap;
        nextForced = pk.forced;
        return { ...pk.d, kind: "table" };
      });
      // Düzenleme reddedildiyse (salt okunur) geri çağrı çalışmaz: anlık görüntü değişmez.
      if (next !== null) {
        tracker.commit(next, current());
        forced = nextForced;
      }
    });
  };
  const edit = (fn: (d: TableDef) => TableDef) =>
    editSnap((d, snap) => {
      const out = fn(d);
      // Geliştirme koruması (üretimde silinir): kısıt sayısını değiştiren işlem anlık görüntüyü kaydırır; `editSnap` ve
      // `*Snap` sarmalayıcıları kullanılmalı (çıplak add/remove/restoreConstraint anlık görüntüyü bayatlatır).
      if (import.meta.env.DEV && out.constraints.length !== d.constraints.length) throw new Error(SNAP_GUARD);
      return { d: out, snap };
    });

  // Odak dağıtıcı (tek yer):
  // - taslak dışarıdan değiştiyse (yükleme/yeniden yükleme) eski satır vurgusu ve "satır yok" şeridi temizlenir;
  // - ad → Genel, istek başına BİR kez (alanı TableGeneral'daki createFocusOnRequest odaklar ve tüketir; salt okunur/kilitli
  //   sekmede istek bekler ama kullanıcıyı her yeniden çalışmada Genel'e çekmez);
  // - kolon/kısıt → bölüm + satır vurgusu, istek hemen tüketilir (salt okunurda da: bölüm/kalıtımlı tablo).
  let lastDraft: TableDef | null = null;
  let nameSeq: number | null = null;
  createEffect(() => {
    const d = asTable(props.model.draft);
    if (d !== lastDraft) {
      if (lastDraft !== null && !tracker.isOwn(d)) setHighlight(null);
      lastDraft = d;
    }
    epoch();
    const req = props.designer.focusRequest(props.tabId);
    if (!req || !d || props.model.status !== "ready") return;
    if (req.field === "name") {
      if (nameSeq !== req.seq) {
        nameSeq = req.seq;
        setSection("general");
      }
      return;
    }
    const row = rowOfFocus(d, req.field);
    if (!row) return;
    setSection(row.section);
    setHighlight({ ...row, seq: req.seq });
    props.designer.consumeFocus(props.tabId, req.seq);
  });

  return { section, setSection, highlight, editor: { edit, editSnap, epoch, snap: () => tracker.snap() } };
}

/**
 * Kolon yeniden adlandırma başvuru kaydı (form başına bir tane; tüm kolon satırları paylaşır). Her satırın başvuru
 * konumları (kısıt kolon listeleri, kendine-başvuran FK hedef kolonları, bölümleme anahtarları) o satırın İLK ad
 * düzenlemesinde formun SelfSnap'iyle alınır ve odak oturumları boyunca KORUNUR: yazım sırasındaki (ya da odaktan çıkıp
 * dönülen) ara ad başka kolonun başvurularını çalmaz. Başka satırın kaydında olan konumlar yeni kayda alınmaz.
 *
 * Kayıt yalnızca başvuruları taşıyan yapı değişince düşer: kısıtlar/bölümleme anahtarları ya da kolon sayısı kaydın son
 * çıktısından farklıysa (kısıt düzenlemesi, kolon ekle/sil/geri al) ya da taslak dışarıdan değiştiyse (yükleme/yeniden
 * yükleme; `epoch`). Ad değişimi PK üyeliğini değiştirmez (başvurular kolonla taşınır): NOT NULL'a dokunulmaz.
 */
export function createColumnRenames(editor: TableEditor, original: () => TableDef | null) {
  const byRow = new Map<number, ColumnRef[]>();
  let key: string | null = null;
  const keyOf = (d: TableDef) => `${editor.epoch()}|${d.columns.length}|${JSON.stringify([d.constraints, d.partitionBy])}`;
  const refsFor = (d: TableDef, i: number, snap: SelfSnap): ColumnRef[] => {
    if (key !== keyOf(d)) byRow.clear();
    const have = byRow.get(i);
    if (have) return have;
    const claimed = new Set<string>();
    for (const [row, refs] of byRow) if (row !== i) for (const r of refs) claimed.add(JSON.stringify(r));
    const refs = columnRefs(d, d.columns[i]?.name ?? "", snap).filter((r) => !claimed.has(JSON.stringify(r)));
    byRow.set(i, refs);
    return refs;
  };
  return {
    rename: (i: number, next: string) =>
      editor.editSnap((d, snap) => {
        const out = renameTableColumn(d, i, next, refsFor(d, i, snap));
        key = keyOf(out);
        return { d: out, snap };
      }),
    /** Kolonu sil (yeniden adlandırılmış yüklü kolon önce özgün adına döner: geri alma birebir). Kayıt düşer. */
    remove: (i: number) =>
      editor.editSnap((d, snap) => ({ d: removeColumnRestoringName(d, i, original(), refsFor(d, i, snap)), snap })),
  };
}
