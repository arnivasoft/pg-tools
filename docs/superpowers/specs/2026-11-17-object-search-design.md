# pg-tools — Nesne Arama Tasarımı

- **Tarih:** 2026-11-17
- **Durum:** Plan 32 (`docs/superpowers/plans/2026-11-17-object-search-p32.md`) uygulandı (arka uç Docker PG 10–18 yeşil); elle kontrol bekliyor (`docs/smoke-test.md` "Nesne arama (Plan 32)"). Uygulama notları: §7.
- **Kapsam:**
  - Bir veritabanında nesneleri **adına göre** arama: pgAdmin'deki "Search Objects" karşılığı.
  - Sonuca gidildiğinde Object Explorer ağacında düğümün ataları sırayla açılır, düğüm seçilir ve görünür kılınır. Inspector seçili düğümü kendiliğinden gösterir.
  - Tanım içeriğinde (fonksiyon gövdesi, view tanımı) arama **yoktur** (kullanıcı kararı; §6 takip adayı).
- **Kullanıcı kararları (2026-11-17):**
  - Arama ada göre ve seçili veritabanında yapılır.
  - Sonuç listesinden ağaçtaki düğüme gidilir.
  - Ana paket tavanı **asla yükseltilmez**. Arama arayüzü tembel parçadadır; ana pakete gelen her bayt, yalnız tembel parçaların kullandığı kodun ana paketten çıkarılmasıyla telafi edilir. Telafi yetmezse durulup sorulur.
  - Arama metni ve sonuç adları günlüğe yazılmaz (nesne adları kullanıcı verisidir); `tracing`'e yalnız sayılar ve süre gider.

---

## 1. Mevcut altyapı (yeniden kullanılır)

- **Düğüm kimlikleri** (`crates/pgt-catalog/src/node.rs`): `NodeId::Node { server, path: Vec<Seg> }`, metin biçimi `srv:<uuid>/…`. Bir nesnenin kimliği **sahibinin kimliği + nesne parçası**, bir koleksiyonun kimliği **sahibinin kimliği + koleksiyon parçası**dır. Örnek zincir (tablo kolonu):
  - `srv:U` → `srv:U/dbs` → `srv:U/db:D` → `srv:U/db:D/schemas` → `srv:U/db:D/sch:S` → `srv:U/db:D/sch:S/tables` → `srv:U/db:D/sch:S/tbl:T` → `srv:U/db:D/sch:S/tbl:T/columns` → `srv:U/db:D/sch:S/tbl:T/col:N`.
  - Arayüz düğüm kimliğini **ayrıştırmaz**; ata zinciri arka uçtan gelir.
- **Ağaç yapısı** (`kinds/mod.rs` `list_children`): veritabanı → `schemas`, `extensions`; şema → `tables` + kayıt defterindeki şema koleksiyonları (`views`, `matviews`, `functions`, `procedures` (PG11+), `trigfuncs`, `sequences`, `types`, `domains`); tablo → `columns`, `constraints`, `indexes`, `triggers`, `policies`, (bölümlenmişse) `partitions`; view → `columns`; MV → `columns`, `indexes`.
- **Listeleme kuralları** (aramanın birebir uyması gerekir, yoksa sonuç ağaçta bulunamaz): `sql/*.sql`.
  - Tablolar: `relkind IN ('r','p') AND NOT relispartition`. Bölümler (`relispartition`) üst tablonun `partitions` koleksiyonundadır; çok düzeyli bölümlemede zincir iç içedir.
  - Fonksiyonlar: `prokind IN ('f','w')`, dönüş `trigger` değil; trigger fonksiyonları ayrı (`trigfuncs`). `pg_depend deptype='i'` olanlar gizli. PG10'da `prokind` yok (`.pg10.sql` sürümleri).
  - Tipler: `typtype IN ('e','c','r','b')`, tablo satır tipleri ve dizi tipleri hariç. Domainler ayrı (`typtype='d'`).
  - Sequence'lar: `relkind='S'`, kimlik (identity) sequence'ları (`deptype='i'`) gizli.
  - Trigger'lar: `NOT tgisinternal`. İndeksler: tablo ve MV altında. Kısıtlar: tablo altında (domain kısıtları domain düğümünde listelenmez).
  - Sistem nesneleri: `schemas.sql` `is_system` (pg_catalog, information_schema, pg_toast*, pg_temp_*); "Sistem nesnelerini göster" kapalıyken gizli. Kayıt defteri (`engine.rs`) satır bazında `system` bayrağıyla süzer.
