# Nesne Diyalogları — Bölüm (Partition) Arayüzü (Plan 37) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 36 arka ucunun üstüne Grup B'nin **arayüzü**: bölümlü tablonun `partitions` koleksiyonunda **Oluştur…** (yeni bölüm tablosu ya da var olan tabloyu bağlama), bölüm düğümünde **Bölüm sınırı…** (sınır değiştirme) ve **Bölümü ayır…** (normal / CONCURRENTLY / FINALIZE); tembel `PartitionForm` (sınır düzenleyicisi), ayırma için silme diyaloğunun bölüm biçimi, uygulama sonrası iki düğüm yenilemesi ve elle kontrol listesi. Rust değişikliği **yoktur** (sözleşme boşluğu bulunursa düzeltilmez, raporlanır).

**Mimari:** Plan 11/13/15/19/35 kalıbı aynen: istemci SQL üretmez, `ObjectDef` düzenler; mantık saf modüllerde (`partition-model.ts`), Solid bileşenleri ince; metinler tembel `src/app/strings-partition.ts`'te. Ana pakete yalnız menü görünürlüğü (`ddl-kinds.ts`, `actions.ts`, `t.menu` iki etiket) girer ve **aynı görevde adı konmuş telafiyle** karşılanır. Açıcı (`designer-open.ts`, tembel) yeni iki kipte `DdlTarget.partition`'ı kullanır; arayüz kimlik ayrıştırmaz.

**Teknoloji:** SolidJS, Tailwind 4, vitest (env `node`), mevcut designer çerçevesi (`designer-controller`, `SqlPreview`, `ApplyBar`, `FormParts`, `ReadOnlyReasons`, `DropDialog`/`drop-controller`, `RowList`).

**Spec:** `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md` — §1, §3, §5, §6; **§11 "Plan 36 sözleşme notları" bağlayıcıdır** (§3 taslağından ayrıldığı yerde §11 geçerlidir). Önceki arayüz planı: `docs/superpowers/plans/2026-11-20-object-dialogs-p35-policy-tablespace-on-yuz.md` — burada tekrarlanmayan Global Constraints maddeleri geçerlidir.

## Kararlar

