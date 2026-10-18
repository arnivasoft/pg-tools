# pg-tools — Bakım İşlemleri Tasarımı (VACUUM / ANALYZE / REINDEX / CLUSTER)

- **Tarih:** 2026-10-18
- **Durum:** Plan 24 arka ucu tamamlandı (Docker PG 10–18 yeşil); Plan 25 ön yüzü uygulandı; elle kontrol bekliyor (`docs/smoke-test.md` "Bakım işlemleri arayüzü (Plan 25)"). Sözleşme: §6; uygulama notları: §7.
- **Kapsam:** Explorer'dan bir nesne için bakım işlemi çalıştırma: seçenekler, SQL önizlemesi, kilit uyarısı, onay, uygulama, iptal, ilerleme (`pg_stat_progress_*`) ve `VERBOSE` iletileri.
- **Temel:** `pgt-ddl` çerçevesi (`PlanRequest`, `DdlManager::apply` kapıları, `run_statements`, `CancelMap`) ve object-dialogs spec'inin risk modeli (§3).
- **Teslim:** Plan 24 (arka uç), Plan 25 (ön yüz: "Bakım…" diyaloğu).

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| İşlemler ve hedefler | **VACUUM**: tablo (sıradan/bölümlenmiş/bölüm), MV, veritabanı (tümü). **ANALYZE**: tablo, MV, veritabanı. **REINDEX**: indeks (`INDEX`), tablo/MV (`TABLE`), şema (`SCHEMA`), veritabanı (`DATABASE` — PG gereği yalnız bağlı veritabanı, adla). **CLUSTER**: tablo (sıradan; bölümlenmiş PG 15+), isteğe bağlı `USING <indeks>`. Desteklenmeyen birleşim → doğrulama hatası. Tablo dışı nesneler (view, sequence, foreign table…) için işlem yok. |
| Seçenekler | VACUUM: `FULL`, `FREEZE`, `ANALYZE`, `VERBOSE`, `DISABLE_PAGE_SKIPPING`, `SKIP_LOCKED` (PG 12+), `INDEX_CLEANUP {AUTO (PG 14+), ON, OFF}` (PG 12+), `PARALLEL n` (PG 13+, 0–1024; `FULL` ile birlikte değil). ANALYZE: `VERBOSE`, `SKIP_LOCKED` (PG 12+). REINDEX: `CONCURRENTLY` (PG 12+; sistem kataloğunda değil), `VERBOSE`. CLUSTER: `VERBOSE`, `USING` indeksi (tablonun indeksi olmalı; yoksa ve tabloda kümeleme indeksi yoksa hata). Sürüm kapıları doğrulama hatasıdır (sessiz atlama yok). |
| Sözdizimi | VACUUM/ANALYZE: parantezli seçenek listesi, değersiz boolean anahtar sözcükleri (PG 10–11 uyumu): `VACUUM (FULL, ANALYZE, VERBOSE) s.t`; seçenek yoksa parantez yazılmaz. İstisna: PG 10'da `ANALYZE` parantezli listeyi desteklemez → `ANALYZE VERBOSE [s.t]` (PG 10'da `SKIP_LOCKED` zaten kapılı). REINDEX: seçenekler hedef sözcüğünden önce, `CONCURRENTLY` sonra — `REINDEX (VERBOSE) TABLE CONCURRENTLY s.t` (CONCURRENTLY PG 12+; bu biçim PG 10–18'de geçerli, PG 14+'ın `REINDEX (CONCURRENTLY) …` biçimi kullanılmaz); PG 10–11'de `REINDEX (VERBOSE) TABLE s.t`. CLUSTER: parantezli `CLUSTER (VERBOSE) …` biçimi PG 14+; tüm sürümlerde geçerli parantezsiz biçim kullanılır — `CLUSTER [VERBOSE] s.t [USING i]`. Tanımlayıcılar yalnız `quote_ident`/`qualified`. |
| Çalıştırma | Tek ifade, **transaction dışı** (`transactional = false`; VACUUM/REINDEX CONCURRENTLY/SCHEMA/DATABASE zorunlu, diğerleri tutarlılık için). `statement_timeout = 0` (DDL oturumu); `lock_timeout` kullanıcı seçimi (ApplyOptions). |
| Risk | **Destructive** (onay ister): özel kilit alan işlemler — `VACUUM FULL`, `CLUSTER`, `REINDEX` (CONCURRENTLY olmadan), `REINDEX SCHEMA/DATABASE` (CONCURRENTLY olmadan). Not: `VACUUM FULL`/`CLUSTER` için "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler." (veritabanı genelinde VACUUM FULL: + " Tablolar sırayla kilitlenir.") + " Tablo boyutu kadar ek boş disk alanı gerekir."; REINDEX (CONCURRENTLY'siz) için "Tabloda SHARE, indekste ACCESS EXCLUSIVE kilit alır; yazma ve indeksi kullanan okumalar bekler." (SCHEMA/DATABASE: sonuna " Tablolar sırayla kilitlenir.") **Safe**: `VACUUM` (FULL'suz), `ANALYZE`, `REINDEX … CONCURRENTLY` (not: "Başarısız/iptal edilirse geçersiz bir *_ccnew indeksi kalabilir."). DataLoss yok (üretimde ad yazma yok). Kilitli sunucuda (salt okunur kilit) tüm bakım işlemleri reddedilir. |
| Nesne denetimi | İstek, yüklenen `MaintenanceTarget { kind, schema, name, relkind, partitioned, system, has_cluster_index, indexes }`'i taşır; uygulamada yeniden yüklenir ve eşitlik aranır (`PLAN_CHANGED`), sistem şeması nesnesinde `VACUUM`/`ANALYZE` ve düz (CONCURRENTLY'siz) `REINDEX` serbest; `REINDEX CONCURRENTLY` ve `CLUSTER` reddedilir (PG kuralı). Veritabanı hedefi bağlı (hedef) veritabanıdır. |
| İletiler | Uygulama bağlantısının NOTICE/INFO iletileri (`VERBOSE` çıktısı, uyarılar) toplanır ve `ApplyOutcome`'a eklenir (`messages`, yoksa alan yazılmaz): en çok 500 ileti, ileti başına 4 KiB (kesilir; kesildiği belirtilir). İletiler yalnız kullanıcıya döner; `tracing`'e yazılmaz. |
| İlerleme | Uygulama bağlantısının arka uç pid'i kaydedilir; `maintenance_progress(apply_id)` komutu katalog havuzundan `pg_stat_progress_vacuum` (9.6+), `pg_stat_progress_cluster` (12+; VACUUM FULL ve CLUSTER), `pg_stat_progress_create_index` (12+; REINDEX), `pg_stat_progress_analyze` (13+) okur → `{ phase, done, total }` (bilinmiyorsa `None`). Sürümde görünüm yoksa ya da satır yoksa `None` (belirsiz ilerleme). Ön yüz 1 sn aralıkla sorar. |
| Kapsam dışı | `VACUUM` çoklu tablo listesi, kolon listesi (`ANALYZE t(c)`), `REINDEX SYSTEM`, `TABLESPACE` (REINDEX PG 14+), `vacuumdb`/`reindexdb` paralel iş, zamanlama, `pg_repack`. |