- **Bağlantı:** `pgt_catalog::db::query(&mgr, server, Target::Database(oid), &VersionedSql, params)`; sürüm seçimi `VersionedSql`.
- **Ön yüz:**
  - Ağaç: `ExplorerController` (`expand`, `select`, `ensureConnected`, `setFilter`), `tree-model.ts`. `TreeView` seçili satırı kendiliğinden kaydırır (`scrollToIndex`).
  - Bağlam menüsü: `features/explorer/actions.ts` (grup 3 eylemleri; `schemaDiff` emsali).
  - Tembel diyalog emsali: Bakım (`App.tsx` `maintenanceId` sinyali + `lazy` + `everOpened`). Diyalog bileşeni `ui/Dialog` (Kobalte; tembel parçada).
  - Komut sarmalayıcı emsali: `features/schema-diff/diff-commands.ts` (tembel parçada, `commands.ts`'e eklenmez).

## 2. Kararlar

### 2.1 Aranan türler

| Tür (`SearchKind`) | Ağaçta yeri | Ata zinciri (D=db, S=şema) |
|---|---|---|
| `schema` | veritabanı → Schemas | `srv`, `dbs`, `db:D`, `schemas` |
| `table` | şema → Tables (bölüm değil) | … `sch:S`, `tables` |
| `partition` | üst tablo → Partitions (iç içe) | … `tables`, `tbl:P0`, `partitions`, `tbl:P1`, `partitions` … |
| `view` | şema → Views | … `sch:S`, `views` |
| `matview` | şema → Materialized Views | … `sch:S`, `matviews` |
| `function` | şema → Functions | … `sch:S`, `functions` |
| `procedure` | şema → Procedures (PG11+) | … `sch:S`, `procedures` |
| `triggerFunction` | şema → Trigger Functions | … `sch:S`, `trigfuncs` |
| `sequence` | şema → Sequences | … `sch:S`, `sequences` |
| `type` | şema → Types | … `sch:S`, `types` |
| `domain` | şema → Domains | … `sch:S`, `domains` |
| `column` | tablo/view/MV → Columns | … `<rel>`, `columns` |
| `index` | tablo/MV → Indexes | … `<rel>`, `indexes` |
| `constraint` | tablo → Constraints | … `<tbl>`, `constraints` |
| `trigger` | tablo → Triggers | … `<tbl>`, `triggers` |
| `policy` | tablo → Policies | … `<tbl>`, `policies` |
| `extension` | veritabanı → Extensions | `srv`, `dbs`, `db:D`, `extensions` |

- `<rel>` bir tablo, bölüm, view ya da MV olabilir; bölüm içindeki kolonların zinciri bölümün kendi zincirini içerir.
- Roller ve tablespace'ler sunucu düzeyindedir, aranmaz.
- Her türün süzgeci §1'deki listeleme SQL'iyle **aynıdır** (aynı `WHERE`); sistem şemalarındaki nesneler "Sistem nesnelerini göster" kapalıyken dönmez. Kolonlar: `attnum > 0 AND NOT attisdropped`.

### 2.2 Eşleşme ve sıralama

- Eşleşme büyük/küçük harf duyarsız alt dizgi: `name ILIKE '%' || $q || '%' ESCAPE '\'`. `$q` arka uçta kaçırılır (`\`, `%`, `_` önüne `\`). Arama metni kırpılır; boşsa sorgu atılmaz, boş sonuç döner. En fazla 200 karakter (`invalid`).
- Fonksiyon/procedure/trigger fonksiyonu için eşleşme `proname` üzerindedir; görüntülenen ad ağaçtakiyle aynı `ad(argümanlar)` biçimidir.
- Sıralama: tam eşleşme (büyük/küçük harf duyarsız), sonra önek eşleşmesi, sonra diğerleri; her grupta ad (`COLLATE "C"`), sonra şema adı, sonra tür.
- **Sınır:** en fazla 500 sonuç; daha fazlası varsa `truncated: true` ("İlk 500 sonuç; aramayı daraltın").
- Tür süzgeci: istekte `kinds: SearchKind[]` (boş = hepsi). Süzülmeyen türlerin alt sorguları hiç çalıştırılmaz.

### 2.3 Sonuç biçimi

```
SearchHit {
  id: string,            // ağaçtaki düğüm kimliği
  path: string[],        // ata zinciri: sunucu düğümünün çocuğundan (`srv:U/dbs`) düğümün ebeveynine kadar, sırayla
  kind: NodeKind,        // ağaçtaki düğüm türü (simge için)
  name: string,          // ağaçtaki etiket
  schema: string | null, // şema adı (şema/extension için null)
  owner: string | null,  // sahip nesne adı (kolon/indeks/kısıt/trigger/policy/bölüm için tablo ya da view adı)
  system: boolean,       // sistem şemasında mı
}
SearchResult { hits: SearchHit[], truncated: boolean }
```

- `path` ve `id` arka uçta **listelemede kullanılan aynı `NodeId` yapıcılarıyla** üretilir (tek kaynak). Entegrasyon testi her zincir adımının bir önceki düğümün `list_children` sonucunda bulunduğunu doğrular.

### 2.4 IPC

- `search_objects(serverId: Uuid, databaseOid: u32, query: String, kinds: Vec<SearchKind>) -> SearchResult`.
- Sunucu bağlı değilse `not_connected` (diğer katalog komutları gibi). Bağlantı ve parola sorma ön yüzdedir (`connect` bağımlılığı).
- Hedef veritabanı bağlantıya izin vermiyorsa (`datallowconn = false`) sunucunun hatası döner.
- Kilitli (salt okunur) sunucuda da çalışır (yalnız katalog okur).
- İptal yoktur: sorgu katalog üzerinde tek bir `UNION ALL`'dur ve sınırlıdır (§4). Ön yüz yeni arama başlayınca eski yanıtı nesil sayacıyla yok sayar.

### 2.5 Ön yüz

- **Giriş:** bağlam menüsünde **"Nesne ara…"**. Veritabanı düğümünde ve bir veritabanının altındaki her düğümde (koleksiyonlar dahil) görünür; hedef, düğümün veritabanıdır (`qtTargetOf` ile çözülür; ön yüz kimliği ayrıştırmaz). Sunucu düğümünde görünmez. Grup 3.
- **Diyalog** (tembel parça, modal): başlık "Nesne ara — `<veritabanı>` (`<sunucu>`)".
  - Arama kutusu (odak açılışta burada), 250 ms gecikmeyle arar; Enter hemen arar.
  - Tür süzgeci: "Tümü" + tür onay kutuları (açılır menü değil, sarılan bir satır; klavyeyle erişilebilir).
  - Sonuç listesi: satır başına tür adı, ad, `şema.sahip` bağlamı. Sanal liste gerekmez (≤500). Satır sayısı ve `truncated` uyarısı listenin üstünde.
  - Klavye: ↓/↑ arama kutusundan listeye ve liste içinde gezinir, Enter seçili sonuca gider, Esc kapatır.
  - Durumlar: "Aranıyor…", "Sonuç yok", hata metni (`errorText`), arama kutusu boşken ipucu ("En az bir karakter yazın").
- **Sonuca gitme:** çift tık ya da Enter → diyalog kapanır ve:
  1. Ağaç süzgeci temizlenir (süzülmüş görünüm hedefi gizleyebilir).
  2. Sunucu düğümünün ağaçtaki ataları (grup) ve sunucu düğümü açılır (`expand`; gerekirse bağlanır).
  3. `path` sırayla `expand` edilir; her adımdan sonra düğüm ağaçta yoksa (bu arada silinmiş, sistem nesneleri gizlenmiş vb.) durulur ve "Nesne ağaçta bulunamadı; listeyi yenileyin." bildirimi gösterilir. Yüklü bir liste eskiyse (nesne aramadan sonra eklenmiş ama ağaç önceden yüklenmiş) o liste **bir kez** yeniden yüklenip yeniden denenir.
  4. Düğüm seçilir (`select`); `TreeView` onu görünür kılar ve ağaca odak verilir. Inspector seçimi kendiliğinden gösterir.
- **Hafıza:** diyalog son arama metnini ve tür süzgecini veritabanı başına modül düzeyinde hatırlar (uygulama oturumu boyunca; `localStorage`'a yazılmaz).
- **Ana paket:** yalnız eylem kimliği/etiketi, `App.tsx`'teki açma kancası ve `ExplorerController`'a eklenen `reveal(path, id)` metodu ana pakettedir. Diyalog, komut sarmalayıcısı (`search-commands.ts`), metinler (`strings-search.ts`) ve model tembeldir.

### 2.6 Hatalar

- `not_connected`: ön yüz `connect` ile bağlanıp bir kez yeniden dener; bağlanamazsa hata metni.
- `invalid` (200 karakteri aşan arama): arama kutusu altında metin.
- Sunucu hataları (`code: "query"` vb.): `errorText` ile.

## 3. Test

- **Rust birim:** arama metni kaçırma (`%`, `_`, `\`, Türkçe harfler), uzunluk sınırı, boş metin, tür süzgecinden SQL parçalarının seçimi, sıralama anahtarı.
- **Rust entegrasyon (Docker PG 10–18, `scripts/test-matrix.ps1`):** fikstürde her türden en az bir nesne, iç içe bölüm, view ve MV kolonu, bir extension, sistem şeması nesnesi (`show_system` kapalı/açık), kimlik sequence'ı (dönmemeli), iç trigger (dönmemeli), PG10'da procedure yok. Her sonuç için `path` zincirinin her adımı bir önceki düğümün `list_children` çıktısında bulunur ve `id` son adımın çocuğudur. `truncated` 500'de. Büyük/küçük harf ve `%`/`_` içeren adlar.
- **Ön yüz (vitest):** model (gecikmeli arama, nesil sayacı, süzgeç, klavye gezinmesi, hafıza), `ExplorerController.reveal` (sırayla açma, eksik düğümde durma, eskimiş listeyi bir kez yenileme, süzgeç temizleme, gruptaki sunucu), eylem görünürlüğü.
- **Elle:** `docs/smoke-test.md` "Nesne arama (Plan 32)" `[—]` maddeleri.

## 4. Performans hedefi

- 10.000 tablo / 200.000 kolonlu bir veritabanında tüm türlerde arama < 500 ms (yerel sunucu). Kolon araması `pg_attribute` taramasıdır; sınır (`LIMIT 501`) dış sorgudadır, sıralama bunun öncesindedir. Hedef tutmazsa kolon türü yalnız açıkça seçildiğinde aranır (plan uygulama notunda kayda geçer).

## 5. Bilinen sınırlar (ilk sürüm)

- Tanım içeriğinde (gövde, view tanımı, yorum) arama yok.
- Roller ve tablespace'ler aranmaz.
- Tüm veritabanlarında (sunucu genelinde) arama yok.
- Sonuç listesi aramadan sonra eskiyebilir; gitme sırasında bulunamazsa bildirim gösterilir.

## 6. Takip adayları

- Tanım içeriğinde arama (`prosrc`, `pg_get_viewdef`, yorumlar).
- Sunucu genelinde arama (bağlantıya izin veren tüm veritabanları).
- Sonuç satırında bağlam menüsü (Query Tool'da aç, verileri görüntüle).

## 7. Plan 32 uygulama notları

**Kararlar:**

- **Menü yeri:** `searchObjects` eylemi `actionsFor`'da Query Tool eylemlerinin (grup −1) hemen ardına, grup 3'ün başına eklenir; görünürlük `ancestorOfKind(state, id, ["database"]) !== null` (düğümün kendisi dahil). Koleksiyonların listesi de bu öneki alır (`[...q, "createObject", …]`). `disabledReason` kilidi uygulamaz (ad `Object` ile bitmez: designer soneki kuralı korunur).
- **Hedef çözümü (App):** `qtTargetOf(id)` sunucu ve veritabanı OID'ini verir; veritabanı adı ağaçtaki veritabanı atasının etiketi, sunucu adı ağaçtaki sunucu düğümünün etiketidir (`serverOf(...).label`; sunucu düğümünün etiketi kayıttaki addır, `query.servers()` taraması yerine — ana paket baytı). Hedef `{ ...target, databaseOid, databaseName, serverName }` olarak kurulur.
- **Diyalog** (`src/features/search/SearchDialog.tsx`, tembel): `<Show keyed>` her açılışta yeni bir model kurar, kapanınca `dispose`. Odak arama kutusunda kalır (`role="combobox"`, `aria-controls`, `aria-expanded`, `aria-activedescendant`); liste `role="listbox"`, satırlar `role="option"` + `aria-selected`; satırlarda `mousedown` varsayılanı engellenir (odak kutudan çıkmaz). Etkin satır `scrollIntoView({ block: "nearest" })` ile görünür kalır. Durum satırı tek bir `role="status"` / `aria-live="polite"` bölgesidir (ipucu, "Aranıyor…", sayı + kırpılma uyarısı, "Sonuç yok.", hata).
- **Model eklemeleri** (`search-model.ts`): `setActive(i)` (tek tıkla etkinleştirme; aralık dışı yok sayılır) ve `activeHit()`'in yazım gecikmesi ya da arama sürerken `null` dönmesi: Enter eski sonuç listesindeki satıra gitmez, hemen arar (spec §2.5 "Enter hemen arar" ile "Enter seçili sonuca gider" çelişkisini çözer).
- **Tür etiketi:** satırdaki tür adı `ts.kinds`'ten `NodeKind` → `SearchKind` eşlemesiyle (`materializedView` → `matview`) alınır. Bölümler ağaçta `table` türündedir; satırda "Tablo" görünür (ön yüz kimlik ayrıştırmaz).
- **Bağlam:** `şema.sahip` (`[schema, owner].filter(Boolean).join(".")`); sistem nesnesi satırı soluk (`text-muted`).
- **Gitme:** diyalog önce `onClose()` ile kapanır, sonra `onReveal(hit, serverId)`; `false` → `notify(ts.notFound, "error")`, ret → `toPayload(e).message` (ad içermez). Başarıda App ağaca odak verir (`treeEl()?.focus()`).
- **Yükleme hatası:** Bakım diyaloğu emsaliyle `lazy` + `<Suspense>`; ayrı bir "yüklenemedi" bildirimi yok. `ts.chunkLoadFailed` bu yüzden kullanılmıyor (tembel modülde, ana pakete etkisi yok).
- Diyalog `prefetchIdle` listesine eklenmedi (ana paket baytı); ilk "Nesne ara…"da iner.

**Paket ölçümleri ve telafi kaydı** (`npm run build` + `npm run bundle:check`, 2026-11-17). Plan 32 öncesi tavanlar: ana paket 126,623 B, açılış kümesi 154,296 B. Tavan yükseltilmedi; her adımda aday komutların tüm çağrı yerleri `rg` ile denetlendi, ana paketten (yani `src/main.tsx`'ten dinamik içe aktarma olmadan ulaşılan bir modülden) çağrılan hiçbir komut taşınmadı:

| Adım | Ana paket | Açılış kümesi | `mapDeps` |
|---|---|---|---|
| Task 4 sonrası (`reveal`) | 127,168 | 154,841 | 40 |
| Task 6 bağlantısı (menü eylemi, metin, App kancası, tembel diyalog) | 128,165 | 155,838 | 41 |
| 1. Designer'a özgü `ddlDefaults`, `ddlLoad`, `ddlLoadAcl`, `ddlLoadDefaultAcl` → `features/designer/designer-commands.ts`; QueryTab parçasına özgü `qtExplain`, `qtComplete`, `qtCheck`, `qtFormat`, `qtExportCsv`, `qtHistoryList`, `qtHistoryClear`, `settingsSetHistory`, `settingsSetEditor` → `features/query/tab-commands.ts` | 127,255 | 154,928 | 41 |
| 2. QueryTab parçasına özgü `qtAnalyze`, `qtTx`, `qtHistoryPushMany`, `qtDraftsClear` → `tab-commands.ts` | 127,052 | 154,725 | 41 |
| 3. `previewPgadminImport`, `applyPgadminImport` → `features/servers/import-commands.ts` (ImportDialog); `testConnection` → `features/servers/connection-commands.ts` (ServerDialog) | 126,855 | 154,528 | 41 |
| 4. Birden çok tembel parçanın ortak `ddlLookup`, `ddlPreview`, `ddlApply`, `ddlCancel` → `src/ipc/ddl-commands.ts` (designer, bakım, yedekleme, şema karşılaştırma; ayrı ortak parça) | 126,632 | 154,305 | 42 |
| 5. App `openSearch`: sunucu adı `serverOf(...).label`, hedef yayılımla kurulur | **126,582** | **154,255** | 42 |

Tavanlar ölçülen değerlere indirildi: `mainMaxBytes` 126,582, `startupMaxBytes` 154,255 (sıfır pay). Adım 1'in iki taşıması birlikte ölçüldü (−910 B). Taşınan komutların tüketicileri komutları ya doğrudan yeni modülden alır (QueryTab, ExportUi, HistoryView, ImportDialog, ServerDialog, diff-opener) ya da enjekte edilen `api`'ye yayılımla ekler (`createDesigner`: `{ ...deps.api, ...designerCommands, ...ddlCommands }`, `MaintenanceDialog`: `{ ...deps.api, ...maintenanceCommands, ...ddlCommands }`, `BackupDialogs`: `ddlLookup`); `Pick<Api, …>` türleri `DesignerApi`, `TabApi`, `DdlApi` ile güncellendi. `commands.ts`'te kalan komutların her biri ana paketten de çağrılır ya da birden çok tembel parçaca paylaşılır ve ayrı bir ortak parça gerektirir (`ddlTargetOf`, `settingsGet`, `qtCancel`, `qtRun`, `qtMore`, `qtClose`).

**Elle kontrol:** `docs/smoke-test.md` "Nesne arama (Plan 32)" (telafi sonrası Query Tool / designer / bakım / yedekleme / şema karşılaştırma / sunucu diyaloğu gerilemesi dahil).
