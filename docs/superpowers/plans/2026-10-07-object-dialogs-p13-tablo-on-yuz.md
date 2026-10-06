# Nesne Diyalogları — Tablo Arayüzü (Plan 13) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 12 arka ucunun (Table / kolon / kısıt) üstüne **Table designer** arayüzü: Explorer'dan tablo **Oluştur… / Düzenle… / Yeniden adlandır… / Sil…**, kolon ve kısıt düğümlerinden **yalnızca Düzenle…** (tablo diyaloğu ilgili satıra odaklanır); bölüm sekmeleri **Genel** (ad, şema, sahip, tablespace, kalıcılık, fillfactor, yorum, bölüm bilgisi), **Kolonlar** (ızgara: ad, tip seçici, NOT NULL, değer kaynağı (DEFAULT / IDENTITY / GENERATED), collation, yorum, tip değişiminde `USING`; sil/geri al), **Kısıtlar** (PK / UNIQUE / CHECK / FK, kolon seçiciler, FK hedef tablo/kolon seçicisi, DEFERRABLE, NOT VALID / VALIDATE; desteklenmeyenler salt okunur), oluşturmada **Bölümleme**; mevcut SQL önizleme + risk onaylı uygulama akışı; uygulama sonrası başlık/ağaç yenileme ve "oluşturulan sekme = düzenleme sekmesi" eşlemesi.

**Mimari:** Plan 11 kalıbı aynen: istemci SQL üretmez, `ObjectDef::Table(TableDef)` düzenler; tüm mantık saf modüllerde (`table-model.ts`, `table-constraints.ts`, `refresh-model.ts`), Solid bileşenleri ince. Table formu **tek tembel parça** (`TableForm-*.js`: Genel/Kolonlar/Kısıtlar/Bölümleme bölümleri statik içe aktarılır) ve kendi metin modülü `src/app/strings-table.ts` (yalnızca bu parçada). Ana pakete yalnızca menü görünürlüğü kancası girer (Task 1; aynı görevde **adı konmuş bir azaltmayla telafi edilir**, net büyüme hedefi ≤ 0). Sekme anahtarı tabs-store'da (ana paket) değiştirilmez: oluşturulmuş/kimliği değişmiş sekmeyi tembel açıcı denetleyici modelinden (`findAlter`) bulur; başlık mevcut `tabs.rename` ile güncellenir.

**Teknoloji:** SolidJS, Kobalte (`src/ui/Tabs.tsx` sarmalayıcısı), Tailwind 4, vitest (env `node`), mevcut designer çerçevesi (`designer-controller`, `SqlPreview`, `ApplyBar`, `FormParts`).

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§3 risk modeli, §4 arayüz, §5 Table, §6 güvenlik; **"Sözleşme notları"** — özellikle "Table arka ucu (Plan 12) sözleşme notları" paragrafı bağlayıcıdır). Önceki planlar: `docs/superpowers/plans/2026-10-06-object-dialogs-p11-on-yuz.md` (burada tekrarlanmayan tüm Global Constraints ve Review Focus maddeleri geçerlidir) ve `docs/superpowers/plans/2026-10-06-object-dialogs-p12-tablo-arka-uc.md` ("Plan 13 için notlar"). Bağlamalar: `src/ipc/bindings/{TableDef,ColumnDef,ConstraintDef,ConstraintKind,GeneratedDef,IdentityKind,FkAction,FkMatch,PartitionByDef,PartitionKey,PartitionStrategy,Persistence,DdlTarget,DdlFocus,LookupRequest,LookupItem,ObjectDef,PlanView}.ts`.

## Global Constraints