## 2. Model (özet)

- `MaintenanceOp` = `Vacuum(VacuumOptions)` | `Analyze(AnalyzeOptions)` | `Reindex(ReindexOptions)` | `Cluster(ClusterOptions)` (serde `tag = "op"`).
- `MaintenanceTarget` (yükleyici; salt okunur kimlik): `kind` (`table` | `materialized_view` | `index` | `schema` | `database`), `schema`, `name`, `table` (indeks için ait tablo), `partitioned`, `system`, `clusterIndex: Option<String>`, `indexes: Vec<String>` (tablo için, CLUSTER USING seçimi), `editable`, `unsupported`.
- `PlanRequest::Maintenance { object: ObjectRef, target: MaintenanceTarget, operation: MaintenanceOp }`; JSON `{op:"maintenance", object, target, operation}` (§6.1).
- `ApplyOutcome::{Applied, Failed}` + `messages?: Vec<ServerMessage { severity, code, message, detail, hint }>`.
- `MaintenanceProgress { phase: String, done: Option<u64>, total: Option<u64>, unit: "blocks" | "tuples" | "indexes" | null }`.

## 3. Arayüz (Plan 25 özeti)

- Explorer: tablo, MV, indeks, şema, veritabanı düğümlerinde **"Bakım…"** (kilitli sunucuda devre dışı).
- Tembel diyalog: işlem seçimi (hedefe ve sürüme göre), seçenekler, SQL önizlemesi, risk + kilit notu, lock timeout, onay kutusu (Destructive), Uygula / İptal; çalışırken ilerleme çubuğu (belirsiz/yüzde, aşama adı), geçen süre; bitince sonuç + iletiler listesi (`VERBOSE`). Diyalog çalışırken kapatılırsa iptal onayı.