1. **Menü (tek "Oluştur…"):** `partitionCollection` koleksiyonunda mevcut **Oluştur…** eylemi `PartitionForm`'u açar; form üstünde kip seçimi **"Yeni bölüm tablosu" / "Var olan tabloyu bağla"** (`mode = create | attach`). Spec §3.2'deki ayrı "Bölüm bağla…" menü öğesi **yoktur** (ledger kararı: ana paket baytı; bağlama formda ilk seçimdir). `DDL_CREATE_COLLECTIONS`'a `"partitionCollection"` eklenir (`partition` bir `NodeKind` değildir; `DDL_OBJECT_KINDS` değişmez).
2. **Bölüm düğümü eylemleri:** düğüm `kind === "table"` ve **doğrudan üstü** `partitionCollection` ise mevcut tablo eylemlerine (Düzenle… = Tablo diyaloğu, Sil… = DROP TABLE) ek iki eylem: **`boundObject`** ("Bölüm sınırı…") ve **`detachObject`** ("Bölümü ayır…"), `editObject`/`renameObject`/`privilegesObject`'ten sonra, `maintenance`'tan önce. Adlar `Object` sonekiyle biter (mevcut kural: kip = önek; `DesignerOpenMode` += `"bound" | "detach"`); kilitli sunucuda mevcut `disabledReason` kapatır; `executeAction`'ın designer kolu bu iki eylemi de kapsar.
3. **Açıcı (`designer-open.ts`):** `bound` → `target.partition` ile **Partition alter sekmesi** (`openAlter`, `object = target.partition`; `findAlter` aynı bölümün açık sekmesini etkinleştirir; başlık `tdes.tab.edit(kindLabel("partition"), node.label)`); `detach` → `openDrop({ …, object: target.partition, … })`. `target.partition` yoksa `tdes.open.none` bildirimi. Create (`partitionCollection`) mevcut akış (`target.create.kind === "partition"`).
4. **İkinci yenileme:** `OpenContext`'e `tablesNodeId?: string | null` (= `target.tablesRefreshNodeId ?? null`); `extraRefreshIds` Partition türünde (create/attach/alter) bu düğümü ekler (yalnız yüklüyse — mevcut `tree.children.has` kuralı). Ayırma: `DropInput`'a `extraRefreshNodeIds?: string[]` (= `[target.tablesRefreshNodeId]` varsa); drop denetleyicisi başarıda `refreshNodeId`'den sonra bunları yeniler (hata yutulur). Farklı şemadaki tablonun koleksiyonu yenilenmez (spec §11.4 bilinen sınır).
5. **Oluşturma bağlamı:** `DesignerModel`'e `createContext: CreateContext | null` (yalnız create sekmesinde dolu; `openCreate` yazar). Form üst tablo oid'ini `createContext.tableOid`'den alır (`attachable_tables` lookup'ı: `{ kind: "attachable_tables", parent_oid }`).
6. **Form (`PartitionForm.tsx`):**
   - **Üst tablo** salt okunur satır (`parentSchema.parentTable`, strateji `RANGE|LIST|HASH`, anahtar sayısı).
   - **Create, `mode = create`:** şema (`schemas` lookup; varsayılan `parentSchema`), ad, **alt bölümleme** (isteğe bağlı: "Alt bölümleme yok" + `RANGE|LIST|HASH` seçimi ve anahtar satırları — her satır "kolon" ya da "ifade" + metin; `PartitionKey`), tablespace (`tablespaces` lookup, boş = varsayılan), sahip (`OwnerSelect`), yorum (`CommentField`).
   - **Create, `mode = attach`:** "Bağlanacak tablo" seçimi (`attachable_tables`; seçenek etiketi `şema.ad`; seçim `schema`+`name` yazar); boş listede "Bağlanabilecek tablo yok" notu; Destructive bilgi notu "Tablo taranır ve sınır doğrulanır; üst tablo kilitlenir (Yıkıcı)". Kip `attach`'e geçince `partitionBy/tablespace/owner/comment` **`null`** yapılır (arka uç yok sayar ama gönderilen taslak temiz kalsın); `create`'e dönünce `name = ""`, `schema = parentSchema`.
   - **Alter (Bölüm sınırı…):** bölüm adı salt okunur (`schema.name`), yalnız **sınır düzenleyicisi**; bilgi notu "Sınır değişirse bölüm aynı transaction içinde ayrılıp yeniden bağlanır; tablo yeniden taranır (Yıkıcı)." (yalnız `boundChanged(original, draft)` iken); `detachPending` → uyarı "Yarım kalmış ayırma var; önce Bölümü ayır… ile tamamlayın (FINALIZE)." ve düzenleyici kapalı. Alter'da sahip/yorum/tablespace/alt bölümleme **gösterilmez** (spec §11.2 alter yasakları).
   - **Sınır düzenleyicisi** (`strategy`'ye göre; tüm metin değerleri olduğu gibi gönderilir, kırpılmaz):
     - `range`: `keyCount` satır; her satırda **Başlangıç (FROM)** ve **Bitiş (TO)** için tür seçimi (`Değer` / `MINVALUE` / `MAXVALUE`) + `Değer`'de font-mono metin. Satır sayısı `keyCount`'a sabittir (ekle/sil yok).
     - `list`: değer satırları (`RowList` kalıbı: ekle/sil; `NULL` serbest; boş liste ipucu "En az bir değer").
     - `hash`: `MODULUS` ve `REMAINDER` sayı alanları (`inputmode="numeric"`; geçersiz metin → alan değişmez, mevcut sayı alanı kalıbı).
     - **DEFAULT** onay kutusu (`range`/`list`'te; `hash`'te gösterilmez): işaretlenince `bound = { kind: "default" }`; kaldırınca stratejinin boş iskeleti (`emptyBound(strategy, keyCount)`).
     - İpucu (`FreeSqlNote` yanında): "PG 10–11'de yalnız sabit değer (ör. 10, '2024-01-01'); yorum ve virgüllü ifade kullanılamaz. Metin değerler tek tırnakla yazılır."
   - `props.readOnly || !d.editable` → tüm denetimler `disabled`; `ReadOnlyReasons` üstte.
7. **Ayırma diyaloğu (silme diyaloğunun bölüm biçimi):** `dropOptionsFor` dönüşüne `concurrently: boolean` (yalnız `partition` → `true`); drop denetleyicisinde `concurrently` durumu ve `setConcurrently` (cascade kalıbı: sürüm ilerler, onay sıfırlanır, önizleme yeniden); `dropRequest(object, def, cascade, force, concurrently)` partition'da `concurrently` alanını yazar (diğer türlerde alan **yazılmaz** — JSON geriye uyumlu). `DropDialog` partition'da: başlık "Bölümü ayır", eylem düğmesi "Ayır", açıklama "Tablo silinmez; satırlar üst tablodan görünmez olur.", **CONCURRENTLY** kutusu ("PG 14+; üst tablo kısa kilitlenir, transaction dışında çalışır; DEFAULT bölüm varken kullanılamaz") — yüklü tanım `detachPending` ise kutu yerine not "Yarım kalmış ayırma tamamlanacak (FINALIZE)." Başarı bildirimi "‘ad’ ayrıldı." Onay kapısı mevcut (Destructive → onay kutusu; üretimde ad yazma yok).
8. **Alan yolları:** `PARTITION_PATHS = new Set(["parentSchema", "parentTable", "schema", "name", "bound", "partitionBy", "tablespace", "owner", "comment", "object"])` (`groupIssues` alt yolları kapsar: `bound.from[i]` vb.). Sınır alt yolları ilgili denetimin altında gösterilir: `bound.from[i]`/`bound.to[i]` satır i'nin FROM/TO hücresi, `bound.values[i]` liste satırı, `bound.modulus`/`bound.remainder` alanları; `bound`, `bound.from`, `bound.to`, `bound.values` sınır bölümünün başında. Ayırma diyaloğunda `concurrently`/`object` hataları mevcut diyalog hata şeridinde.
9. **Geçici koruma kaldırılır:** `DesignerView` `formMissing` bileşeni ve `tdes.view.formMissing` metni silinir (`partition` → `lazy(() => import("./PartitionForm"))`).
10. **Kapsam dışı:** ayrı "Bölüm bağla…" menüsü, sınır değerleri için CodeMirror, sunucu sürümüne göre alan gizleme (sürüm kapıları arka uç alan hatasıdır — Plan 13 ilkesi), farklı şemaya bağlanan/ayrılan tablonun şema koleksiyonunun yenilenmesi, bölüm bölme/birleştirme.

## Global Constraints

- **Paket bütçesi (bağlayıcı):** `scripts/bundle-budget.json` tavanları **yükseltilmez** (`mainMaxBytes 126553`, `startupMaxBytes 154226`). Ana pakete yalnız Task 1'in değişiklikleri (`ddl-kinds.ts`, `actions.ts`, `src/app/strings.ts` `t.menu.boundObject`/`detachObject`) girer; **aynı görevde** adı konmuş telafiyle karşılanır: `src/ipc/commands.ts`'teki `api` nesnesinden **ana pakette tüketicisi olmayan** komut(lar) tembel bir komut modülüne taşınır (emsal `src/ipc/qt-more-commands.ts`, `ddl-commands.ts`, `backup-commands.ts`, `dash-commands.ts`). Aday seçimi: her aday için `grep -rn "<komut>" src` ile tüm tüketiciler listelenir; tüketicisi yalnız tembel parçalarda olan (dinamik `import()` ile yüklenen modüller ve onların statik bağımlılıkları; ana paket modülleri: `src/app/*` — `strings-*.ts` hariç —, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `src/ipc/commands.ts`, `ddl-kinds.ts`, `App.tsx`'in statik içe aktardıkları (ör. Inspector)) taşınır; tüketicisi ana pakette olan **taşınmaz**. Taşıma davranışı değiştirmez. Telafi yetmezse **dur ve sor**. Görev sonunda tavanlar **ölçülen değere indirilir** (yalnız düşürme) ve ölçüm commit mesajına yazılır.
- **Boyut doğrulaması (her UI görevinin son adımı):** `npm run build` ve `npm run bundle:check` geçer; yeni form metinlerinden en az iki benzersiz dizge ana `index-*.js`'te **bulunmaz** (`Select-String -Path dist/assets/index-*.js -SimpleMatch "<dizge>"` boş). `PartitionForm-*.js` ayrı parçadır.
- **Tembel kural:** ana paket modülleri yeni form/model/metin modüllerini **statik içe aktarmaz**. Tembel formlarda Kobalte yok; lucide simgelerinden yalnız mevcut designer formlarının kullandıkları. Formlara yalnız `DesignerView`'ın `lazy(() => import(…))` girdileriyle erişilir.
- **İstemci SQL üretmez:** sınır değerleri ve alt bölümleme ifadeleri olduğu gibi gönderilir (kırpılmaz, tırnak eklenmez); altında `FreeSqlNote`.
- **Sözleşme (spec §11):** yeni tanımlar `editable: true, unsupported: []` ile gider (`ddl_defaults` doldurur; form düşürmez); `strategy`, `keyCount`, `detachPending`, `parentSchema`, `parentTable` formda **yazılmaz**; alter'da `partitionBy/tablespace/owner/comment` `null` kalır (yükleyici `null` verir; form bu alanları alter'da göstermez); `mode` alter'da değişmez.
- **Uygulama güvenliği** Plan 11 ile aynı ve **değişmez** (`applyGate`, onay kutusu, kilit, tek uçuş, `plan_changed` sonrası otomatik yükleme yok).
- **Tepkisellik dersleri:** denetleyicide okunan her yeni yapı mevcut `registry` sinyaline bağlanır; Solid sinyali kullanan her test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));`; saf model testleri Solid içe aktarmaz.
- **Gizlilik/erişilebilirlik/tema:** sınır değerleri, adlar ve SQL `console.*`'a ve depolamaya yazılmaz; arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce (`FROM`, `TO`, `MINVALUE`, `MAXVALUE`, `DEFAULT`, `MODULUS`, `REMAINDER`, `CONCURRENTLY`, `FINALIZE`, `RANGE`, `LIST`, `HASH`); renkler yalnız CSS değişkenleri; her alan etiketli (range satırlarında `aria-label` "Anahtar i başlangıcı/bitişi"), hatalar `aria-describedby`; kip seçimi `fieldset`/`legend` radio; satır silme düğmeleri klavyeyle erişilir ve `aria-label`'lıdır.
- **Test:** `npm run test` (vitest), `npm run typecheck`, `npm run build`, `npm run bundle:check`; `cargo test --workspace` (Docker'sız) yeşil kalır (Rust değişmez). `cargo test` `src/ipc/bindings/*.ts`'i LF ile yeniden yazarsa `git checkout -- src/ipc/bindings/`. Git worktree **kullanılmaz** (dal `feat/partition-ui`). Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `graphify-out/` ve `.superpowers/` commit edilmez. Crate genelinde `cargo fmt` yok. Dosyalar CRLF'dir; düzenlemeler satır sonunu korur.

## Review Focus

- **Paket bütçesi:** Task 1 sonrası ve her UI görevinde `bundle:check` geçer; telafi taşıması davranışı değiştirmez; tavanlar yalnız düşer; yeni metinler ana pakette yok.
- **Menü doğruluğu:** `boundObject`/`detachObject` yalnız doğrudan `partitionCollection` altındaki tablo düğümünde; şema altındaki tablo, bölüm tablosunun kolonu/indeksi ve alt bölüm koleksiyonu dışındaki düğümlerde yok; kilitli sunucuda kapalı; `partitionCollection`'da Oluştur var.
- **Sınır düzenleyicisi ↔ model:** range satır sayısı `keyCount`; tür değişimi (`Değer`↔`MINVALUE`) doğru `BoundValue` yazar ve metni korumaz/korur kuralı belgeli; DEFAULT aç/kapa iskeleti; list ekle/sil sonrası `bound.values[i]` hatası doğru satırda (kayma yok); hash sayı alanları; metin kırpılmaz.
- **Kip geçişi ve sözleşme alanları:** create↔attach geçişinde temizlik; attach'te seçilen tablo `schema`+`name`'e doğru yazılır; `editable`/`unsupported`/`strategy`/`keyCount`/`detachPending` gönderilen taslakta korunur; alter'da yasak alanlar `null`.
- **Ayırma ve yenileme:** CONCURRENTLY kutusu yalnız partition'da, istekte yalnız partition'da `concurrently` alanı; `detachPending`'de kutu yok + FINALIZE notu; başarıda iki düğüm yenilenir; Bölüm sınırı sekmesi aynı bölüm için tekildir.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/app/strings.ts` (`t.menu`), `src/ipc/commands.ts` + tembel komut modülü + tüketiciler, `scripts/bundle-budget.json` | **Ana paket (Task 1):** menü kancası, telafi, tavan düşürme |
| `src/features/designer/partition-model.ts` (+ test), `PartitionForm.tsx`, `src/app/strings-partition.ts`, `forms-model.ts` (+ test), `DesignerView.tsx`, `src/app/strings-designer.ts`, `designer-controller.ts` (+ test; `createContext`) | Bölüm formu (Task 2) |
| `designer-open.ts` (+ test), `refresh-model.ts` (+ test), `drop-model.ts` (+ test), `drop-controller.ts` (+ test), `DropDialog.tsx`, `src/app/strings-designer.ts` (ayırma metinleri; tembel) | Açıcı kipleri, yenileme, ayırma diyaloğu (Task 3) |
| `docs/smoke-test.md`, spec | Elle kontrol listesi, durum (Task 4) |

---

### Task 1: Ana paket — menü kancası ve adı konmuş telafi

**Dosyalar:** Değiştir `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts`, `src/features/explorer/actions.test.ts`, `src/app/strings.ts`, `src/ipc/commands.ts`, telafi tüketicileri, `scripts/bundle-budget.json` (yalnız düşürme); Oluştur (gerekirse) tembel komut modülü.

**Arayüzler:**
- `ddl-kinds.ts`: `DDL_CREATE_COLLECTIONS = [...DDL_OBJECT_KINDS.map(...), "partitionCollection"]` (yorum: Plan 37 bölüm oluştur/bağla; `partition` NodeKind değildir).
- `actions.ts`: `ActionId` += `"boundObject" | "detachObject"`; `DesignerOpenMode` += `"bound" | "detach"`; `actionsFor`: Karar 2 (doğrudan üst `partitionCollection` denetimi: `s.nodes.get(s.parent.get(node.id) ?? "")?.kind === "partitionCollection"`); `executeAction` designer kolu iki eylemi kapsar (`action.slice(0, -6)` → `bound`/`detach`).
- `strings.ts` `t.menu`: `boundObject: "Bölüm sınırı…"`, `detachObject: "Bölümü ayır…"`.

- [ ] **Adım 1: Başlangıç ölçümü.** `npm run build` + `npm run bundle:check`: ana **126553**, başlangıç **154226** (değilse dur ve raporla).
- [ ] **Adım 2: Başarısız testler** (`actions.test.ts`): `partitionCollection`'da `createObject`; `partitionCollection` altındaki tablo düğümünde `editObject`, `renameObject`, `privilegesObject`, `boundObject`, `detachObject`, `dropObject` (sırayla; `boundObject`/`detachObject` `privilegesObject`'ten sonra, `maintenance`'tan önce); şema altındaki tablo düğümünde `boundObject`/`detachObject` **yok**; alt bölüm (bölümün `partitionCollection`'ı altındaki tablo) var; bölüm tablosunun kolonunda yok; kilitli sunucuda iki eylem `disabledReason` ile kapalı; `executeAction("boundObject")` → `openDesigner(id, "bound")`, `detachObject` → `"detach"`; `groupOf` iki eylem için 3.
- [ ] **Adım 3: Başarısızlığı gör** (`npm run test -- actions`).
- [ ] **Adım 4: Uygula** (menü + telafi).
- [ ] **Adım 5: Doğrulama.** `npm run test`, `npm run typecheck`, `npm run build`, `npm run bundle:check`. Ana ≤ 126553 ve başlangıç ≤ 154226 olmalı; değilse sıradaki telafi adayı (Global Constraints kuralıyla). Geçince tavanları ölçülen değerlere **indir** (eşitse dokunma). Taşınan komutun adı `src/ipc/commands.ts`'te yok.
- [ ] **Adım 6: Commit** (`feat(explorer): bölüm menüsü (Oluştur, Bölüm sınırı, Bölümü ayır); <komut> tembel modüle (ana paket telafisi, index N B)`).

---

### Task 2: Bölüm formu

**Dosyalar:** Oluştur `src/features/designer/partition-model.ts`, `partition-model.test.ts`, `PartitionForm.tsx`, `src/app/strings-partition.ts`; Değiştir `forms-model.ts` (`PARTITION_PATHS`) + test, `DesignerView.tsx` (`partition` lazy girdisi; `formMissing` silinir), `src/app/strings-designer.ts` (`view.formMissing` silinir), `designer-controller.ts` (+ test; `DesignerModel.createContext`).

**Arayüzler (`partition-model.ts`, saf; Solid içe aktarmaz):**

```ts
import type { BoundValue } from "../../ipc/bindings/BoundValue";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PartitionBound } from "../../ipc/bindings/PartitionBound";
import type { PartitionDef } from "../../ipc/bindings/PartitionDef";
import type { PartitionMode } from "../../ipc/bindings/PartitionMode";
import type { PartitionStrategy } from "../../ipc/bindings/PartitionStrategy";

export function asPartition(d: ObjectDef | null): (PartitionDef & { kind: "partition" }) | null;
/** Tür uyuşmazlığında aynı nesneyi döndürür. */
export function partitionPatch(d: ObjectDef, patch: Partial<PartitionDef>): ObjectDef;
/** Stratejinin boş iskeleti (spec §11.4 `defaults` ile aynı): range → keyCount boş `value`; list → []; hash → 1/0. */
export function emptyBound(strategy: PartitionStrategy, keyCount: number): PartitionBound;
/** Karar 6 kip geçişi: attach → partitionBy/tablespace/owner/comment = null; create → name "", schema = parentSchema. Aynı kip → aynı nesne. */
export function modePatch(d: PartitionDef, mode: PartitionMode): PartitionDef;
/** DEFAULT aç/kapa (kapatınca emptyBound). */
export function defaultPatch(d: PartitionDef, on: boolean): PartitionDef;
export type BoundSide = "from" | "to";
/** Range hücresi tür değişimi: "value" seçilirse önceki metin (aynı hücre value idiyse) korunur, aksi hâlde ""; anahtar sözcükte metin atılır. Range değilse aynı nesne. */
export function rangeKindPatch(d: PartitionDef, side: BoundSide, i: number, kind: BoundValue["kind"]): PartitionDef;
export function rangeValuePatch(d: PartitionDef, side: BoundSide, i: number, sql: string): PartitionDef;
export function listAdd(d: PartitionDef): PartitionDef;            // sona "" ekler
export function listRemove(d: PartitionDef, i: number): PartitionDef; // indeks dışı no-op
export function listValuePatch(d: PartitionDef, i: number, sql: string): PartitionDef;
/** Sayı metni: /^\d+$/ ve ≤ 2^32−1 değilse aynı nesne. */
export function hashPatch(d: PartitionDef, field: "modulus" | "remainder", text: string): PartitionDef;
/** Alter'da sınır değişti mi (yapısal eşitlik). Create'te (original null) false. */
export function boundChanged(original: PartitionDef | null, d: PartitionDef): boolean;
/** Attach seçimi: lookup öğesinden (value = ad, detail = şema) schema/name. */
export function attachPick(d: PartitionDef, name: string, schema: string): PartitionDef;
```

- `PARTITION_PATHS` Karar 8.
- **Form:** Karar 6 (tüm alt maddeler). Sınır alt yolu → hücre eşlemesi Karar 8 (`issues` yardımcısıyla; `FieldRow path`).
- **Denetleyici:** `DesignerModel.createContext: CreateContext | null` (`openCreate` `create.context`'i yazar; alter/acl/default_acl `null`; create→alter dönüşümünde korunması gerekmez).
- **Metinler** `src/app/strings-partition.ts` (`export const tpart = { … } as const;`): Karar 6 etiketleri, notları ve ipuçları.

- [ ] **Adım 1: Başarısız testler** (`partition-model.test.ts`): `emptyBound` üç strateji (`keyCount` 1 ve 3); `modePatch` (attach temizler; create'e dönüş ad/şema; aynı kip aynı nesne; girdi değişmez); `defaultPatch` (aç → default; kapat → iskelet); `rangeKindPatch` (value→minvalue metni atar; minvalue→value "" verir; value→value metni korur; indeks dışı no-op; list'te no-op); `rangeValuePatch` metni kırpmaz; `listAdd`/`listRemove`/`listValuePatch` (indeks dışı no-op); `hashPatch` (`"4"` yazar, `"x"`/`"-1"`/`"4294967296"` aynı nesne); `boundChanged` (create false; eşit false; range değer farkı true; default↔list true); `attachPick`; `asPartition` başka türde null; `partitionPatch` tür uyuşmazlığında aynı nesne. `forms-model.test.ts`: `FORM_PATHS.partition === PARTITION_PATHS`; `bound.from[1]`, `bound.values[0]`, `bound.modulus` alana; `old.name` genel şeride; **her `ObjectKind` için boş olmayan `FORM_PATHS`** (17 tür). `designer-controller.test.ts`: create sekmesinde `createContext` dolu, alter'da `null`.
- [ ] **Adım 2: Başarısızlığı gör.**
- [ ] **Adım 3: Uygula.** `grep -rn "formMissing" src` → 0.
- [ ] **Adım 4: Doğrulama.** `npm run test`, `npm run typecheck`, **Boyut doğrulaması** (`PartitionForm-*.js` ayrı parça; iki benzersiz dizge ana pakette yok; `grep -rn "strings-partition" src` yalnız form/model).
- [ ] **Adım 5: Commit** (`feat(designer): Bölüm formu (oluştur/bağla kipi, sınır düzenleyicisi, alt bölümleme); form yok koruması kaldırıldı`).

---

### Task 3: Açıcı kipleri, ikinci yenileme ve ayırma diyaloğu

**Dosyalar:** Değiştir `designer-open.ts` (+ `designer-open.test.ts`), `refresh-model.ts` (+ test), `drop-model.ts` (+ test), `drop-controller.ts` (+ test), `DropDialog.tsx`, `src/app/strings-designer.ts`.

**Arayüzler:**
- `designer-open.ts`: Karar 3 (`bound`/`detach` kipleri `target.partition` ile; yoksa `tdes.open.none`); `OpenContext.tablesNodeId` (Karar 4); `DropInput.extraRefreshNodeIds` (Karar 4).
- `refresh-model.ts`: `extraRefreshIds` Partition türünde `ctx.tablesNodeId` (yüklüyse, tekrarsız, `primary` hariç).
- `drop-model.ts`: `dropOptionsFor(kind): { cascade; force; concurrently }` (partition: `{false,false,true}`); `dropRequest(object, def, cascade, force = false, concurrently = false)` — `concurrently` yalnız `object.kind === "partition"` iken istek nesnesine yazılır.
- `drop-controller.ts`: `DropState.concurrently`, `setConcurrently(v)` (cascade kalıbı; `dropOptionsFor(kind).concurrently` değilse no-op; yüklü tanım `detachPending` ise no-op), önizleme/uygulama isteğine aktarım, başarıda `extraRefreshNodeIds` yenileme (her biri ayrı; hata yutulur).
- `DropDialog.tsx`: Karar 7 (başlık/düğme/açıklama/kutu/FINALIZE notu/başarı metni partition'da).

- [ ] **Adım 1: Başarısız testler.** `designer-open.test.ts`: `bound` kipi `target.partition` ile `openAlter` çağırır (`object.kind === "partition"`), aynı bölüm ikinci kez açılınca mevcut sekme etkinleşir; `detach` kipi `openDrop`'u `object = partition` ve `extraRefreshNodeIds = [tablesRefreshNodeId]` ile çağırır; `target.partition` yoksa iki kip de `tdes.open.none`; create (`partitionCollection`) mevcut akışla `openCreate` ve bağlamda `tablesNodeId`. `refresh-model.test.ts`: partition create/alter sonrası `tablesNodeId` yüklüyse eklenir, yüklü değilse eklenmez, `primary` ile aynıysa eklenmez; başka türde eklenmez. `drop-model.test.ts`: `dropOptionsFor` tablosu (partition concurrently true, diğerleri false); `dropRequest` partition'da `concurrently` alanı var, tablo/şemada **yok**. `drop-controller.test.ts`: `setConcurrently` sürümü ilerletir/onayı sıfırlar/önizlemeyi `concurrently: true` ile ister; partition dışı türde no-op; `detachPending` tanımda no-op; başarıda ek yenileme çağrılır ve hatası sonucu bozmaz.
- [ ] **Adım 2: Başarısızlığı gör.**
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrulama.** `npm run test`, `npm run typecheck`, **Boyut doğrulaması** (ayırma metinleri ana pakette yok; ana paket değişmedi).
- [ ] **Adım 5: Commit** (`feat(designer): Bölüm sınırı ve Bölümü ayır açılışları, CONCURRENTLY/FINALIZE ayırma diyaloğu, tablolar koleksiyonu yenilemesi`).

---

### Task 4: Elle kontrol listesi, spec durumu ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md`.