- **Paket bütçesi (denetleyici kararı, bağlayıcı):** ana `index-*.js` bu planın başında **tam 233,733 B**'dir ve pay yoktur. Plan 13 **yalnızca tembel**dir: Table arayüzü ve tüm Table metinleri tembel parçalarda (`TableForm-*.js`, `strings-table` yalnızca bu parçadan içe aktarılır; `strings-designer.ts`'e yalnızca birkaç kısa satır eklenir — o da tembel). **Tek istisna** Task 1'deki menü görünürlüğü kancasıdır (`ddl-kinds.ts` + `actions.ts`; gerekçe: menü ana pakettedir, tablo/kolon/kısıt düğümlerinde eylem göstermek başka türlü mümkün değil). Bu kanca **en çok +1 KB** olabilir ve **aynı görevde** şu adı konmuş azaltmayla telafi edilir: `t.query.export` metin bloğu (`src/app/strings.ts`, yalnızca Query Tool tembel parçasındaki `export.ts`/`ExportUi.tsx`/`QueryTab.tsx` kullanır, ≈1,3 KB) yeni tembel `src/features/query/strings-export.ts`'e taşınır; yetmezse ikinci adlı azaltma `t.query.editorSettings` (yalnızca `EditorSettings.tsx`, QueryTab parçası) → aynı dosya. **Kabul ölçütü:** Task 1 sonunda ve sonraki **her** UI görevinde `bun run build` sonrası ana `index-*.js` ≤ **233,733 B** (hedef: net küçülme; Task 1'de ölçülen değer sonraki görevlerin tavanıdır ve hiçbir sonraki görev onu aşamaz). Doğrulama komutu her görevde aynıdır (bkz. "Boyut doğrulaması").
- **Boyut doğrulaması (her UI görevinin son adımı):**
  ```powershell
  bun run build
  (Get-Item dist/assets/index-*.js).Length            # ≤ Task 1'de kaydedilen değer (≤ 233,733)
  Select-String -Path dist/assets/index-*.js -SimpleMatch "Silinecek kolonlar","Kısıt ekle","Dışa aktarılıyor"   # çıktı BOŞ olmalı
  Get-ChildItem dist/assets -Filter "TableForm-*.js" | Select-Object Name, Length
  ```
  (`"Dışa aktarılıyor"` Task 1'den itibaren ana pakette olmamalı; `ttbl` metinleri hiçbir zaman.)
- **Tembel kural:** ana paket modülleri (`src/app/*` hariç `strings-designer.ts`/`strings-table.ts`, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `ddl-kinds.ts`) Table modüllerini **statik içe aktarmaz**; `strings-table.ts` yalnızca `TableForm*.tsx`, `table-model.ts`, `table-constraints.ts` tarafından içe aktarılır. Lucide: ana paketin kullandığı simgeler (`Plus`, `Pencil`, `X`, `ChevronDown`, `ChevronRight`, `Lock`, `RefreshCw`, `Copy`, `Eye`, `Columns3`, `KeyRound`, `Table2`, `TableProperties`, NodeIcon'daki diğerleri…) Table formunda **kullanılmaz**; yalnızca `Trash2` (IndexForm'da zaten tembel), `Undo2` (QueryTab parçasında zaten var) ve `Settings2` (yeni, yalnızca TableForm) kullanılır.
- **İstemci SQL üretmez:** tip metni, DEFAULT/GENERATED/CHECK/USING/bölümleme ifadeleri **olduğu gibi** gönderilir (yalnızca boş → `null` normalleştirmesi). Tip alanının istemci tarafı denetimi yalnızca **uyarı**dır (asıl doğrulama arka ucun `columns[i].dataType` hatası). Serbest SQL alanlarında `FreeSqlNote` + `\` için `E'…'` ipucu gösterilir.
- **Sözleşme (Plan 12 notları):** `ColumnDef.attnum`/`ConstraintDef.oid` yalnızca yükleme kimliğidir: istemci **asla üretmez/değiştirmez** (yeni satırda `null`; geri alınan satır yüklenen nesnenin kopyasıyla döner). `using` yalnızca alter ve yalnızca yüklü kolonun tipi/collation'ı değiştiyse görünür; tip özgün değere dönünce `null`'a çekilir. Kolon sırası değiştirilemez: yeni kolonlar **sona** eklenir, satır taşıma yoktur; geri alınan yüklü kolon `attnum` sırasındaki yerine döner. `isPartition || inherits` → kolon ve kısıt bölümleri salt okunur (ad/şema/sahip/tablespace/yorum serbest). `partitionBy` yalnızca create; alter'da `partitionText` salt okunur. `defaults(Table)` `columns = []` döner; form ilk kolon için açık çağrı gösterir. Tablespace temizlenemez (`pg_default`). `DdlTarget.focus` doluyken drop/rename **reddedilir** (Plan 12 koruması korunur; ek olarak menü bu eylemleri hiç göstermez).
- **FieldError yolları (Table):** `name`, `schema`, `owner`, `tablespace`, `persistence`, `fillfactor`, `comment`, `columns` (üst düzey → genel şerit, `groupIssues` kuralı), `columns[i]`, `columns[i].name|dataType|notNull|default|identity|generated|collation|comment|using|attnum`, `constraints`, `constraints[i]`, `constraints[i].name|expr|include|include[j]|columns[j]|refSchema|refTable|refColumns[j]|deferrable|initiallyDeferred|notValid|nullsNotDistinct|noInherit|oid`, `partitionBy`, `partitionBy.strategy`, `partitionBy.keys`, `partitionBy.keys[j]`; `old.` önekliler genel şerit. Satır indeksleri **taslaktaki** sıradır (taslak olduğu gibi gönderilir).
- **Uygulama güvenliği** Plan 11 ile aynı ve **değişmez** (`applyGate`, onay kutusu, üretimde ad yazma, kilit, tek uçuş, `plan_changed` sonrası otomatik yükleme yok). Table için yeni kapı **eklenmez**; `DROP COLUMN`/`TYPE` DataLoss, `SET NOT NULL`/kısıt DROP Destructive, `DROP TABLE` (CASCADE'siz de) DataLoss arka uçtan gelir.
- **Düğüm kimlikleri ayrıştırılmaz:** hedef `ddl_target_of`; ek yenilemeler ağaç **yapısı ve etiketleriyle** (ata/çocuk/tür) bulunur, kimlik metni bölünmez.
- **Tepkisellik dersleri:** denetleyicide okunan her yeni `Map`, mevcut `registry` sinyaline bağlanır (`registry(), map.get(...)`); Solid sinyali kullanan her yeni test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));` taşır (vitest env `node` sunucu yapısına çözer); saf model testleri Solid içe aktarmaz.
- **Gizlilik:** SQL/tanım değerleri/yorumlar `console.*`'a, `localStorage`'a, geçmişe yazılmaz; bağlantı URL'si/parola hiçbir çıktıya yazılmaz. Arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce; renkler yalnızca CSS değişkenleri; kalın tek taraflı vurgu kenarlığı yok (seçili satır: `bg-accent/10` zemin + "seçili" rozeti); her alan etiketli, hatalar `aria-describedby`, klavyeyle tam kullanım.
- **Rust değişikliği yok:** gereken tüm lookup'lar mevcut (`types{query}`, `collations`, `tables{schema}`, `table_columns{schema,table}`, `schemas`, `roles`, `tablespaces`). `cargo test --workspace` (Docker'sız) yeşil kalır; Docker matrisi bu planda çalıştırılmaz. Bir sözleşme boşluğu bulunursa **düzeltilmez, raporlanır**.
- Test: `bun run test`, `bun run typecheck`, `bun run build`; git worktree **kullanılmaz**; `src/ipc/bindings`'e satır sonu gürültüsü düşerse `git checkout -- src/ipc/bindings`. Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. `console.*` yok (testlerde spy hariç). Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Review Focus

- **Paket bütçesi:** Task 1 sonrası ve her UI görevinde ana `index-*.js` ≤ kaydedilen değer (≤ 233,733 B); `strings-table`/Table metinleri ana pakette yok; `t.query.export` taşınması davranışı değiştirmez (CSV dışa aktarma metinleri aynı).
- **Kimlik ve fark güvenliği:** istemci `attnum`/`oid` üretmez; kolon yeniden adlandırma kısıt kolon listelerine **konum anlık görüntüsüyle** yayılır (yazım sırasında çakışan ara adlar başka satırın referansını çalmaz); silinen yüklü kolon/kısıt geri alınınca yüklenen nesneyle **birebir** aynı döner ve "değişiklik yok" planı verir; kolon sırası hiçbir yoldan değişmez.
- **Sözleşme sınırları:** `using` yalnızca tip/collation değişen yüklü kolonda; `NOT VALID` yalnızca alter'da yeni FK/CHECK'te; yüklü doğrulanmamış kısıtta yalnızca "Doğrula (VALIDATE)"; `initiallyDeferred` yalnızca `deferrable` iken; IDENTITY seçimi `notNull = true` yapar; mevcut kolonu GENERATED'a çevirme seçeneği sunulmaz; bölüm/kalıtımlı tabloda kolon/kısıt düzenlenemez.
- **Odak ve açma akışı:** kolon/kısıt düğümünden açılınca doğru bölüm + satır seçili/görünür; satır bulunamazsa (silinmiş/desteklenmeyen kısıt) açıklayıcı şerit; kolon/kısıt düğümünde Yeniden adlandır/Sil menüde yok ve açıcı yine reddeder; oluşturma sonrası aynı tablo için "Düzenle…" **ikinci sekme açmaz**; başlık yeni adı gösterir.
- **Ağaç yenileme:** create → koleksiyon; alter → üst koleksiyon + tablonun yüklü Columns/Constraints/Indexes koleksiyonları; şema taşımada yeni şemanın Tables koleksiyonu (yüklüyse); hata/olmayan düğüm sessizce yok sayılır.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts` (+ `actions.test.ts`) | **Ana paket:** `table` + `tableCollection` eylemleri; kolon/kısıt düğümünde yalnızca `editObject` |
| `src/app/strings.ts`, `src/features/query/strings-export.ts` (yeni), `export.ts`, `ExportUi.tsx`, `QueryTab.tsx` | **Ana paket telafisi:** CSV metinleri tembel parçaya |
| `src/app/strings-table.ts` (yeni) | Tüm Table metinleri (`ttbl`); yalnızca Table formu parçası |
| `src/app/strings-designer.ts` | Birkaç kısa satır (`open.rowInTable`), `view.formMissing` kaldırılır |
| `src/features/designer/table-model.ts` (+ test) | Saf: genel alanlar, kolon işlemleri, değer kaynağı, tip uyarısı, yeniden adlandırma yayılımı, sil/geri al, bölümleme, odak eşleme |
| `src/features/designer/table-constraints.ts` (+ test) | Saf: kısıt oluşturma/tür değişimi, kolon seçicileri, FK çiftleri, DEFERRABLE/NOT VALID/VALIDATE kuralları, sil/geri al, sarkan referans, desteklenmeyen sınıflandırma |
| `src/features/designer/refresh-model.ts` (+ test) | Saf: uygulama sonrası başlık ve ek ağaç yenileme kimlikleri |
| `src/features/designer/designer-model.ts`, `designer-controller.ts`, `designer-open.ts` (+ testler) | Odak alanı biçimi, `findAlter`, `onApplied`, collations önbelleği; açıcı: mevcut sekme eşleme, odak, bağlam |
| `src/features/designer/forms-model.ts`, `DesignerView.tsx` | `TABLE_PATHS`, `FORM_PATHS` tam `Record`; `table` formu; `formMissing` kaldırılır |
| `src/features/designer/{TableForm,TableGeneral,TableColumns,TypeInput,TableConstraints}.tsx` | Table formu (tek tembel parça) |
| `docs/smoke-test.md`, spec | Elle kontrol listesi, durum satırı |

---

### Task 1: Ana paket — Table menü kancası ve adı konmuş telafi

**Dosyalar:**
- Değiştir: `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts`, `src/features/explorer/actions.test.ts`, `src/app/strings.ts`, `src/features/query/export.ts`, `src/features/query/ExportUi.tsx`, `src/features/query/QueryTab.tsx`
- Oluştur: `src/features/query/strings-export.ts`

**Arayüzler:**
- Tüketir: `NodeKind`, `TreeState`, `isCollection`, `ROOT` (`tree-model.ts`; `actions.ts` zaten içe aktarır).
- Üretir (`ddl-kinds.ts`, bağımlılıksız kalır):

```ts
export const DDL_OBJECT_KINDS: NodeKind[] = ["schema", "sequence", "extension", "index", "table"];
/** schemaCollection, sequenceCollection, extensionCollection, indexCollection, tableCollection. */
export const DDL_CREATE_COLLECTIONS = DDL_OBJECT_KINDS.map((k) => `${k}Collection` as NodeKind);
/**
 * Tablo satırları: yalnızca "Düzenle…" (tablo diyaloğu ilgili kolona/kısıta odaklanır). Silme ve yeniden adlandırma
 * tablo diyaloğunun içinden yapılır (açıcı `focus` doluyken drop/rename'i ayrıca reddeder).
 */
export const DDL_ROW_KINDS: NodeKind[] = ["column", "constraint"];
export const canCreateIn = (kind: NodeKind) => DDL_CREATE_COLLECTIONS.includes(kind);
export const canEditObject = (kind: NodeKind) => DDL_OBJECT_KINDS.includes(kind);
export const isTableRowKind = (kind: NodeKind) => DDL_ROW_KINDS.includes(kind);
```

- Üretir (`actions.ts`): `actionsFor` içinde (mevcut `canEditObject` dalının hemen ardından):

```ts
/** Kolon/kısıt düğümünün sahibi (koleksiyonlar atlanarak ilk ata) tablo mu (görünüm/MV kolonu ya da domain kısıtı değil). */
function ownedByTable(s: TreeState, id: string): boolean {
  let cur = s.parent.get(id);
  while (cur !== undefined && cur !== ROOT) {
    const kind = s.nodes.get(cur)?.kind;
    if (!kind) return false;
    if (!isCollection(kind)) return kind === "table";
    cur = s.parent.get(cur);
  }
  return false;
}
// actionsFor:
if (canEditObject(node.kind)) out.push("editObject", "renameObject", "dropObject");
else if (isTableRowKind(node.kind) && ownedByTable(s, node.id)) out.push("editObject");
```

  Bölüm (partition) düğümleri `table` türündedir: Düzenle/Yeniden adlandır/Sil alır (arka uç `isPartition` korumasını uygular; DROP bölüm tablosu için not). `partitionCollection`'da "Oluştur…" **yok** (spec §1.3: bölüm oluşturma kapsam dışı). `groupOf`, `disabledReason`, `executeAction` değişmez (sonek kuralı `…Object`).
- Üretir (`strings-export.ts`): `export const texp = { …t.query.export bloğunun birebir kopyası… } as const;` — `button`, `disabledHint`, `optionsTitle`, `delimiter`, `delimiters`, `header`, `bom`, `start`, `dialogTitle`, `filterName`, `statementLabel`, `progressTitle`, `progress`, `cancel`, `done`, `cancelled`, `busy`, `lockedTarget`, `failed`, `defaultName` (metinler **değişmez**). `strings.ts`'ten `query.export` anahtarı silinir; `export.ts`, `ExportUi.tsx`, `QueryTab.tsx`'te `t.query.export.` → `texp.` (`import { texp } from "./strings-export";`).

- [ ] **Adım 1: Başlangıç ölçümü.** `bun run build`; `(Get-Item dist/assets/index-*.js).Length` → **233733** olmalı (değilse dur ve raporla: bütçe tabanı farklı).
- [ ] **Adım 2: Başarısız testler** (`actions.test.ts`, `designer eylemleri` bloğu). `ddlTree()`'ye görünüm kolonu ekle: `add("coll-vcols", "view-1", "columnCollection", "Columns", { hasChildren: true }); add("vcol-1", "coll-vcols", "column", "id");`. Testler:

```ts
it("tablo: Tables koleksiyonunda createObject; tablo ve bölüm düğümünde edit/rename/drop", () => {
  expect(actionsFor(s, node(s, "coll-tables"))).toEqual(["createObject", "refresh", "copyName"]);
  for (const id of ["tbl-orders", "tbl-partition"]) {
    expect(actionsFor(s, node(s, id)).filter((x) => DES.includes(x))).toEqual(["editObject", "renameObject", "dropObject"]);
  }
});
it("tablo kolonu/kısıtı (bölüm kolonu dahil): yalnızca editObject", () => {
  for (const id of ["col-id", "con-1", "col-part"]) {
    expect(actionsFor(s, node(s, id)).filter((x) => DES.includes(x))).toEqual(["editObject"]);
  }
});
it("görünüm kolonu ve koleksiyon düğümleri (Columns/Constraints/Partitions) designer eylemi almaz", () => {
  for (const id of ["vcol-1", "coll-cols", "coll-con", "coll-part"]) {
    expect(actionsFor(s, node(s, id)).filter((x) => DES.includes(x))).toEqual([]);
  }
});
```

  Mevcut "diğer düğümlerde designer eylemi yok" testinin listesinden `tbl-orders`, `col-id`, `con-1`, `coll-tables` çıkarılır (yeni testler kapsar); kalanlar aynı. `disabledReason` testine `col-id` + `editObject` (kilitli → gerekçe) eklenir.
- [ ] **Adım 3:** `bun run test src/features/explorer` — yeni testlerin başarısız olduğunu gör.
- [ ] **Adım 4: Uygula** (`ddl-kinds.ts`, `actions.ts`); ardından telafi: `strings-export.ts` oluştur, `strings.ts`'ten bloğu sil, üç dosyayı güncelle. `rg`/Grep ile `t\.query\.export` → **0 eşleşme**.
- [ ] **Adım 5: Doğrulama.** `bun run test` (tümü), `bun run typecheck`, **Boyut doğrulaması**: ana `index-*.js` ≤ 233,733 B (beklenen ≈ 232,6–232,9 KB; ölçülen değeri commit mesajına ve `docs/smoke-test.md` "Ölçümler" altına bir satırla yaz: "Plan 13 Task 1: index-*.js = N B"). `Select-String … "Dışa aktarılıyor"` boş. Eğer > 233,733: ikinci adlı azaltmayı uygula (`t.query.editorSettings` → `strings-export.ts`'de `teditor` olarak; yalnızca `EditorSettings.tsx` güncellenir) ve yeniden ölç; yine aşıyorsa **dur ve raporla** (başka azaltma bu planda kararlaştırılmadı).
- [ ] **Adım 6: Commit** (`feat(explorer): Table/kolon/kısıt menü kancası; CSV metinleri tembel parçaya (ana paket telafisi, index N B)`).

  Not: Bu görevden Task 5'e kadar dal içinde tablo "Düzenle…" `formMissing` korumasını gösterir; dal `main`'e yalnızca Task 8 sonrası birleşir.

---

### Task 2: Table saf modeli — genel alanlar, kolonlar, bölümleme, odak; metin modülü

**Dosyalar:**
- Oluştur: `src/app/strings-table.ts`, `src/features/designer/table-model.ts`, `src/features/designer/table-model.test.ts`
- Değiştir: `src/features/designer/designer-model.ts` (+ `designer-model.test.ts`: odak alanı biçimi), `src/features/designer/forms-model.ts` (+ `forms-model.test.ts`: `TABLE_PATHS`, `FORM_PATHS` tam `Record`)

**Arayüzler:**
- Tüketir: `TableDef`, `ColumnDef`, `IdentityKind`, `GeneratedDef`, `PartitionByDef`, `PartitionKey`, `PartitionStrategy`, `ObjectDef`, `DdlFocus`, `FieldIssues`, `normalizeEmpty`.
- Üretir (`designer-model.ts`, metinsiz — açıcı da kullanır):

```ts
/** Tablo diyaloğu odak alanı: `column:<attnum>` / `constraint:<oid>` (`requestFocus` alanı; `name` gibi düz alanlarla çakışmaz). */
export function focusFieldOf(f: DdlFocus): string {
  return `${f.kind}:${f.id}`;
}
export function parseFocusField(field: string): DdlFocus | null {
  const m = /^(column|constraint):([0-9]+)$/.exec(field);
  return m ? ({ kind: m[1] as DdlFocus["kind"], id: Number(m[2]) }) : null;
}
```

- Üretir (`forms-model.ts`):

```ts
export const TABLE_PATHS: ReadonlySet<string> = new Set([
  "name", "schema", "owner", "tablespace", "persistence", "fillfactor", "comment", "columns", "constraints", "partitionBy",
]);
/** Tür → bilinen yollar; HER ObjectKind için form vardır (Partial değil; testte denetlenir). */
export const FORM_PATHS: Record<ObjectKind, ReadonlySet<string>> = {
  schema: SCHEMA_PATHS, extension: EXTENSION_PATHS, sequence: SEQUENCE_PATHS, index: INDEX_PATHS, table: TABLE_PATHS,
};
```

  (`DesignerView.tsx`'teki `FORM_PATHS.x!` ünlemleri Task 5'te kalkar; bu görevde tip hâlâ uyumludur.)
- Üretir (`strings-table.ts`) — eksiksiz metin modülü (sonraki görevler yalnızca buradan okur; yeni metin gerekirse **buraya** eklenir):

```ts
// Table diyaloğu metinleri. Yalnızca Table formu parçası (TableForm*.tsx, table-model.ts, table-constraints.ts) içe
// aktarır; strings-designer.ts'e eklenmez (diğer formların parçası büyümesin). PostgreSQL terimleri İngilizce kalır.
export const ttbl = {
  sections: { general: "Genel", columns: "Kolonlar", constraints: "Kısıtlar", partitioning: "Bölümleme", ariaLabel: "Tablo bölümleri" },
  general: {
    schema: "Şema",
    schemaCreateHint: "Tablo bu şemada oluşturulur.",
    schemaAlterHint: "Şema değişirse ALTER TABLE … SET SCHEMA çalışır; bağımlı nesneler etkilenebilir.",
    name: "Ad",
    rename: "Ad (değişirse yeniden adlandırılır)",
    namePlaceholder: "tablo_adı",
    persistence: "Kalıcılık",
    persistenceOptions: { permanent: "Kalıcı (LOGGED)", unlogged: "UNLOGGED (WAL yazılmaz; çökme sonrası boşaltılır)" },
    persistenceAlterNote: "Kalıcılık değişimi tabloyu yeniden yazar ve ACCESS EXCLUSIVE kilit alır.",
    tablespace: "Tablespace",
    tablespaceUnset: "Belirtilmemiş (veritabanı varsayılanı)",
    tablespaceDefault: "varsayılan (pg_default)",
    tablespaceHint: "Tablespace kaldırılamaz; varsayılana dönmek için pg_default'u seçin. Değişince tablo yeniden yazılır ve kilit alır.",
    fillfactor: "Doldurma oranı (fillfactor)",
    fillfactorHint: "10–100; boş = varsayılan (100).",
    partitionText: "Bölümleme anahtarı (PARTITION BY)",
    partitionTextHint: "Bölümleme anahtarı oluşturulduktan sonra değiştirilemez.",
    partitionBadge: "Bölüm (partition)",
    inheritsBadge: "Kalıtımlı (INHERITS)",
    structureLocked:
      "Bu tablo bir bölüm ya da kalıtımlı tablodur: kolon ve kısıtlar üst tablodan yönetilir. Burada yalnızca ad, şema, sahip, tablespace ve yorum değiştirilebilir.",
  },
  partitioning: {
    strategy: "Bölümleme yöntemi",
    none: "Bölümlenmemiş",
    strategies: { range: "RANGE", list: "LIST", hash: "HASH (PG 11+)" },
    hint: "Yalnızca oluştururken. Bölümlerin kendisi bu diyalogda oluşturulmaz.",
    keys: "Anahtarlar",
    noKeys: "En az bir bölümleme anahtarı ekleyin.",
    key: (j: number) => `${j + 1}. anahtar`,
    keyKind: "Anahtar türü",
    keyColumn: "Kolon",
    keyExpression: "İfade",
    expressionPlaceholder: "ör. date_trunc('month', created_at)",
    addKey: "Anahtar ekle",
    removeKey: (j: number, label: string) => `${j + 1}. anahtarı sil (${label})`,
  },
  columns: {
    listLabel: "Kolonlar",
    empty: "Henüz kolon yok; tablo en az bir kolon ister.",
    add: "Kolon ekle",
    row: (i: number, name: string) => `${i + 1}. kolon${name ? ` (${name})` : ""}`,
    newBadge: "yeni",
    selectedBadge: "seçili",
    name: "Ad",
    namePlaceholder: "kolon_adı",
    dataType: "Veri tipi",
    dataTypePlaceholder: "ör. integer, text, numeric(10,2)",
    dataTypeHint: "Listeden seçin ya da yazın (ör. character varying(50), timestamp(3) with time zone, int4[]).",
    typesLoading: "Tipler yükleniyor…",
    notNull: "NOT NULL",
    notNullIdentity: "Kimlik kolonu her zaman NOT NULL'dır.",
    source: "Değer kaynağı",
    sources: {
      none: "Yok",
      default: "Varsayılan (DEFAULT)",
      identity_always: "Kimlik: GENERATED ALWAYS AS IDENTITY",
      identity_by_default: "Kimlik: GENERATED BY DEFAULT AS IDENTITY",
      generated_stored: "Üretilmiş: STORED (PG 12+)",
      generated_virtual: "Üretilmiş: VIRTUAL (PG 18+)",
    },
    sourceNoGenerate: "Mevcut kolon üretilmiş kolona çevrilemez; yeni kolon ekleyin.",
    sourceDropExpression: "Üretilmiş ifade kaldırılırsa (DROP EXPRESSION, PG 13+) değerler sabit kalır.",
    expr: "İfade (SQL)",
    exprDefaultPlaceholder: "ör. now()",
    exprGeneratedPlaceholder: "ör. price * qty",
    details: "Ayrıntılar",
    detailsToggle: (i: number, name: string) => `${i + 1}. kolonun ayrıntıları${name ? ` (${name})` : ""}`,
    collation: "Collation",
    collationPlaceholder: "varsayılan",
    comment: "Yorum (COMMENT)",
    using: "Dönüşüm ifadesi (USING)",
    usingPlaceholder: "ör. amount::numeric",
    usingHint: "Tip değişimi tabloyu yeniden yazar; otomatik dönüşüm yoksa USING ifadesi gerekir. Boş = örtük dönüşüm.",
    typeChanged: "Tip değişti: tablo yeniden yazılır, bağlı indeks ve kısıtlar yeniden oluşturulur (veri kaybı riski).",
    remove: (i: number, name: string) => `${i + 1}. kolonu sil${name ? ` (${name})` : ""}`,
    removed: "Silinecek kolonlar",
    removedNote: "Uygulanınca bu kolonlar ve verileri kalıcı olarak silinir (DROP COLUMN).",
    restore: (name: string) => `Geri al (${name})`,
    orderNote: "Kolon sırası değiştirilemez (PostgreSQL); yeni kolonlar sona eklenir.",
    escapeHint: "Sıradan '…' dizesinde \\ kullanılamaz; kaçış gerekiyorsa E'…' yazın.",
  },
  type: {
    required: "Veri tipi gerekli.",
    tooLong: "Veri tipi en çok 128 karakter olabilir.",
    control: "Veri tipi satır sonu, sekme gibi denetim karakteri içeremez.",
    spaces: "Sözcükler arasında tek boşluk olmalı; başta/sonda boşluk olamaz.",
    unclosedQuote: "Çift tırnaklı ad kapanmamış.",
    comment: "Veri tipi yorum (-- ya da /*) içeremez.",
    forbidden: (ch: string) => `Veri tipi '${ch}' içeremez.`,
    modifier: "Değiştirici yalnızca rakamlardan oluşabilir ve en çok bir tane olabilir: ör. numeric(10,2), varchar(50).",
    array: "Dizi eki yalnızca sonda olabilir: [] ya da [n].",
    serial: "serial gerçek bir tip değildir; kimlik (IDENTITY) kolonu kullanın.",
    serialFix: (type: string) => `${type} + IDENTITY kullan`,
    serverChecks: "Tipin varlığı sunucuda denetlenir; hata önizlemede görünür.",
  },
  constraints: {
    listLabel: "Kısıtlar",
    empty: "Bu tabloda düzenlenebilir kısıt yok.",
    addType: "Eklenecek kısıt türü",
    add: "Kısıt ekle",
    pkExists: "Tabloda zaten bir PRIMARY KEY var.",
    types: { primary_key: "PRIMARY KEY", unique: "UNIQUE", check: "CHECK", foreign_key: "FOREIGN KEY" },
    card: (i: number, label: string) => `${i + 1}. kısıt (${label})`,
    unnamed: (type: string) => `adsız ${type}`,
    name: "Kısıt adı",
    namePlaceholder: "boş = sunucu adlandırır",
    newBadge: "yeni",
    notValidBadge: "doğrulanmamış",
    columns: "Kolonlar",
    include: "Ek kolonlar (INCLUDE, PG 11+)",
    addColumn: "Kolon ekle",
    chooseColumn: "Kolon seçin",
    removeColumn: (name: string) => `${name} kolonunu çıkar`,
    noColumns: "En az bir kolon seçin.",
    nullsNotDistinct: "NULL değerleri eşit say (NULLS NOT DISTINCT, PG 15+)",
    expr: "Koşul (CHECK, SQL)",
    exprPlaceholder: "ör. price >= 0",
    noInherit: "Alt tablolara aktarma (NO INHERIT)",
    refSchema: "Hedef şema",
    refTable: "Hedef tablo",
    refTablePlaceholder: "tablo seçin ya da yazın",
    pairs: "Kolon eşleşmeleri (yerel → hedef)",
    pair: (j: number) => `${j + 1}. eşleşme`,
    localColumn: "Yerel kolon",
    refColumn: "Hedef kolon",
    addPair: "Eşleşme ekle",
    removePair: (j: number) => `${j + 1}. eşleşmeyi sil`,
    matchType: "MATCH",
    matchTypes: { simple: "SIMPLE", full: "FULL" },
    onUpdate: "ON UPDATE",
    onDelete: "ON DELETE",
    actions: { no_action: "NO ACTION", restrict: "RESTRICT", cascade: "CASCADE", set_null: "SET NULL", set_default: "SET DEFAULT" },
    deferrable: "Ertelenebilir (DEFERRABLE)",
    initiallyDeferred: "Başlangıçta ertelenmiş (INITIALLY DEFERRED)",
    notValid: "Mevcut satırları denetleme (NOT VALID)",
    notValidHint: "Kısıt yalnızca yeni satırlara uygulanır; daha sonra VALIDATE ile doğrulanabilir.",
    validate: "Mevcut satırları şimdi doğrula (VALIDATE CONSTRAINT)",
    validateHint: "Tabloyu tarar; doğrulanmamış satır varsa uygulama başarısız olur.",
    scanNote: "Kısıt eklemek tabloyu tarar ve kilitler.",
    dangling: (names: string) => `Bu kısıt artık tabloda olmayan kolona başvuruyor: ${names}. Kısıtı da kaldırın ya da kolonu geri alın.`,
    remove: (i: number, label: string) => `${i + 1}. kısıtı sil (${label})`,
    removed: "Silinecek kısıtlar",
    removedNote: "Uygulanınca bu kısıtlar kaldırılır (DROP CONSTRAINT); bağımlı yabancı anahtar varsa uygulama başarısız olur.",
    restore: (label: string) => `Geri al (${label})`,
    kindFixed: "Mevcut kısıtın türü değiştirilemez; silip yeni kısıt ekleyin.",
  },
  unsupported: {
    title: "Düzenlenmeyen kısıt ve özellikler",
    note: "Bunlar bu diyalogda düzenlenmez ve değişiklik uygulanırken olduğu gibi korunur.",
    notNull: "PG 18 NOT NULL kısıtı (özel seçenekli): kolonun NOT NULL kutusuyla düzenlenmez; Query Tool'da yönetin.",
    exclude: "EXCLUDE kısıtı: bu diyalogda desteklenmez.",
    trigger: "Kısıt tetikleyicisi: bu diyalogda desteklenmez.",
    identity: "Kimlik dizisi seçenekleri (START/INCREMENT…): bu diyalogda düzenlenmez.",
    storage: "Depolama seçeneği: yalnızca fillfactor düzenlenir.",
    typed: "Tipli tablo (OF tip): kolonlar tipten gelir.",
    other: "Modellenmeyen kısıt/özellik.",
  },
  focus: {
    columnMissing: "Gezginde seçilen kolon bu tabloda artık yok (silinmiş ya da yeniden yüklenmeli).",
    constraintMissing:
      "Gezginde seçilen kısıt düzenlenebilir kısıtlar arasında yok: desteklenmeyen bir tür olabilir (aşağıdaki listeye bakın) ya da silinmiş olabilir.",
  },
} as const;
```

- Üretir (`table-model.ts`; hepsi saf ve değişmez — girdi nesnesini asla değiştirmez; sınır dışı indeks → aynı nesne):

```ts
export type TableDraft = TableDef & { kind: "table" };
export function asTable(d: ObjectDef | null): TableDraft | null;                 // d?.kind === "table" ? d : null
export function tableEditPatch(d: ObjectDef, patch: Partial<TableDef>): ObjectDef; // tür uyuşmazsa d
/** Kolon/kısıt yapısı salt okunur mu (yüklenen tanıma göre): bölüm ya da kalıtımlı. */
export function structureLocked(original: TableDef | null): boolean;             // !!original && (original.isPartition || original.inherits)

// ---- kolonlar
export function newColumn(): ColumnDef;   // { attnum: null, name: "", dataType: "", notNull: false, default: null, identity: null, generated: null, collation: null, comment: null, using: null }
export function addTableColumn(d: TableDef): TableDef;                            // sona ekler
export function updateTableColumn(d: TableDef, i: number, f: (c: ColumnDef) => ColumnDef): TableDef;
export function removeTableColumn(d: TableDef, i: number): TableDef;
/** Yüklenen tanımdaki eşi (attnum ile); yeni kolon → null. */
export function originalColumn(original: TableDef | null, c: ColumnDef): ColumnDef | null;
/** Taslakta olmayan yüklü kolonlar (özgün sırayla). */
export function removedColumns(original: TableDef | null, d: TableDef): ColumnDef[];
/** Yüklü kolonu (özgün nesnenin kopyası) attnum sırasındaki yerine geri koyar; zaten varsa ya da bulunmazsa d. */
export function restoreColumn(d: TableDef, original: TableDef, attnum: number): TableDef;

// ---- değer kaynağı (DEFAULT / IDENTITY / GENERATED birbirini dışlar)
export type ValueSource = "none" | "default" | "identity_always" | "identity_by_default" | "generated_stored" | "generated_virtual";
export function valueSourceOf(c: ColumnDef): ValueSource;
/** İfade metni taşınır (default ↔ generated); identity seçilince notNull = true; diğer kaynaklar temizlenir. */
export function withValueSource(c: ColumnDef, s: ValueSource): ColumnDef;
/** Kaynağın ifade metni (default ya da generated.expr; yoksa ""). */
export function valueExpr(c: ColumnDef): string;
export function withValueExpr(c: ColumnDef, text: string): ColumnDef;             // kaynağa göre default/generated.expr'e yazar (boş metin "" olarak kalır: alan hatası arka uçtan)
/**
 * Satırda sunulan kaynaklar: yeni kolon → hepsi; yüklü ve üretilmiş olmayan → none/default/identity_*;
 * yüklü üretilmiş → none (DROP EXPRESSION) + kendi türü (stored/virtual).
 */
export function allowedValueSources(original: ColumnDef | null): ValueSource[];

// ---- tip
/** Tip/collation değişimi: değer özgün tipe ve collation'a dönerse `using` null'a çekilir. */
export function withDataType(c: ColumnDef, original: ColumnDef | null, dataType: string): ColumnDef;
export function withCollation(c: ColumnDef, original: ColumnDef | null, text: string): ColumnDef;   // normalizeEmpty + aynı using kuralı
export function usingVisible(original: ColumnDef | null, c: ColumnDef): boolean;  // original && (dataType || collation farklı)
/** İstemci tarafı ÖN uyarı (hata değil): ilk sorunun metni ya da null. Asıl dilbilgisi arka uçtadır. */
export function typeTextWarning(text: string): string | null;
/** serial ailesi → { dataType } (serial|serial4 → integer, bigserial|serial8 → bigint, smallserial|serial2 → smallint; büyük/küçük harf duyarsız, isteğe bağlı `pg_catalog.`). */
export function serialFix(text: string): { dataType: string } | null;
/** Tip lookup sorgusu: kırpılmış metnin ilk `(`/`[` öncesi kısmı; boş → null (ilk 200 öneri). */
export function typeQueryOf(text: string): string | null;

// ---- yeniden adlandırma yayılımı (konum anlık görüntüsü)
export type ColumnRef =
  | { at: "constraint"; c: number; list: "columns" | "include" | "refColumns"; j: number }
  | { at: "partitionKey"; j: number };
/**
 * `name`e başvuran konumlar: PK/UNIQUE `columns`+`include`, FK `columns` (ve FK tablonun kendisini hedefliyorsa —
 * `refSchema/refTable` = taslağın şema/adı — `refColumns`), bölümleme kolon anahtarları. CHECK ifadeleri SQL'dir,
 * dokunulmaz (PG yüklü CHECK'i kendisi izler). Boş ad → [].
 */
export function columnRefs(d: TableDef, name: string): ColumnRef[];
/** i. kolonun adını `next` yapar ve YALNIZCA verilen konumları aynı değere yazar (konum artık o adı taşımasa bile). */
export function renameTableColumn(d: TableDef, i: number, next: string, refs: readonly ColumnRef[]): TableDef;
/** Tabloyu hedefleyen FK'lerin konumları (refSchema/refTable = taslağın şema/adı). */
export function selfRefs(d: TableDef): number[];
/** Tablo adı/şeması değişimi; verilen kendine-başvuran FK'lerin refSchema/refTable'ı da yeni değere yazılır. */
export function renameTable(d: TableDef, patch: { name?: string; schema?: string }, selfRefIdx: readonly number[]): TableDef;

// ---- bölümleme (yalnız create)
export function withPartitionStrategy(d: TableDef, s: PartitionStrategy | null): TableDef;  // null → partitionBy null; yok→var: { strategy, keys: [] }; var→başka: keys korunur
export function addPartitionKey(d: TableDef, kind: PartitionKey["kind"]): TableDef;        // { kind: "column", name: "" } | { kind: "expression", sql: "" }
export function updatePartitionKey(d: TableDef, j: number, key: PartitionKey): TableDef;
export function removePartitionKey(d: TableDef, j: number): TableDef;
export function switchPartitionKeyKind(k: PartitionKey, kind: PartitionKey["kind"]): PartitionKey;  // metin taşınır (ad ↔ sql)

// ---- odak
/** Odak alanından bölüm + satır: column → columns bölümü ve attnum'u eşleşen satır; constraint → oid; bulunamazsa index null. */
export function rowOfFocus(d: TableDef, field: string): { section: "columns" | "constraints"; index: number | null } | null;

// ---- genel
export type TableSection = "general" | "columns" | "constraints" | "partitioning";
/** Görünen bölümler: create'te partitioning dahil; alter'da yok. */
export function tableSections(mode: "create" | "alter"): TableSection[];
/** Satır hatası var mı (bölüm sekmesinde rozet): `columns[…`/`constraints[…`/`partitionBy…` yollarından biri. */
export function sectionHasIssues(issues: FieldIssues, section: TableSection): boolean;
```

  Kritik uygulamalar (aynen):

```ts
const inRange = (i: number, len: number) => Number.isInteger(i) && i >= 0 && i < len;

export function restoreColumn(d: TableDef, original: TableDef, attnum: number): TableDef {
  if (d.columns.some((c) => c.attnum === attnum)) return d;
  const src = original.columns.find((c) => c.attnum === attnum);
  if (!src) return d;
  // Yüklü kolonlar attnum sırasındadır ve yeni kolonlar her zaman sondadır: hedef yer, kendisinden küçük attnum'lu yüklü
  // kolonların sayısıdır (sıra değişmez → arka uçta "kolon sırası" uyarısı oluşmaz).
  const at = d.columns.filter((c) => c.attnum !== null && c.attnum < attnum).length;
  const columns = [...d.columns];
  columns.splice(at, 0, structuredClone(src));
  return { ...d, columns };
}

export function withValueSource(c: ColumnDef, s: ValueSource): ColumnDef {
  const text = valueExpr(c);
  const base: ColumnDef = { ...c, default: null, identity: null, generated: null };
  switch (s) {
    case "none": return base;
    case "default": return { ...base, default: text };
    case "identity_always": return { ...base, identity: "always", notNull: true };
    case "identity_by_default": return { ...base, identity: "by_default", notNull: true };
    case "generated_stored": return { ...base, generated: { expr: text, stored: true } };
    case "generated_virtual": return { ...base, generated: { expr: text, stored: false } };
  }
}

export function renameTableColumn(d: TableDef, i: number, next: string, refs: readonly ColumnRef[]): TableDef {
  if (!inRange(i, d.columns.length)) return d;
  const columns = d.columns.map((c, k) => (k === i ? { ...c, name: next } : c));
  const constraints = d.constraints.map((k, c) => {
    const mine = refs.filter((r): r is Extract<ColumnRef, { at: "constraint" }> => r.at === "constraint" && r.c === c);
    if (mine.length === 0) return k;
    const kind = structuredClone(k.kind) as Record<string, unknown>;
    for (const r of mine) {
      const list = kind[r.list];
      if (Array.isArray(list) && inRange(r.j, list.length)) list[r.j] = next;
    }
    return { ...k, kind: kind as unknown as ConstraintKind };
  });
  const keys = d.partitionBy?.keys.map((key, j) =>
    refs.some((r) => r.at === "partitionKey" && r.j === j) && key.kind === "column" ? { ...key, name: next } : key);
  return { ...d, columns, constraints, partitionBy: d.partitionBy && keys ? { ...d.partitionBy, keys } : d.partitionBy };
}
```

  `typeTextWarning` kuralları (sırayla; ilk eşleşen döner; hepsi `ttbl.type.*`): `""` → `required`; `> 128` karakter → `tooLong`; `/[\u0000-\u001f\u007f]/` → `control`; başta/sonda boşluk ya da `"  "` → `spaces`; `serialFix(text) !== null` → `serial`; çift tırnaklı bölümler (`/"(?:[^"]|"")*"/g`) çıkarıldıktan sonra kalan metinde `"` → `unclosedQuote`; `--` ya da `/*` → `comment`; `;`, `'`, `\` → `forbidden(ch)`; `[A-Za-z0-9_$ .,()\[\]-]` dışı karakter → `forbidden(ch)`; `(` sayısı > 1 ya da parantez dengesiz ya da parantez içi `/^\s*\d+\s*(,\s*-?\d+\s*)?$/` değil → `modifier`; köşeli parantezler sonda bitişik `(\[\d*\])+$` biçiminde değilse → `array`; aksi halde `null`. (Bilerek eksik: yerleşik çok sözcüklü kalıplar ve 63 bayt sınırı arka uçtadır; uyarı yalnızca bariz hataları erken gösterir.)

- [ ] **Adım 1: Başarısız testler** (`table-model.test.ts`; Solid içe aktarmaz). Yardımcı: `const T = (p: Partial<TableDef> = {}): TableDef => ({ schema: "public", name: "t", owner: null, tablespace: null, persistence: "permanent", columns: [], constraints: [], partitionBy: null, fillfactor: null, comment: null, partitionText: null, isPartition: false, inherits: false, unsupported: [], ...p });` ve `col(name, attnum, p?)`.

```ts
it("addTableColumn sona ekler; newColumn attnum=null ve tüm isteğe bağlılar null", () => {});
it("removeTableColumn/updateTableColumn değişmez; sınır dışı indeks aynı nesneyi döner", () => {});
it("removedColumns: taslakta olmayan yüklü kolonlar özgün sırayla; yeni kolonlar sayılmaz", () => {});
it("restoreColumn: attnum sırasındaki yerine (yeni kolonlardan önce) özgün nesnenin derin kopyasıyla döner; defsEqual(original) → true", () => {
  const o = T({ columns: [col("a", 1), col("b", 2), col("c", 3)] });
  let d = removeTableColumn(removeTableColumn(o, 1), 0);            // [c]
  d = addTableColumn(d);                                            // [c, yeni]
  d = restoreColumn(restoreColumn(d, o, 2), o, 1);                  // [a, b, c, yeni]
  expect(d.columns.map((c) => c.name)).toEqual(["a", "b", "c", ""]);
  expect(d.columns[0]).not.toBe(o.columns[0]);
  expect(restoreColumn(d, o, 2)).toBe(d);                           // zaten var
});
it("valueSourceOf/withValueSource: ifade metni default↔generated taşınır; identity notNull=true yapar; none hepsini temizler", () => {});
it("allowedValueSources: yeni → 6 seçenek; yüklü düz → none/default/identity_*; yüklü stored → none + generated_stored", () => {});
it("withDataType/withCollation: tip özgüne dönünce using null; farklıyken using korunur; usingVisible yalnız yüklü+değişmiş", () => {});
it("typeTextWarning kabul: integer, double precision, character varying(50), numeric(10,2), numeric(10,-2), timestamp(3) with time zone, public.mytype, \"My Type\"[], int4[][], numeric(10,2)[3]", () => {});
it("typeTextWarning ret (doğru metinle): '', 129 karakter, 'int;', \"int'x'\", 'text --', 'a/*b*/', 'numeric(1;2)', 'numeric(a)', 'int[', '\"unterminated', 'a\\nb', 'int  4', ' int', 'x[1]y', 'numeric(1)(2)', 'serial', 'BIGSERIAL', 'pg_catalog.serial4'", () => {});
it("serialFix: serial/serial4→integer, bigserial/serial8→bigint, smallserial/serial2→smallint; 'serial[]' ve 'myserial' null", () => {});
it("typeQueryOf: ' numeric(10,2) ' → 'numeric'; 'int4[]' → 'int4'; '' → null; 'double precision' → 'double precision'", () => {});
it("columnRefs: PK/UNIQUE columns+include, FK columns; kendine-başvuran FK refColumns; başka tabloya FK refColumns hariç; CHECK hariç; bölümleme kolon anahtarı", () => {});
it("renameTableColumn: konum anlık görüntüsü — ara ad başka kolonun adıyla çakışsa da yalnızca anlık görüntüdeki konumlar güncellenir", () => {
  // kolonlar a, ab; UNIQUE(a), UNIQUE(ab). 'a' → 'ab' → 'abc' yazımı: UNIQUE(ab) (öteki kolonun) hiç değişmez.
  const d0 = T({ columns: [col("a", 1), col("ab", 2)], constraints: [uq(["a"], 10), uq(["ab"], 11)] });
  const refs = columnRefs(d0, "a");
  const d1 = renameTableColumn(d0, 0, "ab", refs);
  const d2 = renameTableColumn(d1, 0, "abc", refs);
  expect(d2.constraints.map((k) => (k.kind as { columns: string[] }).columns)).toEqual([["abc"], ["ab"]]);
});
it("selfRefs/renameTable: tabloyu hedefleyen FK'lerin refSchema/refTable'ı adla birlikte güncellenir; başka tabloya FK dokunulmaz", () => {});
it("bölümleme: withPartitionStrategy null/yok→var/strateji değişimi anahtarları korur; anahtar ekle/sil/güncelle/tür değiştir (metin taşınır)", () => {});
it("rowOfFocus: column:2 → columns bölümü ve attnum=2 satırının indeksi; constraint:40 → oid eşleşmesi; bulunamazsa index null; bozuk alan → null", () => {});
it("tableSections: create → general, columns, constraints, partitioning; alter → ilk üçü", () => {});
it("sectionHasIssues: columns[1].dataType → columns; constraints[0].expr → constraints; partitionBy.keys[0] → partitioning; name → general", () => {});
it("structureLocked: isPartition ya da inherits → true; create (original null) → false", () => {});
```

  `designer-model.test.ts`: `focusFieldOf({kind:"column",id:3}) === "column:3"`; `parseFocusField` gidiş-dönüş; `"name"`, `"column:"`, `"column:-1"`, `"index:3"` → `null`. `forms-model.test.ts`: `FORM_PATHS`'in her `ObjectKind` için tanımlı olduğu (`(["schema","sequence","extension","index","table"] as ObjectKind[]).every((k) => FORM_PATHS[k].size > 0)`), `groupIssues([{path:"columns[1].dataType"}, {path:"columns"}, {path:"old.columns[0].attnum"}, {path:"partitionBy.keys[0]"}], TABLE_PATHS)` → `byPath` yalnızca `columns[1].dataType` ve `partitionBy.keys[0]`, general iki ileti.
- [ ] **Adım 2:** `bun run test src/features/designer` — başarısızlığı gör. **Adım 3: Uygula.**
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (yeni modüller henüz hiçbir bileşene bağlı değil; ana boyut Task 1 değeriyle **aynı**).
- [ ] **Adım 5: Commit** (`feat(designer): Table saf modeli (kolonlar, değer kaynağı, tip uyarısı, yeniden adlandırma yayılımı, bölümleme, odak) ve strings-table`).

---

### Task 3: Table kısıt saf modeli

**Dosyalar:**
- Oluştur: `src/features/designer/table-constraints.ts`, `src/features/designer/table-constraints.test.ts`

**Arayüzler:**
- Tüketir: Task 2 (`TableDef`, `ttbl`), `ConstraintDef`, `ConstraintKind`, `FkAction`, `FkMatch`.
- Üretir (saf, değişmez):

```ts
export type ConstraintType = ConstraintKind["type"];   // "primary_key" | "unique" | "check" | "foreign_key"
/** Yeni kısıt (oid null, name null, deferrable/initiallyDeferred/notValid false). FK hedefi varsayılan olarak taslağın şeması. */
export function newConstraint(type: ConstraintType, d: TableDef): ConstraintDef;
//   primary_key → { columns: [], include: [] }; unique → { columns: [], include: [], nullsNotDistinct: false };
//   check → { expr: "", noInherit: false };
//   foreign_key → { columns: [""], refSchema: d.schema, refTable: "", refColumns: [""], matchType: "simple", onUpdate: "no_action", onDelete: "no_action" }
export function addConstraint(d: TableDef, type: ConstraintType): TableDef;        // sona ekler
export function updateConstraint(d: TableDef, i: number, f: (c: ConstraintDef) => ConstraintDef): TableDef;
export function removeConstraint(d: TableDef, i: number): TableDef;
/** Yalnız yeni kısıtta (oid null) tür değişimi: ad korunur; deferrable bayrakları yeni tür desteklemiyorsa false. Yüklü kısıtta c. */
export function switchConstraintType(c: ConstraintDef, type: ConstraintType, d: TableDef): ConstraintDef;
export function canAddPrimaryKey(d: TableDef): boolean;                            // taslakta primary_key yok
export function supportsDeferrable(type: ConstraintType): boolean;                 // pk, unique, fk
export function supportsNotValid(type: ConstraintType): boolean;                   // fk, check
/** DEFERRABLE kapatılınca initiallyDeferred da false. */
export function withDeferrable(c: ConstraintDef, v: boolean): ConstraintDef;
export function withInitiallyDeferred(c: ConstraintDef, v: boolean): ConstraintDef;   // deferrable false iken c
/**
 * NOT VALID denetimi: "add" → yalnız alter'da, yeni (oid null) FK/CHECK; "validate" → yüklü ve özgün tanımda
 * notValid=true olan kısıt ("Doğrula" kutusu notValid'i false yapar); "none" → gösterilmez.
 */