## 4. Güvenlik ve gizlilik

- İfade metni, nesne adları ve iletiler `tracing`'e yazılmaz (yalnız `op`, `kind`, süre, sonuç kodu).
- İlerleme sorgusu yalnız kendi uygulama pid'imiz için (başka oturumların ilerlemesi gösterilmez).

## 5. Test stratejisi

- Saf üretici: insta (her işlem × hedef × seçenek × sürüm), sürüm kapıları, risk sınıfları, `transactional = false`, düşmanca adlar.
- Docker PG 10–18: her işlemin gerçekten çalışması (`pg_stat_user_tables.last_vacuum/last_analyze`, `pg_index.indisclustered`, yeni indeks relfilenode), `VERBOSE` iletilerinin `messages`'a düşmesi, iptal (uzun VACUUM FULL ya da `pg_sleep` kilit beklemesiyle 57014), lock_timeout (55P03), ilerleme sorgusunun çalışırken satır döndürmesi (en iyi çaba; zamanlamaya bağlı test `None`'u da kabul eder), kilitli sunucu reddi, bayat hedef (`PLAN_CHANGED`).

## 6. Plan 24 sözleşme notları (Plan 25 için)

Kaynak: `crates/pgt-ddl/src/{maintenance,build/maintenance,load_maintenance,apply,manager}.rs`, `src-tauri/src/ddl.rs`. TS bindingleri `src/ipc/bindings/` altındadır; `src/ipc/commands.ts` Plan 25'te genişler.

### 6.1 İstek ve model (camelCase)

