# pg-tools — Alt Proje 6: Veri Düzenleyici (View/Edit Data) Tasarımı

- **Tarih:** 2026-10-07
- **Durum:** Plan 16 arka ucu uygulandı; Plan 17 ön yüzü kalan
- **Kapsam:** Object Explorer'dan bir tablonun/görünümün verisini ızgarada açma; hücre düzenleme, satır ekleme ve silme; değişikliklerin **birincil anahtara (ya da benzersiz indekse)** göre `INSERT`/`UPDATE`/`DELETE` ifadelerine çevrilip **kaydetmeden önce gösterilmesi**; korumalı, tek transaction'lı, bayat-veri denetimli uygulama.
- **Önceki alt projeler:** Object Explorer (Plan 1–4), Query Tool (5–6), Editör zekâsı (7), Dashboard (8–9), Nesne diyalogları (10–15) — hepsi `main`'de. Query Tool spec'i §1.4: "Sonuç ızgarasında hücre düzenleme (veri düzenleyici ayrı alt proje)".
- **Teslim planları:** Plan 16 (arka uç: kaynak yükleme, SELECT üretici, değişiklik planlayıcı, kapılı uygulama, Tauri komutları) → Plan 17 (ön yüz: veri sekmesi, düzenlenebilir ızgara, değişiklik paneli, Explorer eylemi).

---

## 1. Amaç ve bağlam

pgAdmin'in "View/Edit Data" özelliği günlük işte en çok kullanılan yazma yoludur: tabloyu aç, birkaç hücreyi düzelt, satır ekle/sil, kaydet. pgAdmin'de bu akış yavaş, bayat veriye karşı korumasız ve neyin çalışacağını göstermeden uygular. Bu alt proje aynı akışı pg-tools'un üç önceliğiyle (açılış hızı, bellek, Windows uyumu) ve nesne diyaloglarındaki **önizle → onayla → uygula** disipliniyle sunar.

### 1.1 Kararlar