export function notValidMode(mode: "create" | "alter", c: ConstraintDef, original: ConstraintDef | null): "add" | "validate" | "none";
export function originalConstraint(original: TableDef | null, c: ConstraintDef): ConstraintDef | null;   // oid ile
export function removedConstraints(original: TableDef | null, d: TableDef): ConstraintDef[];
/** Yüklü kısıtı (özgün kopyası) özgün sıradaki yerine (önündeki yüklü kısıt sayısı) geri koyar. */
export function restoreConstraint(d: TableDef, original: TableDef, oid: number): TableDef;

// ---- kolon listeleri (PK/UNIQUE columns + include)
export type ColumnListName = "columns" | "include";
export function constraintColumns(c: ConstraintDef, list: ColumnListName): string[];   // check/fk include → []
export function addConstraintColumn(c: ConstraintDef, list: ColumnListName, name: string): ConstraintDef;  // boş/yinelenen ad eklenmez
export function removeConstraintColumn(c: ConstraintDef, list: ColumnListName, j: number): ConstraintDef;
/** Seçilebilir kolonlar: taslaktaki boş olmayan kolon adlarından bu listede (ve PK/UNIQUE'te öteki listede) olmayanlar, taslak sırasıyla. */
export function availableColumns(d: TableDef, c: ConstraintDef, list: ColumnListName): string[];