- Plan/uygulama isteği: `{ op: "maintenance", object: ObjectRef, target: MaintenanceTarget, operation: MaintenanceOp }`. **`operation`** alanı (spec §2'deki eski `op` yanlıştı: `op` istek etiketidir; Rust alanı `op`, serde adı `operation`).
- `MaintenanceOp` etiketi `op`: `{ op: "vacuum", full, freeze, analyze, verbose, disablePageSkipping, skipLocked, indexCleanup: "auto"|"on"|"off"|null, parallel: number|null }`; `{ op: "analyze", verbose, skipLocked }`; `{ op: "reindex", concurrently, verbose }`; `{ op: "cluster", verbose, usingIndex: string|null }`. Boolean alanlar zorunludur (varsayılan yok).
- `MaintenanceTarget` (yükleyici çıktısı; istemci olduğu gibi geri gönderir, değiştirmez): `kind` (`table|materialized_view|index|schema|database`), `schema` (şema/veritabanı hedefinde `null`), `name`, `table` (yalnız indeks), `partitioned`, `system`, `clusterIndex`, `indexes`, `editable`, `unsupported: string[]`.
  - `indexes`: yalnız `CLUSTER USING` için seçilebilir indeksler (geçerli, kısmi olmayan, erişim yöntemi kümelenebilir); ad (bayt) sırasıyla.
  - `system`: `pg_catalog`, `information_schema`, `pg_toast%`.
  - `editable = false` nedenleri `unsupported`'tadır: bölümlenmiş indeks ("bölümlenmiş indeks"; ilk sürümde salt okunur), geçici şema ("geçici şema (başka oturumun nesneleri)"; `pg_temp_N`/`pg_toast_temp_N`), geçersiz UTF-8 ad.
  - Database hedefi veritabanı kapsamlıdır: `databaseOid` bağlı veritabanıdır; farklıysa `Invalid("Veritabanı bakımı yalnız bağlı olunan veritabanında yapılabilir.")`. Bakımı olmayan tür → `Invalid("Bu nesne için bakım işlemi yoktur.")`.
- `ops_for(kind, partitioned, version_num)` (Rust; Plan 25 aynı kuralı TS'e taşır; değerler `"vacuum"|"analyze"|"reindex"|"cluster"`):
  - Tablo: bölümlenmemiş ya da PG 15+ → dördü; bölümlenmiş PG 14 → vacuum/analyze/reindex; bölümlenmiş PG < 14 → vacuum/analyze.
  - MV: vacuum/analyze/reindex. İndeks, şema: reindex (bölümlenmiş indeks PG < 14: boş).
  - Veritabanı: vacuum/analyze/reindex.

### 6.2 Alan yolları (`FieldError.path`) ve kurallar

- `object`: `editable = false` ("Bu nesne için bakım işlemi yapılamıyor."), eksik şema, bozuk ad.
- `op`: kümede olmayan işlem; bölümlenmiş CLUSTER PG < 15 ("Bölümlenmiş tabloda CLUSTER PostgreSQL 15 ve üzerini gerektirir."), bölümlenmiş REINDEX PG < 14, sistem şemasında CLUSTER.
- `options.<alan>` (camelCase): `skipLocked`, `indexCleanup`, `parallel`, `concurrently`, `disablePageSkipping`, `usingIndex`. Sürüm kapısı iletisi: "<SEÇENEK> PostgreSQL <N> ve üzerini gerektirir."
- Kapılar: SKIP_LOCKED, INDEX_CLEANUP, REINDEX CONCURRENTLY PG 12+; PARALLEL PG 13+ (0–1024, `FULL` ile birlikte değil); INDEX_CLEANUP AUTO PG 14+; sistem nesnesinde CONCURRENTLY yok.
- `disablePageSkipping` + `full` birlikte reddedilir (PG 10–18'de VACUUM FULL bu seçeneği reddeder; canlı doğrulandı).
- ANALYZE PG 10'da parantezsiz `ANALYZE VERBOSE [hedef]` yazılır (PG 11+ parantezli); SKIP_LOCKED zaten PG 12+.
- `usingIndex` ∈ `target.indexes`; yoksa ve `clusterIndex` da yoksa hata.
- Risk: VACUUM FULL, CLUSTER, REINDEX (CONCURRENTLY'siz; SCHEMA/DATABASE dahil) Destructive; kilit notu `Statement.note`'ta (VACUUM FULL/CLUSTER: "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler." — veritabanı genelinde + " Tablolar sırayla kilitlenir." — ardından + " Tablo boyutu kadar ek boş disk alanı gerekir."; REINDEX: "Tabloda SHARE, indekste ACCESS EXCLUSIVE kilit alır; yazma ve indeksi kullanan okumalar bekler." — SCHEMA/DATABASE'de + " Tablolar sırayla kilitlenir."). Diğerleri Safe; Safe `REINDEX … CONCURRENTLY` de not taşır: "Başarısız/iptal edilirse geçersiz bir *_ccnew indeksi kalabilir." (risk Safe kalır, onay istemez). Her plan tek ifade, `transactional = false`.
- Kapılar (`apply`): kilitli sunucu `locked`; `database_oid = None` → `NEEDS_DATABASE`; yeniden yüklenen hedef istekteki ile eşit değilse `PLAN_CHANGED`; bakım veritabanı koruması uygulanmaz; `created = None`, uygulama sonrası doğrulama yok.

### 6.3 İpuçları (`ApplyOutcome::Failed.error.hint`, op = `maintenance`)

| SQLSTATE | İpucu |
|---|---|
| 55P03 | "Nesne başka bir oturumca kilitli; daha sonra deneyin ya da lock timeout'u artırın." (`MAINTENANCE_LOCK_HINT`) |
| 42501 | "Bakım işlemi için nesnenin sahibi (ya da PG 17+'da MAINTAIN yetkisi) gerekir." |
| 0A000 | "Bu nesnede bu seçenek desteklenmiyor." |
| 25001 | "Bu bakım işlemi transaction bloğu içinde çalıştırılamaz; yeniden deneyin." |
| 57014 | yok (iptal iletisi yeter) |

### 6.4 İletiler (`ApplyOutcome.messages`)

- Tüm DDL uygulamalarında (create/alter/drop/privileges/maintenance) `Applied` ve `Failed` ile gelir; yoksa alan yazılmaz (`messages?: ServerMessage[]`). Oturum kurulumundaki NOTICE'lar da dahildir. Veri düzenleyici iletileri düşürür.
- `ServerMessage { severity, code: string|null, message, detail: string|null, hint: string|null }`.
- Sınır: **toplam 500** ileti (taşma iletisi dahil: 499 ileti + `{ severity: "INFO", code: null, message: "<N> ileti daha gösterilmedi" }`); alan başına 4096 bayt, UTF-8 sınırında kesilir ve `…` eklenir. İfade bittikten sonra kuyruktakiler ve bağlantı kapanırken kalanlar (en çok 1 sn) alınır.
- Gizlilik: iletiler `tracing`'e, hata iletisine ya da diske yazılmaz.

### 6.5 İlerleme ve komutlar

- Komutlar: `ddl_load_maintenance_target(serverId, databaseOid, object) -> MaintenanceTarget`; `ddl_maintenance_progress(serverId, databaseOid, applyId) -> MaintenanceProgress | null`. Önizleme/uygulama/iptal mevcut `ddl_preview`/`ddl_apply`/`ddl_cancel` ile yapılır; `applyId` istemci tarafından `ApplyOptions`'ta verilir ve ilerleme sorgusunda aynısı kullanılır.
- `MaintenanceProgress { phase: string, done: number|null, total: number|null, unit: "blocks"|"tuples"|"indexes"|null }`. Bilinmeyen/bitmiş/iptal edilmiş `applyId` ya da görünümde satır yok → `null`. Sorgu katalog havuzunda, yalnız kendi uygulama pid'imiz için çalışır (pid yalnız `op = "maintenance"` uygulamalarında okunur; diğer DDL uygulamalarında ilerleme hep `null`); ön yüz ~1 sn aralıkla sorar.
- Kaynaklar ve `done/total/unit`:
  - `pg_stat_progress_vacuum` (FULL'suz VACUUM): aşamaya göre — `scanning heap` → `heap_blks_scanned / heap_blks_total`, `vacuuming heap` → `heap_blks_vacuumed / heap_blks_total`, `blocks`; diğer aşamalarda (ör. `vacuuming indexes`, `truncating heap`) ya da sayaç/toplam boşsa done/total/unit `null`.
  - `pg_stat_progress_cluster` (PG 12+; CLUSTER ve VACUUM FULL): `seq scanning heap` → `heap_blks_scanned / heap_blks_total`, `blocks`; `index scanning heap` ve `writing new heap` → `heap_tuples_written` (total yok), `tuples`; `rebuilding index` → `index_rebuild_count` (total yok), `indexes`; diğer aşamalarda done/total/unit `null`.
  - `pg_stat_progress_create_index` (PG 12+; REINDEX): aşamaya göre — `… loading tuples in tree` (ör. `building index: loading tuples in tree`) ve `tuples_total > 0` → `tuples_done / tuples_total` (`tuples`); tarama aşamaları (`%scanning%`: `building index: scanning table`, `index validation: scanning index/table`) ve `blocks_total > 0` → `blocks_done / blocks_total` (`blocks`); diğer aşamalarda `null` (önceki aşamanın blok sayacı dolu kalabildiğinden yalnız sayaca bakılmaz).
  - `pg_stat_progress_analyze` (PG 13+): `sample_blks_total > 0` ise `sample_blks_scanned / sample_blks_total` (`blocks`), yoksa `null`.
  - PG 10–11'de yalnız vacuum görünümü sorgulanır; PG 12'de analyze yok.
- `phase` PG'nin İngilizce aşama metnidir (ör. `scanning heap`, `vacuuming indexes`, `seq scanning heap`, `sorting tuples`, `rebuilding index`, `acquiring sample rows`); metinler PG sürümleri arasında farklı olabilir: ön yüz bilinenleri çevirir, bilinmeyeni olduğu gibi gösterir. Aşama boşsa `"unknown"` (yedek). `total` yoksa `done` sayaçtır (belirsiz çubuk + sayı).
- İptal/bitişten sonra pid yuvası temizlenir ve ilerleme `null` döner.

### 6.6 Plan 25 için uygulama notları

- **Hedefi yeniden yükle:** her başarılı uygulamadan sonra `ddl_load_maintenance_target` yeniden çağrılır ve diyalogdaki hedef yenisiyle değiştirilir. Örn. `CLUSTER … USING i` `clusterIndex`'i değiştirir; eski hedefle bir sonraki istek `PLAN_CHANGED` ile reddedilir.
- **Yarım kalan `REINDEX CONCURRENTLY`:** başarısız ya da iptal edilen `REINDEX … CONCURRENTLY` geçersiz bir `*_ccnew` indeksi bırakabilir (PG davranışı). Risk Safe kalır; `Statement.note` "Başarısız/iptal edilirse geçersiz bir *_ccnew indeksi kalabilir." der. Arayüz bu notu önizlemede gösterir; başarısız/iptal sonucunda da yineler ve geçersiz indeksin elle silinmesi (`DROP INDEX CONCURRENTLY`) ya da işlemin yeniden denenmesi gerektiğini belirtir.
- **Disk alanı:** `VACUUM FULL` ve `CLUSTER` tabloyu yeniden yazar; tablo (ve indeksleri) boyutu kadar ek boş disk gerekir. Bu, notlarının sonundadır (" Tablo boyutu kadar ek boş disk alanı gerekir."); arayüz notu olduğu gibi gösterir.
- **WARNING iletileri vurgulanır:** `messages` içindeki `severity = "WARNING"` iletileri listede belirgin gösterilir (renk/simge) ve sonuç özetinde sayılır. Yetkisiz nesneler sunucuca atlanır ve WARNING verir (ör. veritabanı genelinde VACUUM'da sahibi olunmayan tablolar) ama sonuç `Applied`'dır; vurgu olmadan kullanıcı işlemin tamamlandığını sanır.

## 7. Plan 25 uygulama notları

- **Sürüm kaynağı:** sunucu düğümünün `detail` alanı (`ServerInfo.version`) `parseVersionNum` ile ayrıştırılır. Sürüm `null` ise (bağlı değil/ayrıştırılamadı) istemci kapıları uygulanmaz; sunucu reddederse hata ilgili seçeneğin altında gösterilir.
- **Tasarımcıdan içe aktarma yok:** küçük uygulama kapısı, önizleme debounce'u (150 ms) ve kilit-zaman-aşımı listesi ana paketi büyütmemek için `maintenance-controller.ts` içine kopyalandı (designer modülünü içe aktarmak bölünmüş parçaları çeker).
- **İşlem ve seçenek kapıları:** `opsFor` hedef türü, bölümleme ve sürüme göre sunulan işlemleri belirler; kümede olmayan işlem (ör. bölümlenmiş tabloda PG < 15 CLUSTER) seçicide hiç gösterilmez. `opBlocked` yalnız CLUSTER içindir (sistem nesnesi; kümeleme indeksi ve seçilebilir indeks yok): işlem seçicide gerekçeli devre dışı bırakılır, gizlenmez. CONCURRENTLY (sistem nesnesi, PG < 12) ve diğer seçenek kapıları `optionStates` ile seçenek düzeyinde devre dışı + gerekçelidir. Uygula kapalıyken gerekçe alt çubukta görünür metin olarak (`aria-describedby`) ve `title`'da gösterilir.
- **PARALLEL kırpma:** değer 0–1024 aralığına kırpılır (sunucu sınırı); FULL ile birlikte devre dışıdır.
- **Canlı kilit:** diyalog açıkken sunucu kilitlenirse Uygula kapanır ve "salt okunur kilitli" şeridi görünür; kapı gerekçesi alt çubukta görünür metin olarak ve `aria-describedby` ile okunur.
- **WARNING sayımı ve `pgt-core` değişikliği:** `Notice` önem derecesi artık yerelleştirilmemiş (`parsed_severity`) okunur; `lc_messages` Türkçe iken de `WARNING`/`NOTICE` gelir ve sayım güvenilirdir. Bu değişiklik Query Tool iletilerini de etkiler: önem etiketleri her zaman İngilizcedir.
- **Bağlantı kaybı** özel ele alınmaz: yükleme sırasında yük hatası, uygulama sırasında başarısız sonuç olarak gösterilir.
- **Paket (ölçülen):** ana paket 129,476 B, açılış kümesi 157,149 B, `__vite__mapDeps` 35; diyalog tembel parçada (`MaintenanceDialog-*.js`, 34.40 kB / gzip 11.27 kB). Ek baytlar `strings-server.ts` üzerinden geri ödendi; `bundle-budget.json` tavanları bu değerlere çekildi.