| Konu | Karar |
|---|---|
| Satır kimliği | **PK**; yoksa en küçük **geçerli, kısmi olmayan, ifadesiz, tüm kolonları NOT NULL benzersiz indeks**; ikisi de yoksa kaynak **salt okunur** (neden: `no_key`). `ctid`/OID yok (kararsız). Kullanıcının anahtar seçmesi backlog. |
| Düzenlenebilir kaynak | Yalnızca `relkind` `r` (tablo) ve `p` (bölümlü tablo), sistem şeması dışı. View/MatView/Foreign tablo **salt okunur açılır** (Plan 17'de aynı sekme, düzenleme kapalı). |
| Okuma yolu | Mevcut **Query Tool oturumu** (`qt_open`/`qt_run`/`qt_more`/`qt_cancel`): akış, 1000'lik yığınlar, sınır, iptal ve bellek sözleşmesi aynen. Arka uç yalnızca **SQL'i üretir** (`data_select`): istemci SQL yazmaz, kimlik/filtre doğrulanır. |
| Yazma yolu | Ayrı **`data_*` komutları** (Query Tool'dan bağımsız): `data_preview` → `data_apply`. Plan istemciden alınmaz; istemci **değişiklik listesi** gönderir, arka uç ifadeleri üretir (DDL kalıbı). |
| Bayat veri denetimi | Her satırın **`xmin` sürümü** okumayla birlikte gelir (son, gizli kolon); `UPDATE`/`DELETE` `WHERE <anahtar> AND xmin::text = '<sürüm>'` ile gider. **Tam olarak 1 satır** etkilenmezse (0 = başkası değiştirdi/sildi ya da `BEFORE` tetikleyicisi iptal etti) tüm uygulama **geri alınır** (`row_conflict`). Kaynak tanımı da (`fingerprint`: kolonlar + anahtar) önizleme/uygulamada yeniden doğrulanır → `plan_changed`. |
| Atomiklik | Tüm değişiklikler **tek transaction**; biri başarısızsa hepsi geri alınır (`committed` her zaman 0). |
| Değer taşıma | Değerler PostgreSQL **metin biçimidir** (okumada olduğu gibi); `CellValue = Null \| Default \| Text`. İfadelerde yalnızca `quote_literal` ve **tipsiz sabit** kullanılır (PG giriş fonksiyonu kolon tipine çevirir; elle `::tip` yok). Anahtar ve sürüm karşılaştırmaları da aynı biçimde. |
| Risk | `INSERT` ve `UPDATE` **Safe**; `DELETE` **Destructive** (onay kutusu). **Üretim sunucusunda** boş olmayan her plan `confirmed` ister (Query Tool `production_write` eşdeğeri; ad yazma yok). Kilitli sunucu → `locked`. |
| Sınırlar | Bir uygulamada en çok **2000 değişiklik**, değer başına ≤ **16 MiB**, NUL yok. Tek anahtarsız toplu `UPDATE/DELETE` **üretilemez** (her ifade tam anahtar ister). |
| Üretilen/kimlik kolonlar | `GENERATED … STORED/VIRTUAL` kolonlar yazılamaz; `GENERATED ALWAYS` kimlik yalnızca `INSERT`te `DEFAULT` alır (OVERRIDING yok); `BY DEFAULT` kimlik düzenlenebilir. |
| Gizlilik | Hücre değerleri, SQL ve anahtar değerleri `tracing`'e, hata iletisine (PG'nin kendi iletisi hariç, yalnız kullanıcıya gider), diske ve geçmişe **yazılmaz**. |

### 1.2 Başarı ölçütleri

- 1.000.000 satırlı tablo açılışı Query Tool ile aynı bütçede (ilk 1000 satır; kalanı `qt_more`); düzenleme katmanı **yalnızca değişen hücreleri** tutar (satır başına tam kopya yok).
- Ana `index-*.js` **büyümez** (Plan 17: tembel parça; yalnızca sekme türü + eylem kimliği + menü metni ana pakete girebilir ve telafi edilir).
- Bayat satır, silinmiş satır, kısıt ihlali, tetikleyici iptali, kilit zaman aşımı, iptal: hiçbirinde kısmi yazma kalmaz.
- PG 10–18 Docker matrisinde yeşil.

### 1.3 Kapsam dışı (ilk sürüm)

- Anahtarsız tablo düzenleme (`ctid`), kullanıcı seçimli anahtar, updatable view düzenleme.
- `OVERRIDING SYSTEM VALUE`, `COPY`/CSV içe aktarma, toplu yapıştırma sihirbazı, hücre içi büyük nesne (bytea/text) düzenleyici (değerler metin olarak düzenlenir; 1 MiB üstü hücre Plan 17'de salt okunur gösterilir), geri alma/yineleme geçmişi, çok sunuculu karşılaştırma.
- Satır düzeyi güvenlik politikası (RLS) görselleştirmesi (ihlal PG hatası olarak döner).

---

## 2. Mimari

`pgt-ddl` içinde yeni bölüm: `crates/pgt-ddl/src/data/` (apply altyapısı — `CancelMap`, bağlantı hazırlığı, hata eşleme, `Risk/Statement/PlanView/plan_hash`, `quote_*` — ortak olduğu için ayrı crate açılmaz; modül belge başlığı bunu söyler).

```
data/
  model.rs      DataSource, DataColumn, DataKey, ReadOnlyReason, RowChange, CellValue, DataChangeSet, DataApplyOptions, DataOutcome
  source.rs     load_source(mgr, server, db_oid, ObjectRef) → DataSource  (+ sql/data_*.sql, .pg10/.pg12 varyantları)
  select.rs     select_sql(source, SelectRequest) → DataSelect {sql, version_index}
  plan.rs       plan_changes(source, changes) → Result<Plan, Vec<FieldError>>   (saf; insta)
  manager.rs    DataManager: load / select / preview / apply / cancel (DdlManager kalıbı)
```

- **Oturum:** `SessionProfile::Data { lock_timeout_ms }` (Ddl'in kopyası: yazılabilir, `statement_timeout = 0`, başlangıç `lock_timeout`, `application_name = "pg-tools data"`, `standard_conforming_strings = on`). Başka kullanıcının kilitlediği satırda sonsuz beklemez.
- **Kapı sırası (`apply`):** (1) kayıtsız sunucu, (2) `read_only_lock` → `locked`, (3) kaynak yükleme + `fingerprint` eşitliği (`plan_changed`) ve yazılabilirlik (`read_only_source` alan hatası), (4) plan/doğrulama (alan hataları, boş plan), (5) hash eşitliği (`plan_changed`), (6) onay: Destructive ya da üretim sunucusu → `confirmed` yoksa `confirmation_required`, (7) `lock_timeout` sınırı, (8) yürütme.
- **Yürütme:** özel bağlantı, `BEGIN` → her ifade `RETURNING` ile **hazırlanıp** çalıştırılır; etkilenen satır sayısı ≠ 1 → `ROLLBACK` + `row_conflict`; PG hatası → `ROLLBACK` + `Failed{ failedIndex, sqlstate, position, error }`; sonda `COMMIT` (sonuç bilinmiyorsa `unknown_outcome` yükü). Her `INSERT`/`UPDATE` için dönen satır (kolonlar + yeni `xmin`) istemciye taşınır → ızgara yeniden sorgu olmadan tazelenir (sunucu varsayılanları/tetikleyici sonuçları görünür). `DELETE` yalnız sayılır.
- **İptal:** `apply_id` anahtarlı `CancelMap` (DDL ile aynı mantık; `cancel_all` sayfa yenileme/çıkışta).
- **SELECT üretici:** `SELECT <tüm kolonlar, tırnaklı>, xmin::text AS <benzersiz takma ad> FROM <şema>.<ad> [WHERE (<filtre>\n)] ORDER BY <sıralama>`; varsayılan sıralama anahtar kolonları (anahtarsız kaynakta sıralama yok). Filtre serbest SQL'dir ve tek kapıdan geçer: NUL/denetim karakteri yok, ≤ 4096 bayt, `scan_semicolons` temiz (üst düzey `;`/kapanmamış tırnak/yorum/parantez yok). Sıralama kolon adı kaynak kolonlarından seçilir (doğrulanır). Okuma `qt_run`'dan geçtiği için kilit/sınıflandırma kapıları ayrıca işler (SELECT'tir).
- **Anahtar seçimi (`source.rs`):** (1) `contype = 'p'`; (2) benzersiz indeks: `indisunique ∧ indisvalid ∧ indisready ∧ indpred IS NULL ∧ tüm indkey > 0 (ifade yok) ∧ anahtar kolonlarının hepsi attnotnull` — en az kolonlu, eşitlikte ad sırası; (3) yoksa `None`. `INCLUDE` kolonları anahtara girmez (`indnkeyatts`).
- **Yetki:** `has_table_privilege`/`has_column_privilege` (`SELECT/INSERT/UPDATE/DELETE`) ile kolon bazında `insertable`/`updatable`; tablo bazında `canInsert/canUpdate/canDelete`. Yetkisizse alan salt okunur; yine de PG 42501 dönerse mevcut hata yolu.
- **Sistem şemaları** (`pg_catalog`, `information_schema`, `pg_toast`) salt okunur (`system_object`).

### 2.1 Komutlar (Tauri; hepsi `Box::pin`)

| Komut | Dönüş |
|---|---|
| `data_load(serverId, databaseOid, object)` | `DataSource` |
| `data_select(serverId, databaseOid, object, fingerprint, request)` | `DataSelect` (SQL + sürüm kolonu indeksi) |
| `data_preview(serverId, databaseOid, changeSet)` | `PlanView` |
| `data_apply(serverId, databaseOid, changeSet, options)` | `DataOutcome` |
| `data_cancel(applyId)` | — |

Hedef çözümü için mevcut `ddl_target_of` (nesne düğümü → `ObjectRef` + `databaseOid`) kullanılır; yeni hedef komutu yok.

---

## 3. Arayüz (Plan 17 özeti)

- **Giriş:** Explorer'da table/view/materializedView/partition düğümünde **"Verileri Görüntüle/Düzenle"** (menü üstü grup, "Sorgu Aracı"nın yanında); tembel parça; yeni sekme türü `data` (aynı nesne için tek sekme).
- **Izgara:** mevcut `ResultGrid`/`grid-model` yeniden kullanılır; düzenleme **üst katman modelidir** (`dirty: Map<satırAnahtarı, Map<kolon, CellValue>>`, yeni satırlar, silinecekler); `StmtResult` değiştirilmez. Çift tık/F2/Enter düzenler, Esc vazgeçer, `Delete` satırı silinecek işaretler, boş satıra yazmak yeni satır ekler; NULL ve DEFAULT için açık komutlar (kısayol + bağlam menüsü). Değişen hücre/satır zemin rengiyle işaretlenir.
- **Değişiklik paneli:** altta/sağda **"N değişiklik"**; `data_preview` ile üretilen ifadeler (Plan 11 `SqlPreview` kalıbı; değerler uzunsa kısaltılarak gösterilir), risk rozetleri, **Kaydet** (onay kutusu kuralları Plan 11 `applyGate` mantığıyla), **Vazgeç**. Çakışma (`row_conflict`) ilgili satırı vurgular ve "Yeniden yükle" önerir; kayıtlı olmayan değişiklikle sekme kapatılırsa onay sorar.
- **Salt okunur durumlar** gerekçeyle (şerit): anahtar yok, yetki yok, kilitli sunucu, görünüm/MV/yabancı tablo, sistem nesnesi.
- **Sıralama/filtre:** kolon başlığı tıklaması → `data_select` ile yeniden sorgu; filtre kutusu (serbest `WHERE` gövdesi, doğrulama hatası alan hatası olarak).

---

## 4. Güvenlik ve gizlilik

- İstemci SQL yazmaz (filtre dışında; o da sunucu tarafında doğrulanıp sarmalanır); tüm tanımlayıcılar `quote_ident`, tüm değerler `quote_literal`.
- Hücre değerleri ve anahtarlar günlüklere, `ErrorPayload`'a (kendi ürettiklerimiz), geçmişe ve diske yazılmaz; yalnızca ifade/satır sayısı ve süre günlüklenir.
- Kilit bir güvenlik sınırı değil, korumadır (Query Tool ile aynı); üretim sunucusunda her yazma onay ister.
- Test sunucu URL'leri/parolaları yazdırılmaz (`PGT_TEST_URLS` yalnızca test komutunun ortamında).

---

## 5. Test stratejisi

- **Saf (insta):** plan üretici (INSERT/UPDATE/DELETE, DEFAULT/NULL, anahtar kolon güncelleme, kimlik/üretilmiş kolon reddi, hostile değerler: `'`, `\`, `--`, `;`, Unicode, NUL, 16 MiB sınırı), SELECT üretici (takma ad çakışması, sıralama, filtre kapısı), anahtar seçimi.
- **Docker (PG 10–18, `scripts/test-matrix.ps1`):** kaynak yükleme (PK/benzersiz indeks/anahtarsız/bölümlü/üretilmiş/kimlik/yetkisiz rol), gidiş-dönüş (insert → returned == okunan), bayat `xmin` → `row_conflict` + hiçbir şey yazılmadı, silinmiş satır, tek ifadede kısıt ihlali → tam geri alma, tetikleyici `RETURN NULL` → conflict, kilit zaman aşımı (55P03), iptal, üretim/kilit kapıları, `float`/`timestamptz`/`bytea`/`json`/dizi/`NULL` ≠ `''` gidiş-dönüş.
- **Bellek/ölçüm:** düzenleme katmanı satır sayısından bağımsız (yalnız dirty); `docs/performance.md` Plan 17'de.

---

## 6. Planlama notu

- **Plan 16 (arka uç):** model + hata kodları + `SessionProfile::Data` → kaynak yükleyici → SELECT üretici → plan üretici → yönetici/uygulama → Tauri komutları → Docker matrisi/belgeler.
- **Plan 17 (ön yüz):** sekme türü ve Explorer eylemi (ana paket telafisiyle) → düzenleme modeli (saf) → ızgara düzenleme katmanı → değişiklik paneli/uygulama akışı → çakışma/hata durumları → smoke listesi.

---

## 7. Plan 16 sözleşme notları (Plan 17 için)

Teslim edilen arka uç davranışı (kodla doğrulanmış); Plan 17 bunlara göre yazılır.

**Kaynak ve salt okunurluk.** `DataSource` (`data_load`) kolonları (`insertable`/`updatable`/`generated`/`identity`), seçilen anahtarı (`DataKey.columns` sırası belirleyicidir), yetkileri ve `fingerprint`'i taşır. `editable` yazmaya izin verilip verilmediğidir; `false` ise `readOnlyReason` gerekçedir (anahtar yok, yetki yok, görünüm/MV/yabancı tablo, sistem nesnesi). `locked` sunucu kilidinin durumunu yansıtır; kilitli sunucuda yazma `locked` hatasıyla reddedilir (kilit koruma, güvenlik sınırı değil). `fingerprint` süreç içidir (`DefaultHasher`, derlemeler arasında kararlı değil): asla kalıcılaştırılmaz.

**Okuma.** `data_select` SELECT SQL'ini elde etmenin tek yoludur (istemci SQL üretmez). `DataSelect.versionIndex` sürüm kolonunun indeksidir ve sürüm kolonu **son kolondur**; `hasVersion` view/matview/foreign tabloda `false`'tur (sürüm kolonu yok). Sürüm kolonu `xmin::pg_catalog.text`'tir (search_path'e dayanıklılık için her yerde nitelikli). Filtre/sıralama hataları `DataSelectResult::Invalid` olarak döner (alan hatası olarak gösterilir). Filtrede `\` için `E'…'` gerekir.

**Değişiklik anahtarı ve sürüm.** `RowChange` anahtarı `DataKey.columns` sırasıyla ve **özgün okunan** (düzenlemeden önceki) değerlerle verilir. `version` = okunan `xmin` metnidir (Update/Delete). Çakışma (`row_conflict`) sonrasında arayüz satırı/ızgarayı **yeniden yükler**; sessizce yeniden denemez. Aynı değerle eşzamanlı yazma da çakışma sayılır (xmin değişir).

**Uygulama sonucu.** `DataOutcome.results[i].returned` yazılan satırın RETURNING değerleridir (kolonlar + **son eleman `xmin`**); arayüz ızgarayı bununla tazeler, yeniden sorgulamaz. İstisna: **bölümlü ana tabloya INSERT** PG 12–18'de `RETURNING xmin` veremez (0A000); bu durumda sürüm yalnızca bu durum için `(txid_current() % 4294967296)::text` olur (başarısızlıkta güvenli tarafta kalır: sonraki UPDATE çakışma verir ve yeniden yükleme ister). `CellValue::Default` UPDATE'te de serbesttir (`SET c = DEFAULT`), üretilmiş/ALWAYS-kimlik/güncellenemez kolonlar hariç.

**Önizleme.** `PlanView.statements[].sql` **değer içerir**; arayüz uzun değerleri kısaltarak göstermelidir. Önizleme değerleri olduğu gibi çizer: bidi ve denetim karakterleri **görünür** gösterilmelidir (Plan 17). Önizleme hücre değerlerini günlüğe yazmaz.

**Hata kodları.** `locked`, `confirmation_required`, `plan_changed`, `invalid_def`, `row_conflict`; ayrıca PG hataları olduğu gibi geçer (42501 için özel ipucu eklenmedi). `invalid_def` yol alanları: `changes`, `changes[i].key`, `changes[i]…` ve `order` (8'den fazla sıralama anahtarı); Plan 17 bunları eşlemelidir. Salt okunur kaynakta boş değişiklik listesi `object` hatası verir. SQLSTATE'siz COMMIT hatası `unknown_outcome` olarak bildirilir (sonuç bilinmiyor; kullanıcı yeniden yüklemelidir).

**Yaşam döngüsü.** `data_cancel` → `DataManager::cancel` **async** ve `Result` döner (komut await eder). Üretimde Safe plan bile `confirmed` ister; `expectedHash` plan değişirse `plan_changed`.

**Bilinen sınırlar.** Float anahtar + PG 10–11'de `extra_float_digits` (anahtar metni yuvarlanabilir); yalnız kolon düzeyi UPDATE yetkisi `NoPrivilege` sayılır (tablo düzeyi `has_table_privilege`); PG 18 `NOT NULL NOT VALID` sınanmadı (satır sayısı korumasıyla güvenli tarafta); `unknown_outcome` yolu Docker'da sınanmadı; src-tauri testkit `fixture()` yalnız ilk `PGT_TEST_URLS` adresini kullanır.

**Ana paket bütçesi.** Ana `index-*.js` tavanı **232,805 B**. Veri sekmesi kodu tembel parçadadır; ana pakete yalnızca sekme türü, eylem kimliği ve menü metni girer; her bayt başka yerden telafi edilir (Plan 15'teki `strings-editor.ts` kalıbı).

**Backlog.** Anahtarsız düzenleme, kullanıcı seçimli anahtar, güncellenebilir view, `OVERRIDING` (kimlik), büyük nesne düzenleyici.