// ---- FK eşleşmeleri (columns[j] ↔ refColumns[j]; uzunluklar her zaman eşit tutulur)
export function addFkPair(c: ConstraintDef): ConstraintDef;                        // ["", ""] ekler
export function updateFkPair(c: ConstraintDef, j: number, p: { local?: string; ref?: string }): ConstraintDef;
export function removeFkPair(c: ConstraintDef, j: number): ConstraintDef;
/** Hedef şema/tablo değişimi: tablo değişince hedef kolon adları "" yapılır (çift sayısı korunur). */
export function withFkTarget(c: ConstraintDef, p: { refSchema?: string; refTable?: string }): ConstraintDef;
export function withFkActions(c: ConstraintDef, p: { matchType?: FkMatch; onUpdate?: FkAction; onDelete?: FkAction }): ConstraintDef;

// ---- görünüm yardımcıları
/** Kart başlığı: ad ya da "adsız <TÜR>". */
export function constraintLabel(c: ConstraintDef): string;
/** Taslakta artık olmayan kolonlara (boş olmayan) başvurular: PK/UNIQUE columns+include, FK columns (+ kendine-başvuran refColumns). */
export function danglingColumns(d: TableDef, c: ConstraintDef): string[];
export type UnsupportedKind = "notNull" | "exclude" | "trigger" | "identity" | "storage" | "typed" | "other";
/** `TableDef.unsupported` girdisinin sınıfı (yükleyici etiketlerine göre önek): "NOT NULL " → notNull, "EXCLUDE " → exclude, "CONSTRAINT TRIGGER " → trigger, "kimlik dizisi seçenekleri" → identity, "WITH (" → storage, "OF " → typed, diğer → other. */
export function unsupportedKind(entry: string): UnsupportedKind;
```

  Kritik uygulamalar:

```ts
export function notValidMode(mode: "create" | "alter", c: ConstraintDef, original: ConstraintDef | null): "add" | "validate" | "none" {
  const type = c.kind.type;
  if (!supportsNotValid(type)) return "none";
  if (c.oid === null) return mode === "alter" ? "add" : "none";          // create'te NOT VALID hata (Plan 12)
  return original?.notValid ? "validate" : "none";                        // doğrulanmış kısıt "doğrulanmamış" yapılamaz
}