- [ ] **Adım 1: `docs/smoke-test.md`** — Plan 36 bölümündeki "GUI maddeleri Plan 37'de" cümlesini "Plan 37 bölümündedir" yap; ardına **"## Nesne diyalogları: Bölüm arayüzü (Plan 37)"** (ön koşul: Docker PG 10, 11, 13 ve 14+; RANGE (iki anahtarlı), LIST ve HASH bölümlü tablolar; bağlanabilir sıradan tablo; üretim etiketli sunucu). Maddeler (hepsi `- [ ] … **[—]**`): menü (Partitions koleksiyonunda Oluştur; bölüm düğümünde Düzenle/Yeniden adlandır/Yetkiler/Bölüm sınırı/Bölümü ayır/Sil; şema altındaki tabloda Bölüm sınırı/ayır yok; kilitli sunucuda kapalı); bölüm oluştur (range iki anahtar MINVALUE/MAXVALUE; list NULL dahil; hash (11+); DEFAULT (11+); alt bölümleme; tablespace/sahip/yorum; sonrası Partitions ve Tables yenilenir; sekme alter'a döner); var olan tabloyu bağla (liste yalnız uygun tablolar; Yıkıcı onay, üretimde ad yazma yok; sınır dışı satır → 23514 ipucu; çakışan sınır → 42P17 ipucu); bölüm sınırı (değişince yeniden bağlama notu + Yıkıcı onay; çakışan yeni sınırda bölüm eski sınırıyla kalır; değişiklik yoksa boş plan); PG 10–11'de ifade/yorumlu değer → alan hatası; bölümü ayır (normal; CONCURRENTLY PG 14+; DEFAULT bölüm varken CONCURRENTLY → 55000 ipucu; PG 13'te CONCURRENTLY alan hatası; tablo şemanın Tables listesinde görünür); yarım kalmış ayırmada yalnız FINALIZE notu ve Bölüm sınırı formu kapalı; salt okunur bölüm (ayrıştırılamayan sınır) gerekçeli; farklı şemaya bağlanan tablonun şema listesi elle yenilenir (bilinen sınır); klavye/erişilebilirlik/tema (range satır etiketleri, liste satırı silme klavyeyle, kip radio grubu, açık/koyu tema, dar pencere ≈1100 px); "Ölçümler" altına son paket ölçümü.
- [ ] **Adım 2: Spec** — durum satırı "Plan 34–37 uygulandı (Grup A–B)"; §11'in ardına kısa **"§12 Plan 37 notları"**: telafi kaydı ve ölçümler, tek "Oluştur…" + kip seçimi kararı (ayrı "Bölüm bağla…" menüsü yok), `boundObject`/`detachObject` eylemleri, `createContext`, ikinci yenileme, ayırma diyaloğu (`concurrently` yalnız partition isteğinde), backlog.
- [ ] **Adım 3: Bütünleşik doğrulama.** `npm run test`, `npm run typecheck`, `npm run build`, `npm run bundle:check`; `cargo test --workspace` (Docker'sız; bindings gürültüsü geri alınır), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Adım 4: Kapanış grep'leri:** `console\.` / `localStorage|sessionStorage` / `innerHTML` yeni dosyalarda boş; `strings-partition` yalnız form/model; `formMissing` → 0.
- [ ] **Adım 5: Commit** (`docs(designer): Bölüm arayüzü elle kontrol listesi, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 37 bitişi)

- Menü → açma → düzenleme → önizleme → kapılı uygulama → iki düğüm yenilemesi; oluştur/bağla/sınır/ayır (CONCURRENTLY, FINALIZE).
- `bundle:check` geçer; tavanlar yalnız düşmüş olabilir; yeni kod/metin yalnız tembel parçalarda.
- Elle kontrol maddeleri `[—]`; `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §3.2 Explorer eylemleri → Task 1 (Karar 1–2; ayrı bağla menüsü yerine kip seçimi — bilinçli sapma), açıcı/hedef → Task 3; §3.1/§11.1–11.2 model ve alan yolları → Task 2; §11.3 ayırma biçimleri ve onay → Task 3 (mevcut kapı); §11.4 yenileme → Task 3; §11.5 ipuçları → mevcut hata şeridi (yeni metin yok); §5 tembel form + bütçe telafisi → Task 1–2; §6 günlük/depolama → Global Constraints + Task 4 grep.
- **Yer tutucu taraması:** telafi komutu Task 1'de ölçümle seçilir (aday kuralı yazılı; tahmin yok); geçici `formMissing` Task 2'de kalkar.
- **Tip tutarlılığı:** `PartitionDef`/`PartitionBound`/`BoundValue`/`PartitionMode`/`PartitionStrategy`/`CreateContext`/`DdlTarget.partition`/`tablesRefreshNodeId`/`PlanRequest.drop.concurrently` (Plan 36 bindings) → Task 2–3; `DesignerOpenMode` `bound`/`detach` (Task 1) → açıcı (Task 3); `createContext` (Task 2) → form attach lookup'ı (Task 2).
