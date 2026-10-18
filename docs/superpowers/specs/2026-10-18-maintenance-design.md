# pg-tools — Bakım İşlemleri Tasarımı (VACUUM / ANALYZE / REINDEX / CLUSTER)

- **Tarih:** 2026-10-18
- **Durum:** Taslak (Plan 24 arka ucu yazıldı; Plan 25 ön yüzü kalan)
- **Kapsam:** Explorer'dan bir nesne için bakım işlemi çalıştırma: seçenekler, SQL önizlemesi, kilit uyarısı, onay, uygulama, iptal, ilerleme (`pg_stat_progress_*`) ve `VERBOSE` iletileri.
- **Temel:** `pgt-ddl` çerçevesi (`PlanRequest`, `DdlManager::apply` kapıları, `run_statements`, `CancelMap`) ve object-dialogs spec'inin risk modeli (§3).
- **Teslim:** Plan 24 (arka uç), Plan 25 (ön yüz: "Bakım…" diyaloğu).

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| İşlemler ve hedefler | **VACUUM**: tablo (sıradan/bölümlenmiş/bölüm), MV, veritabanı (tümü). **ANALYZE**: tablo, MV, veritabanı. **REINDEX**: indeks (`INDEX`), tablo/MV (`TABLE`), şema (`SCHEMA`), veritabanı (`DATABASE` — PG gereği yalnız bağlı veritabanı, adla). **CLUSTER**: tablo (sıradan; bölümlenmiş PG 15+), isteğe bağlı `USING <indeks>`. Desteklenmeyen birleşim → doğrulama hatası. Tablo dışı nesneler (view, sequence, foreign table…) için işlem yok. |
| Seçenekler | VACUUM: `FULL`, `FREEZE`, `ANALYZE`, `VERBOSE`, `DISABLE_PAGE_SKIPPING`, `SKIP_LOCKED` (PG 12+), `INDEX_CLEANUP {AUTO (PG 14+), ON, OFF}` (PG 12+), `PARALLEL n` (PG 13+, 0–1024; `FULL` ile birlikte değil). ANALYZE: `VERBOSE`, `SKIP_LOCKED` (PG 12+). REINDEX: `CONCURRENTLY` (PG 12+; sistem kataloğunda değil), `VERBOSE`. CLUSTER: `VERBOSE`, `USING` indeksi (tablonun indeksi olmalı; yoksa ve tabloda kümeleme indeksi yoksa hata). Sürüm kapıları doğrulama hatasıdır (sessiz atlama yok). |
| Sözdizimi | Her zaman parantezli seçenek listesi, değersiz boolean anahtar sözcükleri (PG 10–11 uyumu): `VACUUM (FULL, ANALYZE, VERBOSE) s.t`; seçenek yoksa parantez yazılmaz. REINDEX PG 14+'ta `REINDEX (VERBOSE) TABLE CONCURRENTLY s.t`, PG 12–13'te `REINDEX (VERBOSE) TABLE CONCURRENTLY s.t` (aynı); PG 10–11'de `REINDEX (VERBOSE) TABLE s.t`. CLUSTER PG 10–16: `CLUSTER [VERBOSE] s.t [USING i]`; PG 17+ parantezli biçim de geçerli — tek biçim olarak `CLUSTER VERBOSE s.t USING i` (tüm sürümlerde geçerli). Tanımlayıcılar yalnız `quote_ident`/`qualified`. |
| Çalıştırma | Tek ifade, **transaction dışı** (`transactional = false`; VACUUM/REINDEX CONCURRENTLY/SCHEMA/DATABASE zorunlu, diğerleri tutarlılık için). `statement_timeout = 0` (DDL oturumu); `lock_timeout` kullanıcı seçimi (ApplyOptions). |
| Risk | **Destructive** (onay ister): özel kilit alan işlemler — `VACUUM FULL`, `CLUSTER`, `REINDEX` (CONCURRENTLY olmadan), `REINDEX SCHEMA/DATABASE` (CONCURRENTLY olmadan). Not: "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler." **Safe**: `VACUUM` (FULL'suz), `ANALYZE`, `REINDEX … CONCURRENTLY`. DataLoss yok (üretimde ad yazma yok). Kilitli sunucuda (salt okunur kilit) tüm bakım işlemleri reddedilir. |
| Nesne denetimi | İstek, yüklenen `MaintenanceTarget { kind, schema, name, relkind, partitioned, system, has_cluster_index, indexes }`'i taşır; uygulamada yeniden yüklenir ve eşitlik aranır (`PLAN_CHANGED`), sistem şeması nesnesinde yalnız `VACUUM`/`ANALYZE` serbest, `REINDEX CONCURRENTLY` ve `CLUSTER` reddedilir (PG kuralı). Veritabanı hedefi bağlı (hedef) veritabanıdır. |
| İletiler | Uygulama bağlantısının NOTICE/INFO iletileri (`VERBOSE` çıktısı, uyarılar) toplanır ve `ApplyOutcome`'a eklenir (`messages`, yoksa alan yazılmaz): en çok 500 ileti, ileti başına 4 KiB (kesilir; kesildiği belirtilir). İletiler yalnız kullanıcıya döner; `tracing`'e yazılmaz. |
| İlerleme | Uygulama bağlantısının arka uç pid'i kaydedilir; `maintenance_progress(apply_id)` komutu katalog havuzundan `pg_stat_progress_vacuum` (9.6+), `pg_stat_progress_cluster` (12+; VACUUM FULL ve CLUSTER), `pg_stat_progress_create_index` (12+; REINDEX), `pg_stat_progress_analyze` (13+) okur → `{ phase, done, total }` (bilinmiyorsa `None`). Sürümde görünüm yoksa ya da satır yoksa `None` (belirsiz ilerleme). Ön yüz 1 sn aralıkla sorar. |
| Kapsam dışı | `VACUUM` çoklu tablo listesi, kolon listesi (`ANALYZE t(c)`), `REINDEX SYSTEM`, `TABLESPACE` (REINDEX PG 14+), `vacuumdb`/`reindexdb` paralel iş, zamanlama, `pg_repack`. |

## 2. Model (özet)

- `MaintenanceOp` = `Vacuum(VacuumOptions)` | `Analyze(AnalyzeOptions)` | `Reindex(ReindexOptions)` | `Cluster(ClusterOptions)` (serde `tag = "op"`).
- `MaintenanceTarget` (yükleyici; salt okunur kimlik): `kind` (`table` | `materialized_view` | `index` | `schema` | `database`), `schema`, `name`, `table` (indeks için ait tablo), `partitioned`, `system`, `clusterIndex: Option<String>`, `indexes: Vec<String>` (tablo için, CLUSTER USING seçimi), `editable`, `unsupported`.
- `PlanRequest::Maintenance { object: ObjectRef, target: MaintenanceTarget, op: MaintenanceOp }` (`op = "maintenance"`).
- `ApplyOutcome::{Applied, Failed}` + `messages?: Vec<ServerMessage { severity, code, message, detail, hint }>`.
- `MaintenanceProgress { phase: String, done: Option<u64>, total: Option<u64>, unit: "blocks" | "tuples" | "indexes" | "partitions" | null }`.

## 3. Arayüz (Plan 25 özeti)

- Explorer: tablo, MV, indeks, şema, veritabanı düğümlerinde **"Bakım…"** (kilitli sunucuda devre dışı).
- Tembel diyalog: işlem seçimi (hedefe ve sürüme göre), seçenekler, SQL önizlemesi, risk + kilit notu, lock timeout, onay kutusu (Destructive), Uygula / İptal; çalışırken ilerleme çubuğu (belirsiz/yüzde, aşama adı), geçen süre; bitince sonuç + iletiler listesi (`VERBOSE`). Diyalog çalışırken kapatılırsa iptal onayı.

## 4. Güvenlik ve gizlilik

- İfade metni, nesne adları ve iletiler `tracing`'e yazılmaz (yalnız `op`, `kind`, süre, sonuç kodu).
- İlerleme sorgusu yalnız kendi uygulama pid'imiz için (başka oturumların ilerlemesi gösterilmez).

## 5. Test stratejisi

- Saf üretici: insta (her işlem × hedef × seçenek × sürüm), sürüm kapıları, risk sınıfları, `transactional = false`, düşmanca adlar.
- Docker PG 10–18: her işlemin gerçekten çalışması (`pg_stat_user_tables.last_vacuum/last_analyze`, `pg_index.indisclustered`, yeni indeks relfilenode), `VERBOSE` iletilerinin `messages`'a düşmesi, iptal (uzun VACUUM FULL ya da `pg_sleep` kilit beklemesiyle 57014), lock_timeout (55P03), ilerleme sorgusunun çalışırken satır döndürmesi (en iyi çaba; zamanlamaya bağlı test `None`'u da kabul eder), kilitli sunucu reddi, bayat hedef (`PLAN_CHANGED`).