export function restoreConstraint(d: TableDef, original: TableDef, oid: number): TableDef {
  if (d.constraints.some((c) => c.oid === oid)) return d;
  const pos = original.constraints.findIndex((c) => c.oid === oid);
  if (pos < 0) return d;
  const before = new Set(original.constraints.slice(0, pos).map((c) => c.oid));
  const at = d.constraints.filter((c) => c.oid !== null && before.has(c.oid)).length;
  const constraints = [...d.constraints];
  constraints.splice(at, 0, structuredClone(original.constraints[pos]!));
  return { ...d, constraints };
}
```

  `unsupportedKind` önek dizgeleri, `crates/pgt-ddl/src/load.rs`'teki yükleyici etiketleriyle (`"NOT NULL"`, `"EXCLUDE"`, `"CONSTRAINT TRIGGER"`, `"kimlik dizisi seçenekleri"`, `"WITH ("`, `"OF "`) **birebir** eşleşmeli; uygulayıcı bu dosyayı okuyup doğrular, fark varsa yükleyiciyi değiştirmez, yalnızca TS'yi uyarlar ve raporlar.

- [ ] **Adım 1: Başarısız testler** (`table-constraints.test.ts`):

```ts
it("newConstraint: her tür için alanlar; FK varsayılan hedef şeması taslağın şeması ve tek boş eşleşme", () => {});
it("canAddPrimaryKey: taslakta PK varken false (yüklü ya da yeni)", () => {});
it("switchConstraintType: yalnız yeni kısıtta; ad korunur; check'e geçişte deferrable=false; yüklü kısıt aynı nesne", () => {});
it("withDeferrable(false) initiallyDeferred'ı da false yapar; withInitiallyDeferred deferrable kapalıyken etkisiz", () => {});
it("notValidMode: create+yeni → none; alter+yeni FK/CHECK → add; alter+yeni PK → none; yüklü notValid → validate; yüklü doğrulanmış → none", () => {});
it("removedConstraints/restoreConstraint: özgün sırasına döner; derin kopya; defsEqual(original) → true; zaten varsa aynı nesne", () => {});
it("addConstraintColumn: boş ve yinelenen eklenmez; removeConstraintColumn sınır dışı → aynı nesne; check/fk include → []", () => {});
it("availableColumns: boş adlar ve listede olanlar hariç; PK include seçenekleri columns'takileri içermez", () => {});
it("FK çiftleri: add/update/remove uzunlukları eşit tutar; withFkTarget tablo değişince ref adlarını boşaltır, şema değişince de", () => {});
it("danglingColumns: silinen kolona başvuran PK/UNIQUE/FK; kendine-başvuran FK refColumns da; başka tabloya FK refColumns hariç; boş adlar hariç", () => {});
it("unsupportedKind: yükleyici etiketleri (\"NOT NULL c (NOT VALID)\", \"EXCLUDE ex\", \"CONSTRAINT TRIGGER t\", \"kimlik dizisi seçenekleri (id)\", \"WITH (autovacuum_enabled=false)\", \"OF mytype (tipli tablo)\", \"CHECK c (NOT ENFORCED)\" → other)", () => {});
it("constraintLabel: ad varsa ad; yoksa 'adsız PRIMARY KEY' vb.", () => {});
```

- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana boyut değişmez).
- [ ] **Adım 5: Commit** (`feat(designer): Table kısıt saf modeli (PK/UNIQUE/CHECK/FK, VALIDATE, sil/geri al, desteklenmeyenler)`).

---

### Task 4: Açma akışı ve yaşam döngüsü — mevcut sekme eşleme, odak, uygulama sonrası başlık ve ek yenilemeler

**Dosyalar:**
- Değiştir: `src/features/designer/designer-controller.ts` (+ `designer-controller.test.ts`, `designer-reactivity.test.ts`), `src/features/designer/designer-open.ts` (+ `designer-open.test.ts`), `src/app/strings-designer.ts`
- Oluştur: `src/features/designer/refresh-model.ts`, `src/features/designer/refresh-model.test.ts`

**Arayüzler:**
- Tüketir: Task 2 (`focusFieldOf`), `matchesObject`, `defName`, `kindLabel`, `TreeState`, `ancestorOfKind`, `isCollection`.
- Üretir (`designer-controller.ts`):

```ts
/** Başarılı uygulama sonrası (create'te yalnızca `created` doluysa): açıcı başlığı ve ek ağaç yenilemelerini yapar. */
export interface AppliedInfo {
  tabId: string;
  kind: ObjectKind;
  /** Uygulama anındaki kip. */
  mode: DesignerMode;
  /** alter: yüklenen tanım; create: null. */
  before: ObjectDef | null;
  /** Uygulanan taslak. */
  after: ObjectDef;
  /** Uygulama sonrası nesne kimliği (create/rename: `created`; aksi halde eski `object`). */
  object: ObjectRef | null;
}
// DesignerControllerDeps'e:
onApplied?(info: AppliedInfo): void;      // hatası yutulur; uygulama sonucunu etkilemez
// Dönen nesneye:
/** Aynı nesnenin (`gone` olmayan) alter sekmesi — create'ten dönüşen ya da kimliği `created` ile güncellenen dahil. */
findAlter(serverId: string, databaseOid: number, object: ObjectRef): string | null;
```

  `applied()` içinde `refresh(e.refreshNodeId)` satırından hemen sonra: `if (mode === "alter" || out.created) safe(() => deps.onApplied?.({ tabId, kind: m.kind, mode, before: m.original ? cloneDef(m.original) : null, after: cloneDef(m.draft!), object: out.created ?? m.object }))`. `SESSION_LOOKUPS`'a `"collations"` eklenir (oturum boyunca; `types` sorguya bağlı ve 30 sn TTL'de kalır). `findAlter`: `registry()` okumaz (tepkisel değil; yalnız açıcı çağırır) — `for (const [id, e] of entries) { const m = e.model(); if (m.status !== "gone" && matchesObject(m, {serverId, databaseOid, object})) return id; } return null`. `applied()`'daki "Sekme anahtarı eski oid'de kalır (Plan 12'ye bırakıldı)" yorumu "eşleme `findAlter` ile yapılır (anahtar değişmez)" olarak güncellenir.

- Üretir (`refresh-model.ts`, saf; `designer-open` parçasında):

```ts
export interface OpenContext {
  /** Sekmenin açıldığı ağaçtaki veritabanı düğümü (yoksa null). */
  databaseNodeId: string | null;
  /** Table alter: tablo düğümü (tablo düğümünden ya da kolon/kısıt düğümünün tablo atasından); diğerleri null. */
  tableNodeId: string | null;
}
/** Başlık güncellemesi: create→alter ya da ad/şema değişiminde `tdes.tab.edit(kindLabel, after.name)`; aksi halde null. */
export function titleAfterApply(info: AppliedInfo): string | null;
/**
 * `refreshNodeId` dışındaki yenilemeler (yalnızca yüklü — `tree.children.has(id)` — düğümler, tekrarsız, `primary` hariç):
 * (1) table: tablo düğümünün yüklü columnCollection/constraintCollection/indexCollection çocukları;
 * (2) sequence/table şema değişimi: veritabanı düğümünün schemaCollection'ındaki `after.schema` etiketli şemanın
 *     `${kind}Collection` çocuğu. Düğüm bulunamazsa hiçbir şey eklenmez.
 */
