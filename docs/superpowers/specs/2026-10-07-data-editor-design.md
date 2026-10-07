# pg-tools — Alt Proje 6: Veri Düzenleyici (View/Edit Data) Tasarımı

- **Tarih:** 2026-10-07
- **Durum:** Plan 16–17 uygulandı (`feat/data-editor-ui`; ön yüz planı `docs/superpowers/plans/2026-10-07-data-editor-p17-on-yuz.md`); elle UI kontrolü (`docs/smoke-test.md` Plan 17 bölümü, `[—]`) bekliyor
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
| Okuma yolu | Mevcut **Query Tool oturumu** (`qt_open`/`qt_run`/`qt_more`/`qt_cancel`; sekmeye özel, salt okunur — §7 "Okuma oturumu sözleşmesi"): akış, 1000'lik yığınlar, sınır, iptal ve bellek sözleşmesi aynen. Arka uç yalnızca **SQL'i üretir** (`data_select`): istemci SQL yazmaz, kimlik/filtre doğrulanır. |
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
- **Yürütme:** özel bağlantı, `BEGIN` → her ifade `RETURNING` ile **metin protokolüyle** (`simple_query`; hazırlanmış ifade/parametre yok, değerler `quote_literal`'li sabittir) tek tek çalıştırılır; etkilenen satır sayısı ≠ 1 → `ROLLBACK` + `row_conflict`; PG hatası → `ROLLBACK` + `Failed{ failedIndex, sqlstate, position, error, conflict, outcomeUnknown }`; iptal bayrağı her ifadeden ve `COMMIT`'ten önce denetlenir; sonda `COMMIT` (SQLSTATE'siz `COMMIT` hatasında `outcomeUnknown = true` + belirsizlik ipucu). Her `INSERT`/`UPDATE` için dönen satır (kolonlar + yeni `xmin`) istemciye taşınır → ızgara yeniden sorgu olmadan tazelenir (sunucu varsayılanları/tetikleyici sonuçları görünür). `DELETE` yalnız sayılır.
- **İptal:** `apply_id` anahtarlı `CancelMap` (DDL ile aynı mantık; `cancel_all` sayfa yenileme/çıkışta).
- **SELECT üretici:** `SELECT <tüm kolonlar, tırnaklı>, xmin::pg_catalog.text AS <benzersiz takma ad> FROM [ONLY] <şema>.<ad> [WHERE (<filtre>\n)] ORDER BY <sıralama>` (`ONLY` yalnızca sıradan tabloda; bkz. §7 "Kalıtım"); varsayılan sıralama anahtar kolonları (anahtarsız kaynakta sıralama yok). Filtre serbest SQL'dir ve tek kapıdan geçer: NUL/denetim karakteri yok, ≤ 4096 bayt, `scan_semicolons` temiz (üst düzey `;`/kapanmamış tırnak/yorum/parantez yok). Sıralama kolon adı kaynak kolonlarından seçilir (doğrulanır). Okuma `qt_run`'dan geçtiği için kilit/sınıflandırma kapıları ayrıca işler (SELECT'tir).
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

### Plan 17 notları

- **Okuma oturumu:** veri sekmesi `qt_open_readonly` ile açılan ayrı, salt okunur bir oturumdan okur (`SessionOptions` değişmez). Kilitli sunucuda salt okunurluk zorlanır; **kilitsiz sunucuda yalnızca oturum varsayılanıdır** (`default_transaction_read_only`) ve kullanıcı filtresinde `set_config` bunu çevirebilir — kullanıcının kendi gücü; sekme yalnızca `data_select` SQL'i çalıştırır. Yazma `data_apply` ile ayrı, kapılı yolla gider.
- **Kendi ızgara bileşeni (Karar 2):** `ResultGrid`/`grid-model` içe aktarılmaz, gerekli mantık `data-grid-model.ts` ve `DataGrid.tsx`'e **kopyalanmıştır**. Gerekçe: `StmtResult` değişmez kalır; ana paket tavanı sıfır pay (232.343 B, mapDeps 21) olduğundan veri parçası yalnızca kendi tembel zincirine bağlıdır (`designer/*`, `ResultGrid`, `src/ui/*`, lucide `Icon` içe aktarılmaz; yerel minimal iletişim kutusu/düğmeler). Ortak ızgara backlog'dadır.
- **`toChangeSet` kuralları (`data-edit-model.ts`):** her satır için tek `RowChange`; var olan satırda yalnız dokunulan kolonlar UPDATE; `''` (boş metin), NULL ve DEFAULT üç ayrı değerdir; yeni satırda yalnız dokunulan kolonlar INSERT'e girer; yeni satır silinirse hiç yazılmaz; işaretli satır DELETE (düzenlemesi atılır); anahtar kolonu değişirse eski anahtar + sürümle UPDATE; üretilmiş/ALWAYS kimlik kolonları ve `notInsertable` satırlar düzenlenemez; `changeCount(l,e)` = `toChangeSet` uzunluğu.
- **Kısayollar:** çift tık/F2/Enter düzenle; Enter/Tab onay, Esc vazgeç (çok satırlı editörde Ctrl+Enter); `Alt+N` NULL, `Alt+D` DEFAULT; `Alt+S` / `Alt+Shift+S` sıralama (başlıklar odaklanamaz; tıklama / Shift+tıklama); `Ctrl+Z` hücre geri alma; `Ctrl+C` kopyala; `Insert` yeni satır; `Delete` sil işareti/geri al; oklar/Home/End/PageUp/PageDown gezinme; AltGr (Ctrl+Alt) kombinasyonları kullanılmaz; IME birleştirmesi sürerken Enter/Esc etkisiz; blur (pencere odağı dahil) editörü onaylar.
- **Sınırlar:** sekme başına en çok 50.000 satır yüklenir (`ROW_CAP`; INSERT'ler sonrası satır sayısı bunu aşabilir, yalnızca yükleme durur); önizleme ifadeleri 4000 karakterde kısaltılır ("Kopyala" tamını verir; bidi/denetim karakterleri `⟦U+202E⟧` olarak görünür); önizleme 150 ms debounce; kilit zaman aşımı varsayılanı 5000 ms; >1 MiB hücre için salt okunur gösterim uygulanmaz (§1.3 notu, Karar 10 gereği backlog).
- **Davranış kararları:** çakışma (`row_conflict`) satırı vurgular, düzenlemeleri korur, `needsReload` kurmaz; `outcomeUnknown` yalnızca yeniden yükleme sunar; uygulama ile "Daha fazla" karşılıklı dışlanır; başarısız sıralama/filtre yüklemesi satır ve düzenlemeleri temizler, bağlantı kaybı ise satırları korur ve seçimi geri yükler; kirli sekmede F5/kapatma/sıralama/filtre onay sorar (veri onayı `role="alertdialog"`).
- **Ana paket telafisi:** sıfır pay için `t.query.history` → `src/features/query/strings-history.ts` (`thist`); `t.query.messages/check/format/confirm` → `strings-querytab.ts` (`tqt`). Denetleyici + açıcı `DataTabs-*.js` içindedir; `data-open.ts` bağımlılıksız cephedir (statik içe aktarma +52 B / 22 mapDeps girdisi verirdi). Son ölçüm: `index-*.js` 232.343 B, `DataTabs-*.js` 68.672 B, `data-open-*.js` 1.281 B.
- **Backlog:** ortak ızgara bileşeni (`ResultGrid` ile); yapıştırma; büyük nesne (>1 MiB, bytea/json) düzenleyici; kullanıcı seçimli anahtar (benzersiz indeks seçimi); kolon gizleme/sıralama; `QueryTabs` F5 korumasının `role="alertdialog"`'u da denetlemesi (ana dosyada birkaç bayt; şimdi veri onayı açıkken F5 `askReload`'u yeniden kuyruğa alır); `isControl` aralığı genişletme (U+2060–206F, U+00AD/034F/180E, etiket karakterleri, varyasyon seçiciler) `data-apply-model.ts`'te; önizlemede "özgün kod birimi" ifadesinin sadeleştirilmesi; `open_read_only` doc yorumuna `set_config` notu; atlanan nötr karakterler için `…(+N)` etiketi; kolon başlığı odaklanabilirliği.

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

**Okuma.** `data_select` SELECT SQL'ini elde etmenin tek yoludur (istemci SQL üretmez). `DataSelect.versionIndex` sürüm kolonunun indeksidir ve sürüm kolonu **son kolondur**; `hasVersion` view/matview/foreign tabloda `false`'tur (sürüm kolonu yok). Sürüm kolonu `xmin::pg_catalog.text`'tir (search_path'e dayanıklılık için her yerde nitelikli). Filtre ve sıralama hataları (`filter`, `order` — 8'den fazla sıralama anahtarı —, `order[i].column`) **`data_select`'in `Ok(DataSelectResult::Invalid { errors })`** sonucunda döner (komut hatası ya da `invalid_def` değildir; alan hatası olarak gösterilir). Filtrede `\` için `E'…'` gerekir.

**Okuma oturumu sözleşmesi.** Her veri sekmesi okumayı **kendi** Query Tool oturumundan yapar: sekmeye özel, `read_only: true` ile açılan oturum (`SessionProfile::Query { read_only: true, … }` → `default_transaction_read_only = on`) — sunucu kilitli olmasa bile salt okunur. Not: bugünkü `qt_open`/`SessionOptions` bu bayrağı istemciden almaz (yalnızca sunucu kilidi salt okunur oturum açar); seçeneği eklemek **Plan 17'nin işidir** —, autocommit (açık transaction yok) ve bu oturumda **asla kullanıcı SQL'i** çalışmaz (yalnızca `data_select` çıktısı; Query Tool sekmesiyle paylaşılmaz). Gerekçe: (1) salt okunur oturum, filtreye gömülen `FOR UPDATE`/`nextval`/yazan fonksiyonları reddeder; (2) okuyucu satır kilidi tutmaz — aksi hâlde `lock_timeout = 0` ile uygulama kendi okuyucusunun kilidini sonsuza dek bekleyebilirdi. Uygulamanın varsayılan kilit zaman aşımı sıfırdan büyüktür (ör. **5000 ms**); `0` (sınırsız) yalnızca kullanıcının açık seçimidir. Bilinen sınır: filtre `set_config('DateStyle' | 'IntervalStyle' | 'extra_float_digits', …)` ile okuma oturumunun çıktı biçimini değiştirebilir; anahtar metni farklı biçimlenirse uygulamadaki tipsiz sabit **başka bir satıra** çözümlenebilir (aynı `xmin`'e sahip satır varsa — ör. aynı transaction'da eklenmiş satırlar — yazma o satıra gider). Uygulama oturumu bu ayarları sabitler; okumadaki sapmayı engellemez.

**Kalıtım.** Sıradan tablo (`relkind = 'r'`) **`ONLY`** ile okunur ve yazılır: `SELECT … FROM ONLY`, `UPDATE ONLY`, `DELETE FROM ONLY` (INSERT'te `ONLY` yoktur). Üst tablonun sekmesi çocuk tabloların satırlarını **göstermez** ve onları düzenleyemez (üst tablo üzerinden çocuğun anahtar/sürümüyle yazma 0 satır → `row_conflict`); çocuk satırları çocuk tablonun **kendi** sekmesinden (kendi `ObjectRef`/düğümü) düzenlenir. Bölümlü tabloda (`relkind = 'p'`) `ONLY` **hiç** kullanılmaz: bölümlerdeki satırlar üst tablonun sekmesinde görünür ve oradan düzenlenir. Üst tabloya dayanan görünümler (`SELECT * FROM parent`) çocuk satırlarını içerir; görünüm ile üst tablo sekmesi bu yüzden farklı satır sayısı gösterebilir.

**Değişiklik anahtarı ve sürüm.** `RowChange` anahtarı `DataKey.columns` sırasıyla ve **özgün okunan** (düzenlemeden önceki) değerlerle verilir. `version` = okunan `xmin` metnidir (Update/Delete). Çakışma (`row_conflict`) sonrasında arayüz satırı/ızgarayı **yeniden yükler**; sessizce yeniden denemez. Aynı değerle eşzamanlı yazma da çakışma sayılır (xmin değişir).

**Uygulama sonucu.** `DataOutcome.results[i].returned` yazılan satırın RETURNING değerleridir (kolonlar + **son eleman `xmin`**); arayüz ızgarayı bununla tazeler, yeniden sorgulamaz. İstisna: **bölümlü ana tabloya INSERT** PG 12–18'de `RETURNING xmin` veremez (0A000); bu durumda sürüm yalnızca bu durum için `(pg_catalog.txid_current() OPERATOR(pg_catalog.%) 4294967296)::pg_catalog.text` olur (fonksiyon, işleç ve tip nitelikli; başarısızlıkta güvenli tarafta kalır: sonraki UPDATE çakışma verir ve yeniden yükleme ister). `DataOutcome::Failed.outcomeUnknown = true` yalnızca `COMMIT` SQLSTATE'siz başarısız olduğunda gelir ve **"kalıcılaşmış olabilir; yeniden denemeden önce yeniden yükle"** demektir (arayüz otomatik yeniden denemez). Diğer her `Failed`'da (çakışma, iptal, ifade hatası, ertelenmiş kısıt) `outcomeUnknown = false` ve hiçbir şey yazılmamıştır. `CellValue::Default` UPDATE'te de serbesttir (`SET c = DEFAULT`), üretilmiş/ALWAYS-kimlik/güncellenemez kolonlar hariç.

**Önizleme.** `PlanView.statements[].sql` **değer içerir**; arayüz uzun değerleri kısaltarak göstermelidir. Önizleme değerleri olduğu gibi çizer: bidi ve denetim karakterleri **görünür** gösterilmelidir (Plan 17). Önizleme hücre değerlerini günlüğe yazmaz.

**Hata kodları.** Komut hatası (`Err`) kodları: `locked`, `confirmation_required`, `plan_changed`, `invalid_def`; ayrıca PG/bağlantı hataları olduğu gibi geçer (42501 için özel ipucu eklenmedi). `data_apply`'ın `invalid_def` hatası **alan yolu taşımaz**; alan hataları `data_preview`'in `PlanView.errors`'ında gelir (yollar: `object`, `changes`, `changes[i]`, `changes[i].key`, `changes[i].key[j]`, `changes[i].version`, `changes[i].values[j]`, `changes[i].set[j]`; Plan 17 bunları eşlemelidir). Salt okunur kaynakta boş değişiklik listesi `object` hatası verir. `row_conflict` **hiçbir zaman komut `Err`'i olarak gelmez**: `Ok(DataOutcome::Failed { conflict: true, error.code: "row_conflict", sqlstate: null })` sonucudur. Çakışmanın anlamı ifade türüne göre değişir: UPDATE/DELETE'te satır bayat/silinmiş, `BEFORE` tetikleyicisi `RETURN NULL` verdi ya da RLS politikası satırı süzdü; **INSERT** sırasındaki çakışma ise satırın bir tetikleyici/kural/RLS tarafından **düşürüldüğü** demektir (ileti "satır değişmiş" der; arayüz INSERT için bu anlamı göstermelidir). SQLSTATE'siz `COMMIT` hatası `outcomeUnknown = true` ile bildirilir (sonuç bilinmiyor; kullanıcı yeniden yüklemelidir).

**Yaşam döngüsü.** `data_cancel` → `DataManager::cancel` **async** ve `Result` döner (komut await eder). Üretimde Safe plan bile `confirmed` ister; `expectedHash` plan değişirse `plan_changed`.

**Bilinen sınırlar.** Bayat veri denetimi **kapalı başarısız olur** (anahtar + `xmin` uyuşmazsa 0 satır → `row_conflict`); tek istisna, kayıplı metin gidiş-dönüşlü anahtarda **aynı `xmin`'e sahip başka bir satırdır**: `extra_float_digits < 1` iken float anahtar (PG 10–11 varsayılanı ya da filtredeki `set_config`), ISO olmayan `DateStyle`/`IntervalStyle` ile tarih/aralık anahtarı — anahtar metni başka bir satıra çözümlenir ve o satır aynı transaction'da yazılmışsa (aynı `xmin`) yazma o satıra gider. Float anahtar + PG 10–11'de `extra_float_digits` (anahtar metni yuvarlanabilir); yalnız kolon düzeyi UPDATE yetkisi `NoPrivilege` sayılır (tablo düzeyi `has_table_privilege`); PG 18 `NOT NULL NOT VALID` sınanmadı (satır sayısı korumasıyla güvenli tarafta); `unknown_outcome` yolu Docker'da sınanmadı; src-tauri testkit `fixture()` yalnız ilk `PGT_TEST_URLS` adresini kullanır.

**Ana paket bütçesi.** Ana `index-*.js` tavanı **232,805 B**. Veri sekmesi kodu tembel parçadadır; ana pakete yalnızca sekme türü, eylem kimliği ve menü metni girer; her bayt başka yerden telafi edilir (Plan 15'teki `strings-editor.ts` kalıbı).

**Backlog.** Anahtarsız düzenleme, kullanıcı seçimli anahtar, güncellenebilir view, `OVERRIDING` (kimlik), büyük nesne düzenleyici. Ayrıca (Plan 16 son incelemesi):
- `DataSource`/`DataKey`/`DataColumn`/`OrderBy`/`DataSelect`/`DataSelectResult` türetilmiş `Debug` kullanır ve nesne/kolon adlarını ya da filtre metnini (`DataSelect.sql`) yazdırır; bugün bunları test paniği dışında yazdıran yer yoktur — elle `Debug`'a geçirilmeli.
- `hasVersion`, salt okunur bölümlü/kalıtımlı kaynaklarda da `true`'dur (yalnızca `relkind`'e bakar); `has_version = editable` düşünülmeli.
- `LOCK_TIMEOUT_MESSAGE` "Nesne kilidi" der; veri düzenleyicide bekleme satır kilididir (ileti genelleştirilmeli).
- `DataManager::select` sıralama hatalarını da günlükte `invalid_filter` sonucu olarak yazar (ayrı `invalid_order` kodu).