export function extraRefreshIds(tree: TreeState, ctx: OpenContext | null, info: AppliedInfo, primary: string | null): string[];
```

- Üretir (`designer-open.ts`):
  - `DesignerOpenDeps.tabs`: `Pick<TabsStore, "state" | "openDesigner" | "activate" | "rename">` (App zaten tam `query.tabs` geçirir → **ana paket değişmez**).
  - Açıcı `contexts: Map<string, OpenContext>` tutar (her açılışta `designer.ids()`'te olmayan girdiler budanır); dönen nesneye `context(tabId): OpenContext | null`.
  - Akış değişiklikleri (mevcut sıranın içinde):
    1. `target.focus && (mode === "drop" || mode === "rename")` → `notify(tdes.open.rowInTable, "info")` (metin: "Kolon ve kısıtlar tablo diyaloğunun içinden silinir ya da yeniden adlandırılır.") — `tdes.open.none` yerine.
    2. Alter (edit/rename): `designerTabKey`'den **önce** `const existing = designer.findAlter(serverId, databaseOid, object)`; varsa `tabs.activate(existing)` ve odak isteği, yeni sekme açılmaz.
    3. Odak: `mode === "rename"` → `requestFocus(id, "name")`; aksi halde `target.focus` doluysa `requestFocus(id, focusFieldOf(target.focus))` (yeni ve mevcut sekmede).
    4. Bağlam: `databaseNodeId = ancestorOfKind(tree, nodeId, ["database"])?.id ?? null`; `tableNodeId = object?.kind === "table" ? (node.kind === "table" ? nodeId : ancestorOfKind(tree, nodeId, ["table"])?.id ?? null) : null`; sekme kimliği belli olunca `contexts.set(id, ctx)`.
  - `createDesigner`: denetleyiciye `onApplied: (info) => { const title = titleAfterApply(info); if (title) deps.tabs.rename(info.tabId, title); for (const id of extraRefreshIds(deps.tree(), opener.context(info.tabId), info, /*primary*/ null)) void Promise.resolve(deps.explorer.refresh(id)).catch(() => {}); }` (`opener` aynı kapanışta sonradan tanımlanır; çağrı uygulama sonrası olduğundan hazırdır). `primary` olarak denetleyicinin `refreshNodeId`'si bilinmediğinden `null` geçer; tekrar yenileme zararsızdır ama `extraRefreshIds` kendi içinde tekrarsızdır.

- [ ] **Adım 1: Başarısız testler.**
  - `designer-controller.test.ts` (mevcut sahte `Api` kalıbıyla; Table tanımları için `tbl()` yardımcı):

```ts
it("findAlter: alter sekmesini bulur; create başarısı (created) sonrası aynı sekme; gone sekme bulunmaz", async () => {});
it("onApplied: alter'da before=original kopyası, after=uygulanan taslak, object=created ?? object; create'te created null ise çağrılmaz", async () => {});
it("onApplied hatası uygulamayı bozmaz (yeniden yükleme yine yapılır, toast tek)", async () => {});
it("Table alter kimliği aynı kalır (rename: created.oid = eski oid): yeniden yükleme aynı oid ile; model.object değişmez", async () => {});
it("Table DataLoss planı (DROP COLUMN) üretimde ad yazılmadan gönderilmez; Err confirmation_required → gateError + önizleme yeniden istenir", async () => {});
it("lookup önbelleği: collations oturum boyunca (TTL sonrası yeni istek yok); types{query} 30 sn", async () => {});
```

  - `designer-reactivity.test.ts` (dosyadaki mevcut `vi.mock("solid-js", …)` korunur): `"column:3"` odak isteği girdi yüklenmeden önce yapılır, memo `"column:3"` görür; `consumeFocus` sonrası `null`.
  - `designer-open.test.ts` — `setup` içinde `tabs`'a `activate: vi.fn((id) => store.activate(id))`, `rename: vi.fn((id, t) => store.rename(id, t))`; `designer`'a `findAlter: vi.fn(() => null)`, `ids: vi.fn(() => [])`. Ağaca tablo düğümleri: `add("coll-tbl", "sch", "tableCollection", "Tables"); add("tbl", "coll-tbl", "table", "orders"); add("coll-col", "tbl", "columnCollection", "Columns"); add("col", "coll-col", "column", "id");` (+ `"sch"` şema düğümü `"db"` altında bir `schemaCollection` içinde).

```ts
it("kolon düğümünden edit: tablo sekmesi açılır ve requestFocus(id, 'column:3')", async () => {});
it("kısıt odağı mevcut sekmede: yeni sekme açmadan activate + requestFocus('constraint:40')", async () => {});
it("findAlter eşleşirse (create'ten dönüşen sekme) openDesigner çağrılmaz, activate edilir", async () => {});
it("focus hedefinde drop/rename: tdes.open.rowInTable bildirimi, bağlanma yok, sekme yok", async () => {});   // mevcut testin metni güncellenir
it("context: tablo düğümünden ve kolon düğümünden açılışta tableNodeId = tablo düğümü; databaseNodeId = db", async () => {});
```

  - `refresh-model.test.ts` (saf, `emptyTree` + `add` kalıbı):

```ts
it("titleAfterApply: create → 'Düzenle: <ad>'; alter ad değişimi → yeni ad; şema değişimi → başlık (ad aynı); değişiklik yok → null", () => {});
it("extraRefreshIds: tablo düğümünün yalnızca YÜKLÜ Columns/Constraints/Indexes koleksiyonları; triggers hariç", () => {});
it("extraRefreshIds: sequence/table şema taşımada yeni şemanın yüklü koleksiyonu; şema/koleksiyon yoksa ya da yüklü değilse eklenmez", () => {});
it("extraRefreshIds: ctx null ya da tablo düğümü ağaçta yoksa yalnızca şema kuralı; sonuç tekrarsız ve primary hariç", () => {});
```

- [ ] **Adım 2–3:** Başarısızlığı gör; uygula.
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (`designer-open-*.js` büyür; ana `index-*.js` Task 1 değeriyle **aynı** — `App.tsx`/`tabs-store.ts`/`ddl-kinds.ts`'e dokunulmadığını `git diff --stat main -- src/app/App.tsx src/features/query/tabs-store.ts` ile doğrula: boş).
- [ ] **Adım 5: Commit** (`feat(designer): Table açma akışı — mevcut sekme eşleme (findAlter), kolon/kısıt odağı, uygulama sonrası başlık ve ek ağaç yenilemeleri`).

---

### Task 5: Table formu kabuğu, Genel ve Bölümleme bölümleri; DesignerView entegrasyonu

**Dosyalar:**
- Oluştur: `src/features/designer/TableForm.tsx`, `src/features/designer/TableGeneral.tsx`
- Değiştir: `src/features/designer/DesignerView.tsx`, `src/app/strings-designer.ts` (`view.formMissing` silinir)

**Arayüzler:**
- Tüketir: `FormProps`, `FieldRow`, `OwnerSelect`, `CommentField`, `FreeSqlNote`, `createFocusOnRequest`, `invalidClass`, `inputClass`, `Tabs`/`TabPanel` (`src/ui/Tabs.tsx`), Task 2 (`asTable`, `tableEditPatch`, `structureLocked`, `renameTable`, `selfRefs`, bölümleme işlevleri, `rowOfFocus`, `tableSections`, `sectionHasIssues`), `tablespaceChoices`, `normalizeTablespaceEdit`, `fillfactorFromInput`, `exactIssues`, `orphanRowIssues`, `ttbl`.
- Üretir:
  - `DesignerView.tsx`: `const FORMS: Record<ObjectKind, FormEntry> = { …, table: { view: lazy(() => import("./TableForm")), paths: FORM_PATHS.table } };` (ünlemler ve `Partial` kalkar); `formMissing` dalı silinir. Sağ panel genişliği türe göre: `classList={{ "w-[46%]": m().kind !== "table", "w-[38%]": m().kind === "table" }}` (diğer sınıflar aynı).
  - `TableForm.tsx` (varsayılan dışa aktarım, `Component<FormProps>`): `Tabs` ile bölümler (`tableSections(mode)`; etiket `ttbl.sections[*]`; `sectionHasIssues` doluysa etikete sr-only " (hata var)" ve `muted` değil **görünür** küçük nokta rozeti `bg-danger`); bölüm gövdeleri: `TableGeneral` (bu görev), `TableColumns`/`TableConstraints` (Task 6–7; bu görevde **bileşen dosyaları oluşturulmaz**, gövde `ttbl.sections.columns` başlıklı boş `section` — Task 6–7 doldurur), `TablePartitioning` (bu görevde, `TableGeneral.tsx` içinde ikinci dışa aktarım). **Odak dağıtıcı** (tek yerde):

```ts
// TableForm içinde:
const [section, setSection] = createSignal<TableSection>("general");
const [highlight, setHighlight] = createSignal<{ section: "columns" | "constraints"; index: number | null; seq: number } | null>(null);
createEffect(() => {
  const req = props.designer.focusRequest(props.tabId);
  const d = draft();
  if (!req || !d || props.model.status !== "ready") return;
  if (req.field === "name") { setSection("general"); return; }        // TableGeneral'daki createFocusOnRequest odaklar (readOnly bekler)
  const row = rowOfFocus(d, req.field);
  if (!row) return;
  setSection(row.section);
  setHighlight({ ...row, seq: req.seq });                              // satır bileşeni görünür yapar/odaklar
  props.designer.consumeFocus(props.tabId, req.seq);                   // salt okunurda da uygulanır (bölüm/kalıtımlı tablo)
});
```

  `highlight` alt bölümlere prop olarak geçer (Task 6–7 kullanır); `index === null` ise bölüm üstünde `ttbl.focus.columnMissing|constraintMissing` `role="status"` şeridi.
  - `TableGeneral.tsx` (`export function TableGeneral(props: FormProps & { draft: TableDraft; original: TableDraft | null })`):
    - **Şema**: yazılabilir girdi + `schemas` datalist (create ipucu `schemaCreateHint`, alter `schemaAlterHint`); düzenleme `renameTable(d, { schema }, selfRefIdx)` — `selfRefIdx` girdi odaklandığında (`onFocus`) `selfRefs(draft)` ile alınır, `onBlur`'da sıfırlanır.
    - **Ad**: alter'da `rename` etiketi; `createFocusOnRequest(props, "name", () => nameEl)`; aynı `selfRefs` anlık görüntüsü kuralı.
    - **Sahip** (`OwnerSelect`), **Kalıcılık** (`select`: permanent/unlogged; alter'da değişmişse `persistenceAlterNote`), **Tablespace** (`select`: boş yalnızca `original?.tablespace == null` iken; `pg_default` ayrı seçenek; `tablespaceChoices`/`normalizeTablespaceEdit` — IndexForm ile aynı kural), **fillfactor** (`fillfactorFromInput`; `type="number" min=10 max=100`), **Yorum** (`CommentField`).
    - Alter'da salt okunur bilgi: `original.partitionText` doluysa "PARTITION BY …" `<code>` + ipucu; `isPartition`/`inherits` rozetleri; `structureLocked` ise `role="note"` şerit `ttbl.general.structureLocked`.
  - `TablePartitioning` (yalnız create): strateji `select` (none/range/list/hash), anahtar listesi (sıra numarasıyla satırlar; tür seçici Kolon/İfade; Kolon → taslak kolon adlarından `select` (+ yazılı değer listede yoksa başa eklenir); İfade → `textarea` + `FreeSqlNote` + `ttbl.columns.escapeHint`; Sil düğmesi `Trash2` + `aria-label` `removeKey(j, label)`), `partitionBy.keys` liste hatası `exactIssues` + `orphanRowIssues(issues, "partitionBy.keys", len)`; anahtar yokken `noKeys`.
    - Tüm denetimler `props.readOnly` iken `disabled`.

- [ ] **Adım 1: Saf kısım için test** — bu görevde yeni saf işlev yok (Task 2'dekiler kullanılır). `forms-model.test.ts`'e: `DesignerView`'ın kullandığı `FORM_PATHS.table === TABLE_PATHS` (zaten Task 2). Bileşen testi yok (ortam `node`).
- [ ] **Adım 2: Uygula.** `rg "formMissing" src` → 0 eşleşme. `rg "strings-table" src` → yalnızca `TableForm.tsx`, `TableGeneral.tsx`, `table-model.ts`, `table-constraints.ts`.
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması**: ana ≤ Task 1 değeri; `TableForm-*.js` ayrı parça olarak listelenir; `Select-String` ana pakette `ttbl` metni bulmaz.
- [ ] **Adım 4: Commit** (`feat(designer): Table formu kabuğu, Genel ve Bölümleme bölümleri; formMissing koruması kaldırıldı`).

---

### Task 6: Kolonlar bölümü — ızgara, tip seçici, değer kaynağı, USING, sil/geri al, odak

**Dosyalar:**
- Oluştur: `src/features/designer/TableColumns.tsx`, `src/features/designer/TypeInput.tsx`
- Değiştir: `src/features/designer/TableForm.tsx` (bölüm gövdesini bağla)

**Arayüzler:**
- Tüketir: Task 2 kolon işlevleri, `createPreviewer` (`preview.ts`; tip sorgusu için 200 ms "son istek kazanır"), `designer.lookup` (`types{query}`, `collations`), `FieldRow`, `CommentField`, `FreeSqlNote`, `exactIssues`, `orphanRowIssues`, `ttbl`.
- Üretir:
  - `TypeInput.tsx`: `export function TypeInput(props: { designer: DesignerController; tabId: string; id: string; value: string; describedBy?: string; invalid: boolean; disabled: boolean; onInput: (v: string) => void })` — yazılabilir `input` + `datalist` (`option value=item.value` ve etiket `item.detail`); odaklanınca ve yazdıkça `typeQueryOf(value)` ile `createPreviewer<LookupItem[]>({ setTimer: setTimeout, clearTimer: clearTimeout }, setItems, () => setItems([]), 200)` üzerinden `designer.lookup(tabId, { kind: "types", query })` (denetleyici önbelleği; hata → boş liste, serbest yazım sürer); `onCleanup(previewer.dispose)`. Yükleme sürerken datalist'te tek `typesLoading` seçeneği **yok** (datalist seçeneği değer sayılır); bunun yerine `aria-busy`.
  - `TableColumns.tsx` (`export function TableColumns(props: FormProps & { draft: TableDraft; original: TableDraft | null; highlight: Highlight | null })`):
    - Liste `role="list"`, her kolon satırı `role="listitem"` + `fieldset` (`legend` sr-only `ttbl.columns.row(i, name)`); satırlar **sıra numarası** listesiyle çizilir (IndexForm kalıbı: `createMemo(() => d().columns.map((_, i) => i), undefined, { equals: (a, b) => a.length === b.length })`) — düzenlemede odak kaybolmaz.
    - Satır üst sırası: **Ad** (`input`; odaklanınca `refs = columnRefs(draft, name)`, yazdıkça `renameTableColumn(d, i, v, refs)`, `onBlur` → `refs = []`), **Veri tipi** (`TypeInput`; `FieldRow` `path=columns[i].dataType`, `warning=typeTextWarning(value)`; `serialFix` doluysa uyarının yanında küçük düğme `serialFix(type)` → tek düzenlemede `withDataType` + `withValueSource(…, "identity_by_default")`), **NOT NULL** (onay kutusu; identity iken işaretli ve `disabled` + `notNullIdentity` ipucu), **Değer kaynağı** (`select`, `allowedValueSources(originalColumn)`; sunulmayan generated seçenekleri için yüklü kolonda `sourceNoGenerate` ipucu; yüklü üretilmişte none seçilirse `sourceDropExpression` notu), **İfade** (yalnız default/generated; `input` + `FreeSqlNote` + `escapeHint`; yol `columns[i].default` ya da `columns[i].generated`), rozetler (`newBadge` attnum null; `selectedBadge` highlight), **Ayrıntılar** düğmesi (`Settings2`, `aria-expanded`, `aria-controls`; `detailsToggle(i, name)`), **Sil** (`Trash2`, `aria-label` `remove(i, name)`).
    - Ayrıntılar alanı (satır başına açık/kapalı sinyal; hatası olan satır **otomatik açık**: `issuesFor(issues, "columns[i].collation"|"…comment"|"…using")` doluysa): **Collation** (`input` + `collations` datalist, `withCollation`), **Yorum** (`CommentField path=columns[i].comment`), **USING** (`usingVisible` iken; `FreeSqlNote`; `usingHint`) ve `usingVisible` iken her zaman görünür `typeChanged` notu (`text-warn-text`).
    - **Silinecek kolonlar**: `removedColumns(original, draft)` boş değilse ayrı liste (`role="list"`, `aria-label=removed`), her biri ad + tip + `Undo2` düğmesi `restore(name)` → `restoreColumn`; altında `removedNote`.
    - **Kolon ekle** (`Button`, simgesiz): `addTableColumn`; yeni satırın ad girdisine `queueMicrotask` ile odak. Altında `orderNote`.
    - Boş durum (`columns.length === 0`): `empty` metni + Kolon ekle.
    - Liste hataları: `exactIssues(issues, "columns")` (genel şeritte de görünür) yerine **yalnızca** `orphanRowIssues(issues, "columns", len)`.
    - Salt okunur: `props.readOnly || structureLocked(original)` → tüm denetimler `disabled`, ekle/sil/geri al gizli; `structureLocked` şeridi TableForm'da (Task 5).
    - **Odak/vurgu:** `highlight.section === "columns" && highlight.index === i` → satıra `bg-accent/10` + `selectedBadge`; `createEffect(on(() => props.highlight?.seq, …))` ile `rowEl.scrollIntoView({ block: "nearest" })` ve düzenlenebilirse ad girdisine, değilse `tabindex="-1"` satır kabına `focus()`.
- [ ] **Adım 1: Saf testler** — gerekli yeni saf işlev çıkarsa (`table-model.ts`'e) önce testi yazılır; bileşen mantığı Task 2 işlevlerine dayanır. Ek test (table-model.test.ts): `withValueSource` + `withDataType` birleşiminin `serialFix` hızlı düzeltmesinde `notNull = true`, `dataType = "integer"`, `identity = "by_default"` ürettiği.
- [ ] **Adım 2: Uygula** ve TableForm'a bağla.
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; `"Silinecek kolonlar"` ana pakette yok; `Settings2` yalnızca `TableForm-*.js`'te).
- [ ] **Adım 4: Commit** (`feat(designer): Table kolon ızgarası (tip seçici, değer kaynağı, USING, sil/geri al, odak)`).

---

### Task 7: Kısıtlar bölümü — PK/UNIQUE/CHECK/FK, VALIDATE, desteklenmeyenler, odak

**Dosyalar:**
- Oluştur: `src/features/designer/TableConstraints.tsx`
- Değiştir: `src/features/designer/TableForm.tsx` (bölüm gövdesini bağla)

**Arayüzler:**
- Tüketir: Task 3 işlevleri, `designer.lookup` (`schemas`, `tables{schema}`, `table_columns{schema,table}`), `FieldRow`, `FreeSqlNote`, `exactIssues`, `orphanRowIssues`, `ttbl`.
- Üretir (`export function TableConstraints(props: FormProps & { draft: TableDraft; original: TableDraft | null; highlight: Highlight | null })`):
  - **Ekle çubuğu**: tür `select` (`types`; PRIMARY KEY seçeneği `canAddPrimaryKey` false iken `disabled` + `pkExists` `title`) + **Kısıt ekle** → `addConstraint`; yeni kartın ad girdisine odak.
  - **Kart** (sıra numarası listesi; `section` + `aria-label=card(i, constraintLabel)`): başlık satırı tür rozeti, `newBadge` (oid null), `notValidBadge` (özgün `notValid`), `selectedBadge`; **Ad** (`input`, boş → `null`, `namePlaceholder`; yol `constraints[i].name`); yeni kısıtta tür `select` (`switchConstraintType`), yüklüde tür sabit + `kindFixed` ipucu; **Sil** (`Trash2`, `remove(i, label)`).
  - Türe göre gövde:
    - PK/UNIQUE: **Kolonlar** ve **INCLUDE** seçicileri — seçili kolonlar sıralı "çip" listesi (her çipte çıkar düğmesi `removeColumn(name)`), altında `availableColumns` `select` + **Kolon ekle**; hatalar `constraints[i].columns[j]` / `constraints[i].include[j]` çipin altında, liste düzeyi `exactIssues`. UNIQUE: `nullsNotDistinct` onay kutusu.
    - CHECK: `textarea` (`expr`, `FreeSqlNote`, `ttbl.columns.escapeHint`), `noInherit` onay kutusu.
    - FK: **Hedef şema** (`input` + `schemas` datalist; `withFkTarget`), **Hedef tablo** (`input` + `tables{schema: refSchema}` datalist; `withFkTarget`), **eşleşme satırları** (yerel kolon `select` (taslak kolon adları) ↔ hedef kolon `input` + `table_columns{refSchema, refTable}` datalist — hedef tablo boşsa liste yok; kendine başvuran FK'de (hedef = taslağın şema/adı) datalist taslak kolon adlarıdır), **Eşleşme ekle/sil**, **MATCH**, **ON UPDATE**, **ON DELETE** `select`'leri.
  - Ortak alt satır: `supportsDeferrable` → **DEFERRABLE** (`withDeferrable`) ve **INITIALLY DEFERRED** (`withInitiallyDeferred`; DEFERRABLE kapalıyken `disabled`); `notValidMode`: `"add"` → **NOT VALID** onay kutusu + `notValidHint`; `"validate"` → **Doğrula** onay kutusu (işaretli = `notValid: false`) + `validateHint`; yeni kısıtta (alter) her zaman `scanNote` notu.
  - `danglingColumns(draft, c)` doluysa kart içinde `role="alert"` değil `text-warn-text` uyarı `dangling(names)` (arka uç planı da uyarı/hata verir; bu yalnızca yerel gösterim).
  - **Silinecek kısıtlar**: `removedConstraints` listesi + `Undo2` **Geri al** (`restoreConstraint`) + `removedNote`.
  - **Düzenlenmeyenler**: `draft.unsupported` boş değilse (alter) salt okunur liste — başlık `unsupported.title`, `note`; her girdi metni + `unsupportedKind`'a göre açıklama (`unsupported.notNull|exclude|…`). Odak `constraint:<oid>` bulunamadıysa (Task 5 şeridi) bu liste görünür kalır ve şerit ona `aria-describedby` ile bağlanır.
  - Salt okunur: `props.readOnly || structureLocked(original)` → tüm denetimler `disabled`, ekle/sil/geri al gizli; desteklenmeyenler listesi her durumda görünür.
  - **Odak/vurgu:** Task 6 ile aynı kural (`highlight.section === "constraints"`), düzenlenebilirse ad girdisine odak.
  - Satır hataları: `orphanRowIssues(issues, "constraints", len)` liste düzeyinde; FK çiftlerinde `orphanRowIssues(issues, \`constraints[${i}].columns\`, pairs)` ve `refColumns` için aynısı.
- [ ] **Adım 1: Saf testler** — yeni saf işlev çıkarsa önce `table-constraints.test.ts`'e. Zorunlu ek test: kendine başvuran FK için hedef kolon seçeneklerinin taslak kolon adları olduğunu belirleyen saf yardımcı `fkRefColumnSource(d, c): "draft" | "lookup" | "none"` (`table-constraints.ts`; hedef boş → none; şema/ad taslakla aynı → draft; aksi halde lookup) ve testi.
- [ ] **Adım 2: Uygula** ve TableForm'a bağla.
- [ ] **Adım 3: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; `"Kısıt ekle"` ana pakette yok).
- [ ] **Adım 4: Commit** (`feat(designer): Table kısıt düzenleyicisi (PK/UNIQUE/CHECK/FK, NOT VALID/VALIDATE, desteklenmeyenler, odak)`).

---

### Task 8: Elle kontrol listesi, spec durumu ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`

**Arayüzler:**
- Tüketir: Task 1–7.
- Üretir: belgeler.

- [ ] **Adım 1: `docs/smoke-test.md`.** Başlık "Duman Testi (Plan 1–9)" aynı kalır; "Nesne diyalogları: Table arka ucu (Plan 12)" bölümündeki "Menüde tablo eylemi henüz yoktur … form henüz yok" cümlesi silinir ve hemen ardına **"## Nesne diyalogları: Table arayüzü (Plan 13)"** bölümü eklenir (ön koşul paragrafı + aşağıdaki maddeler; hepsi `- [ ] … **[—]**`, mevcut stil):
  - Menü: Tables koleksiyonunda **Oluştur…**; tablo ve bölüm düğümünde **Düzenle… / Yeniden adlandır… / Sil…**; tablo kolonu ve kısıtı düğümünde **yalnızca Düzenle…**; görünüm/MV kolonunda ve Columns/Constraints/Partitions koleksiyonlarında designer eylemi yok; kilitli sunucuda soluk + gerekçe.
  - Kolon düğümünden Düzenle → tablo sekmesi **Kolonlar** bölümünde, ilgili satır "seçili" ve görünür; kısıt düğümünden → **Kısıtlar** bölümünde ilgili kart; EXCLUDE kısıt düğümünden → "düzenlenebilir kısıtlar arasında yok" şeridi + Düzenlenmeyenler listesi; aynı tablo zaten açıksa yeni sekme açılmaz, satıra geçer.
  - Oluştur: kolon yokken "en az bir kolon" + Kolon ekle; tip seçici (yazınca öneriler, `int` → integer/int4…; `numeric(10,2)` serbest yazım), `serial` yazınca uyarı + "integer + IDENTITY kullan" düğmesi; değer kaynakları (DEFAULT `now()`, IDENTITY ALWAYS/BY DEFAULT → NOT NULL kilitli, GENERATED STORED; VIRTUAL PG 18 öncesi sunucuda öğretici hata); collation; yorum → önizleme `CREATE TABLE …` + `COMMENT ON …`; Uygula → Tables yenilenir, sekme "Düzenle: <ad>" başlığına geçer, aynı tablo için Gezgin'den Düzenle ikinci sekme açmaz.
  - Oluştur: PK/UNIQUE(+INCLUDE, NULLS NOT DISTINCT)/CHECK/FK (başka tabloya ve kendine), DEFERRABLE/INITIALLY DEFERRED; NOT VALID create'te sunulmaz; UNLOGGED; fillfactor; tablespace; bölümleme RANGE/LIST/HASH (kolon ve ifade anahtarı) → önizleme `PARTITION BY …`.
  - Düzenle: kolon ekle (nullable → Güvenli; NOT NULL varsayılansız → Yıkıcı notu), kolon yeniden adlandır (PK/FK kolon listeleri birlikte güncellenir; önizlemede `RENAME COLUMN`, kısıt DROP+ADD **yok**), yazarken başka kolonun adına geçici olarak denk gelmek diğer kısıtı bozmaz; tip değişimi → USING alanı + "tablo yeniden yazılır" notu, risk "Veri kaybı olabilir", tip geri alınınca USING kaybolur; kolon sil → "Silinecek kolonlar" + Geri al (geri alınca "Değişiklik yok"), üretimde ad yazma; NOT NULL aç/kapa; DEFAULT/IDENTITY değişimleri; yüklü üretilmiş kolonda yalnızca "Yok" (DROP EXPRESSION) seçeneği.
  - Düzenle: kısıt ekle (Yıkıcı değil, "tabloyu tarar" notu; FK/CHECK için NOT VALID), doğrulanmamış kısıtta "Doğrula" → `VALIDATE CONSTRAINT`; kısıt yeniden adlandır → `RENAME CONSTRAINT`; yapısal değişiklik → DROP+ADD; kısıt sil/geri al; silinen kolona başvuran kısıtta yerel uyarı + önizleme uyarısı; PG 18'de özel NOT NULL kısıtı ve EXCLUDE "Düzenlenmeyenler" listesinde, değişiklik uygulanınca korunur.
  - Genel: yeniden adlandır (başlık güncellenir, Tables yenilenir), şema taşı (eski ve — yüklüyse — yeni şemanın Tables koleksiyonu yenilenir), sahip, UNLOGGED ↔ LOGGED (yeniden yazma notu), tablespace (`pg_default`; temizlenemez), fillfactor set/boş, yorum; alter'da PARTITION BY salt okunur.
  - Bölüm tablosu (partition) ve INHERITS çocuğu: rozet + "kolon ve kısıtlar üst tablodan yönetilir" şeridi; Kolonlar/Kısıtlar salt okunur; ad/şema/sahip/tablespace/yorum uygulanır; Sil… bölüm tablosunu siler (DROP TABLE, Veri kaybı onayı).
  - Kolon/kısıt düğümünde Yeniden adlandır/Sil menüde yok; (eski menü durumunda) açıcı "Kolon ve kısıtlar tablo diyaloğunun içinden…" bildirimi.
  - Uygulama sonrası tablonun açık Columns/Constraints/Indexes koleksiyonları yenilenir (yeni kolon Gezgin'de görünür).
  - `plan_changed`: sekme açıkken başka oturumda kolon ekle → Uygula → "Nesne değişti; yeniden yükleyin."; kilit (`lock table … access exclusive`) + 1 sn lock_timeout → "Kilit beklerken zaman aşımı".
  - Kilitli sunucu: form ve Uygula kapalı, önizleme serbest; bağlantı kesilince salt okunur.
  - Klavye: Kolonlar ızgarasında Tab sırası (ad → tip → NOT NULL → kaynak → ifade → Ayrıntılar → Sil), Ayrıntılar `aria-expanded`, çip çıkar düğmeleri etiketli; ekran okuyucu satır/kart adlarını okur; açık/koyu tema; uzun tip/ifade metinleri yatay sayfa kaydırması yaratmaz; dar pencerede (≈1100 px) kolon satırı sarılır, taşmaz.
  - "Ölçümler" altına Task 1 satırı zaten eklendi; bu görevde son ölçüm de eklenir ("Plan 13 sonu: index-*.js = N B; TableForm-*.js = M B").
- [ ] **Adım 2: Spec.** Durum satırı: "Plan 10–13 (Table arayüzü dahil) uygulandı; Plan 14 (View/MatView, Database, Role) kalan". "Table arka ucu (Plan 12) sözleşme notları" paragrafındaki "Geçici: `DesignerView` … `formMissing` …" ve "`ddl-kinds.ts` kümelerinde `table` Plan 13'e kadar yoktur" cümleleri kaldırılır; yerine kısa **"Table arayüzü (Plan 13) notları"**: kolon/kısıt düğümleri yalnızca Düzenle (tablo diyaloğu `column:<attnum>`/`constraint:<oid>` odağıyla açılır); sekme eşleme `findAlter` ile (tabs-store anahtarı değişmez); SQL önizleme ayrı bölüm sekmesi değil, sağ panelde (Plan 11 yerleşimi); sürüm bağımlı alanlar (VIRTUAL, NULLS NOT DISTINCT, INCLUDE, HASH) arayüzde gizlenmez, ipucu + arka uç hatası (sunucu sürümü arayüz modelinde yok); kimlik dizisi seçenekleri düzenlenmez ("Düzenlenmeyenler"); ana paket telafisi (`t.query.export` → `strings-export.ts`).
- [ ] **Adım 3: Bütünleşik doğrulama.** `bun run test` (tümü), `bun run typecheck`, `bun run build` + **Boyut doğrulaması** (ana ≤ Task 1 değeri ≤ 233,733 B; tembel parçalar ve boyutları rapora yazılır: `TableForm-*.js`, `designer-open-*.js`, `DesignerView-*.js`, `strings-designer-*.js`), `cargo test --workspace` (Docker'sız; Rust değişmedi → yeşil), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Adım 4: Kapanış greps:** `console\.` `src/features/designer` → boş; `localStorage|sessionStorage` `src/features/designer` → boş; `dangerouslySetInnerHTML|innerHTML` `src/features/designer` → boş; `strings-table` yalnızca `TableForm*.tsx`, `TableGeneral.tsx`, `TableColumns.tsx`, `TypeInput.tsx`, `TableConstraints.tsx`, `table-model.ts`, `table-constraints.ts`; designer dışından designer içe aktarmaları yalnızca tip içe aktarmaları, `lazy(() => import(...))`/dinamik `import(...)` ve `ddl-kinds.ts` (`designer/` deseniyle `src/features/designer/**` dışını ara); `t\.query\.export` → 0 eşleşme; `attnum:\s*[0-9]|oid:\s*[0-9]` `src/features/designer/Table*.tsx` → 0 (bileşenler kimlik üretmez).
- [ ] **Adım 5: Commit** (`docs(designer): Table arayüzü elle kontrol listesi, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 13 bitişi)

- Table için tam akış: menü → açma (odaklı/odaksız, mevcut sekme eşleme) → düzenleme → önizleme → kapılı uygulama → başlık + ağaç yenileme; silme diyaloğu tablo için mevcut akışla çalışır (DROP TABLE DataLoss).
- Ana paket ≤ 233,733 B (Task 1'de ölçülen değer); Table kodu ve metinleri yalnızca tembel parçalarda.
- Elle kontrol maddeleri `[—]`; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §4 giriş noktaları (Task 1, 4), yerleşim/bölüm sekmeleri (Task 5; SQL önizleme sağ panelde — karar, aşağıda), önizleme/uygulama/hata eşleme (mevcut çerçeve; Task 4 testleri Table planlarıyla regresyon), uygulama sonrası yenileme ve yeniden yükleme (Task 4); §5 Table alanları: şema/ad/sahip/tablespace/UNLOGGED/fillfactor/bölümleme (Task 5), kolon alanları + USING + kimlik/üretilmiş (Task 6), PK/UNIQUE/CHECK/FK + MATCH/ON UPDATE/DELETE/DEFERRABLE/NOT VALID/VALIDATE (Task 7), kolon sırası uyarısı (Task 6 notu + arka uç uyarısı); §1.2 ana paket ölçütü (Task 1 + her görevde boyut doğrulaması); "Plan 13 için notlar" (bölüm düğümleri, `columns = []`, odak koruması, `formMissing` kaldırma) Task 1/4/5/6'da.
- **Bilinçli kararlar:** (1) SQL önizleme ayrı "SQL" bölüm sekmesi değil, Plan 11'deki sağ paneldir (Table için panel %38). (2) Sekme anahtarı yeniden yazılmaz; `findAlter` + `tabs.rename` (ana paket büyümesin). (3) Kolon/kısıt düğümü yalnızca Düzenle (silme/yeniden adlandırma tablo diyaloğunda). (4) Sürüm bağımlı alanlar gizlenmez (arayüzde sunucu sürümü yok); ipucu + arka uç hatası. (5) Kimlik dizisi seçenekleri ve PG 18 özel NOT NULL kısıtları yalnızca "Düzenlenmeyenler"de. (6) Kısıt kolon sırası yalnızca ekleme sırasıyla belirlenir (çip taşıma yok; yeniden sıralamak için çıkar/ekle). (7) Tip denetimi istemcide yalnızca uyarı.
- **Yer tutucu taraması:** "TBD/TODO/sonra" yok; Task 5'teki boş Kolonlar/Kısıtlar gövdeleri Task 6–7'de doldurulur (açıkça belirtilmiş ara durum, dal içi).
- **Tip tutarlılığı:** `focusFieldOf`/`parseFocusField`/`rowOfFocus` aynı `column:<n>`/`constraint:<n>` biçimi; `AppliedInfo` denetleyicide tanımlı, `refresh-model.ts` onu içe aktarır; `Highlight = { section: "columns" | "constraints"; index: number | null; seq: number }` TableForm'da tanımlanır ve Task 6–7 bileşenlerine `import type` ile geçer; `TableDraft` Task 2'de; `FORM_PATHS` artık tam `Record<ObjectKind, …>`.
