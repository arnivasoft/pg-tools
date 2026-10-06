# Dashboard Arka Uç (Plan 8) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Dashboard'un Rust arka ucu: yeni `pgt-dash` crate'i (ayrı bağlantı, sabit SQL sorguları, iptal/sonlandırma) ve `dash_*` Tauri komutları.

**Mimari:** `pgt-core`'a yeni `SessionProfile::Dashboard` eklenir. `pgt-dash::DashManager` her dashboard sekmesi için tek `DedicatedConn` tutar (havuza dönmez), sorguları sabit `.sql` dosyalarından çalıştırır, sonuçları yazılı (typed) yapılara çevirir. Tauri katmanı yalnızca ince `dash_*` komutlarıdır. Ön yüz Plan 9'dadır.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, mevcut `pgt_core::testing` Docker matrisi.

**Spec:** `docs/superpowers/specs/2026-10-06-dashboard-design.md` (§2 mimari, §3 veri kaynakları, §5 güvenlik, §6 test).

## Global Constraints

- PostgreSQL **10–18** vanilla; her sorgu her sürümde çalışmalı (sürüme göre `.pg10`/`.pg13` varyant dosyaları).
- Sorgu metinleri **sabit** SQL dosyalarıdır (`include_str!`); kullanıcıdan SQL alınmaz. Dışarıdan gelen tek değerler bağlı parametredir (`pid: i32`, veritabanı adı, sıralama anahtarı → kodda sabit sütuna eşlenir, asla metin birleştirilmez).
- Sorgu metinleri (`pg_stat_activity.query`, `pg_stat_statements.query`) **günlüğe yazılmaz**, hata iletisine konmaz, diske kaydedilmez. Parola/URL asla günlüğe girmez.
- Dashboard bağlantısı: `application_name = 'pg-tools dash'`, `default_transaction_read_only = on`, `statement_timeout = 10000`, `lock_timeout = 2000`. `search_path` verilmez (hiçbir sorgu şemasız ad kullanmaz; hepsi `pg_catalog.` ya da sistem görünümüdür).
- Süreler sunucuda saniye (`float8`) olarak hesaplanır (`extract(epoch from now() - x)`); istemci saat farkına bağlı değildir. Örnek zamanı `clock_timestamp()` epoch'udur.
- Tüm Tauri komutları gelecekleri `Box::pin` ile kutular (commands.rs'deki 1 MB ana yığın gerekçesi).
- ts-rs: tüm dışa verilen yapılar `#[derive(Serialize, TS)]`, `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`; `i64`/`u64` alanlar `#[ts(type = "number")]`. `src/ipc/bindings`'e cargo test sonrası düşen salt satır sonu değişiklikleri `git checkout -- src/ipc/bindings` ile geri alınır; yeni dosyalar commit edilir.
- Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Test sunucuları: Docker'la geçici; `PGT_TEST_URLS` yalnızca test komutunun ortamında; URL/parola asla yazdırılmaz; işi biten kapsayıcılar silinir. `scripts/test-matrix.ps1` zaten `cargo test --workspace` çalıştırdığı için yeni crate otomatik girer.
- Git worktree **kullanılmaz** (node_modules birleşim noktası sorunu).

## Review Focus

- **Yetkisiz rol** (`pg_monitor` üyesi değil): başkalarının `query` alanı `NULL` gelir; `pg_cancel/terminate_backend` 42501 verir → hata `ErrorPayload` olarak döner, sessiz boş sonuç değil.
- **Kendi pid'i:** `dash_signal` kendi dashboard bağlantısının pid'ine reddedilir (kendini sonlandırma).
- **Var olmayan pid:** `pg_cancel_backend` `false` döner (PG uyarı verir); sonuç `NotFound` olarak gelir, hata değil.
- **Eklenti yok / yüklü değil:** `pg_stat_statements` görünümü yok (42P01) ya da `shared_preload_libraries`'te değil (55000) → `Statements::Unavailable{reason}`, hata değil.
- **Bağlantı koparsa:** kopan dashboard bağlantısı bir sonraki çağrıda sessizce yeniden açılır; açılamazsa `ConnectionLost`/bağlantı hatası döner ve oturum kayıtlı kalır (tekrar denenebilir).
- **Boş/uç veri:** hiç oturum yok, hiç kilit yok, `pg_settings`'te `NULL` `setting` (yetkisiz), çok uzun sorgu (4096 sınırı), `client_addr = NULL` (Unix soket).

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-core/src/connection.rs` | `SessionProfile::Dashboard` (ad, `options`, `SET` yedeği, doğrulama) |
| `crates/pgt-dash/Cargo.toml`, `src/lib.rs` | crate iskeleti, modül dışa aktarımları |
| `crates/pgt-dash/src/types.rs` | Dışa verilen yazılı sonuç yapıları |
| `crates/pgt-dash/src/sql/*.sql` | Sabit sorgular (+ `.pg10` / `.pg13` varyantları) |
| `crates/pgt-dash/src/queries.rs` | Sorgu çalıştırma + satır → yapı dönüşümü, sürüm varyantı seçimi |
| `crates/pgt-dash/src/manager.rs` | `DashManager`, `DashSession` (yaşam döngüsü, yeniden bağlanma, sinyal) |
| `crates/pgt-dash/tests/*.rs` | Docker matrisi entegrasyon testleri |
| `src-tauri/src/dash.rs` | `dash_*` Tauri komutları |
| `src-tauri/src/state.rs`, `main.rs`, `commands.rs` | `AppState.dash`, komut kaydı, sunucu bağlantısı kesilince/yeniden yüklenince kapatma |

---

### Task 1: `SessionProfile::Dashboard` (pgt-core)

**Dosyalar:**
- Değiştir: `crates/pgt-core/src/connection.rs` (profil, `application_name`, `profile_session_options`, `profile_session_sets`, `validate_profile`, `settings_ok_for` ve bunların birim testleri)

**Arayüzler:**
- Üretir: `SessionProfile::Dashboard` (alansız varyant); `ConnectionManager::open_dedicated(server, target, SessionProfile::Dashboard)` bu profille çalışır.

- [ ] **Adım 1: Başarısız birim testleri yaz** (`connection.rs` içindeki mevcut `#[cfg(test)]` modülünde, `Query` profilinin testlerinin yanında)

```rust
#[test]
fn dashboard_profile_options_and_sets_agree() {
    let opts = profile_session_options(SessionProfile::Dashboard);
    assert!(opts.contains("default_transaction_read_only=on"));
    assert!(opts.contains("statement_timeout=10000"));
    assert!(opts.contains("lock_timeout=2000"));
    assert!(!opts.contains("search_path"));
    let from_options: Vec<String> = opts
        .split(" -c ")
        .map(|p| p.trim_start_matches("-c ").replacen('=', " = ", 1))
        .map(|p| format!("SET {p}"))
        .collect();
    let sets: Vec<String> = profile_session_sets(SessionProfile::Dashboard).iter().map(|s| s.to_string()).collect();
    assert_eq!(from_options, sets);
}

#[test]
fn dashboard_profile_settings_check() {
    // settings_ok_for 4 parametre alır (read_only, statement_timeout, search_path); Dashboard search_path'a bakmaz.
    assert!(settings_ok_for(SessionProfile::Dashboard, "on", "10s", "\"$user\", public"));
    assert!(!settings_ok_for(SessionProfile::Dashboard, "off", "10s", "public"));
    assert!(!settings_ok_for(SessionProfile::Dashboard, "on", "30s", "public"));
}

#[test]
fn dashboard_application_name() {
    assert_eq!(SessionProfile::Dashboard.application_name(), "pg-tools dash");
}
```

- [ ] **Adım 2: Testi çalıştır, derleme hatasıyla başarısız olduğunu doğrula**

Çalıştır: `cargo test -p pgt-core dashboard_profile`
Beklenen: `no variant named Dashboard`.

- [ ] **Adım 3: Uygula.** `SessionProfile`'a `Dashboard` ekle; `DASHBOARD_APPLICATION_NAME = "pg-tools dash"`; `profile_session_options` → `"-c default_transaction_read_only=on -c statement_timeout=10000 -c lock_timeout=2000"`; `profile_session_sets` → `SET default_transaction_read_only = on`, `SET statement_timeout = 10000`, `SET lock_timeout = 2000`; `settings_ok_for` Dashboard kolu: `read_only.trim() == "on" && timeout_ms(statement_timeout) == Some(10_000)`; `validate_profile` Dashboard için `Ok(())`. Derleyicinin işaret ettiği tüm tam kapsamlı `match`'leri güncelle (arama: `SessionProfile::`). `dedicated_once` içindeki ayar doğrulaması `settings_ok_for`'u kullandığından Dashboard'a kendiliğinden uyar; `lock_timeout` doğrulanmaz (pgbouncer'da `options` yok sayılırsa `statement_timeout` denetimi zaten bağlantıyı reddeder).

- [ ] **Adım 4:** `cargo test -p pgt-core` — tümü geçmeli (mevcut profil testleri bozulmamalı). `cargo clippy -p pgt-core -- -D warnings` temiz.

- [ ] **Adım 5: Commit**

```bash
git add crates/pgt-core/src/connection.rs
git commit -m "feat(core): Dashboard oturum profili (salt okunur, 10 sn, lock_timeout 2 sn)"
```

---

### Task 2: `pgt-dash` iskeleti, yazılı yapılar ve örnek/oturum sorguları

**Dosyalar:**
- Oluştur: `crates/pgt-dash/Cargo.toml`, `crates/pgt-dash/src/lib.rs`, `types.rs`, `queries.rs`, `sql/sample.sql`, `sql/overview.sql`, `sql/activity.sql`, `sql/activity.pg13.sql`
- Değiştir: kök `Cargo.toml` (`members` + `[workspace.dependencies] pgt-dash`)
- Test: `crates/pgt-dash/tests/queries.rs`

**Arayüzler:**
- Üretir (`types.rs`, hepsi `Serialize + TS`, camelCase):

```rust
pub struct Sample {            // grafik örneği
    #[ts(type = "number")] pub at: f64,            // sunucu clock_timestamp() epoch saniyesi
    #[ts(type = "number")] pub xact_commit: i64,
    #[ts(type = "number")] pub xact_rollback: i64,
    #[ts(type = "number")] pub tup_returned: i64,
    #[ts(type = "number")] pub tup_fetched: i64,
    #[ts(type = "number")] pub tup_inserted: i64,
    #[ts(type = "number")] pub tup_updated: i64,
    #[ts(type = "number")] pub tup_deleted: i64,
    #[ts(type = "number")] pub blks_read: i64,
    #[ts(type = "number")] pub blks_hit: i64,
    #[ts(type = "number")] pub sessions_active: i64,
    #[ts(type = "number")] pub sessions_idle: i64,
    #[ts(type = "number")] pub sessions_idle_in_tx: i64,   // 'idle in transaction' + '... (aborted)'
    #[ts(type = "number")] pub sessions_other: i64,
}
pub struct Overview {
    pub version: String,                              // version() ilk bölüm değil, tam metin
    #[ts(type = "number")] pub uptime_secs: f64,
    #[ts(type = "number")] pub max_connections: i64,
    #[ts(type = "number")] pub connections: i64,       // client backend sayısı
    #[ts(type = "number | null")] pub oldest_xact_secs: Option<f64>,
    pub database: Option<String>,                      // veritabanı düzeyinde ad
}
pub struct ActivityRow {
    pub pid: i32, pub leader_pid: Option<i32>,
    pub database: Option<String>, pub user: Option<String>, pub application: String,
    pub client_addr: Option<String>, pub backend_type: String, pub state: Option<String>,
    pub wait_event_type: Option<String>, pub wait_event: Option<String>,
    #[ts(type = "number | null")] pub backend_age_secs: Option<f64>,
    #[ts(type = "number | null")] pub xact_age_secs: Option<f64>,
    #[ts(type = "number | null")] pub query_age_secs: Option<f64>,
    #[ts(type = "number | null")] pub state_age_secs: Option<f64>,
    pub query: Option<String>,        // yetkisizse None
    pub is_self: bool,                // bu dashboard bağlantısı
}
```

- Tüketir: `pgt_core::connection::DedicatedConn` (`client: tokio_postgres::Client`, `info.version_num`).
- Üretir (`queries.rs`): `pub async fn sample(c: &Client, db: Option<&str>) -> Result<Sample>`, `overview(c, db)`, `activity(c, version_num, db)` — hepsi `pgt_core::Result`. Yardımcı: `fn pick(version_num: u32, variants: &[(u32, &'static str)]) -> &'static str` (ana sürüme göre en yüksek eşleşen; `(10, ...)`, `(13, ...)`).

- [ ] **Adım 1: İskelet.** Kök `Cargo.toml`'a `crates/pgt-dash` üye + `pgt-dash = { path = "crates/pgt-dash" }`. `crates/pgt-dash/Cargo.toml`: `pgt-query`'nin bağımlılıkları (tokio, tokio-postgres, pgt-core, serde, uuid, tracing, ts-rs); dev-bağımlılık: `pgt-core` `features = ["testing"]`, `tempfile`, `tokio` `test-util`. `lib.rs` başına doc: "Dashboard motoru. Sorgu metinleri bu crate'te asla günlüğe yazılmaz."

- [ ] **Adım 2: SQL dosyaları (tam metin).**

`sql/sample.sql` (`$1` = veritabanı adı ya da NULL):

```sql
WITH d AS (
  SELECT coalesce(sum(xact_commit), 0)::int8   AS xact_commit,
         coalesce(sum(xact_rollback), 0)::int8 AS xact_rollback,
         coalesce(sum(tup_returned), 0)::int8  AS tup_returned,
         coalesce(sum(tup_fetched), 0)::int8   AS tup_fetched,
         coalesce(sum(tup_inserted), 0)::int8  AS tup_inserted,
         coalesce(sum(tup_updated), 0)::int8   AS tup_updated,
         coalesce(sum(tup_deleted), 0)::int8   AS tup_deleted,
         coalesce(sum(blks_read), 0)::int8     AS blks_read,
         coalesce(sum(blks_hit), 0)::int8      AS blks_hit
  FROM pg_catalog.pg_stat_database
  WHERE $1::text IS NULL OR datname = $1::text
), a AS (
  SELECT count(*) FILTER (WHERE state = 'active')::int8 AS active,
         count(*) FILTER (WHERE state = 'idle')::int8 AS idle,
         count(*) FILTER (WHERE state LIKE 'idle in transaction%')::int8 AS idle_in_tx,
         count(*) FILTER (WHERE state IS DISTINCT FROM 'active' AND state IS DISTINCT FROM 'idle'
                            AND state NOT LIKE 'idle in transaction%')::int8 AS other
  FROM pg_catalog.pg_stat_activity
  WHERE backend_type = 'client backend' AND ($1::text IS NULL OR datname = $1::text)
)
SELECT extract(epoch FROM clock_timestamp())::float8 AS at, d.*, a.active, a.idle, a.idle_in_tx, a.other
FROM d, a
```

`sql/overview.sql`:

```sql
SELECT pg_catalog.version() AS version,
       extract(epoch FROM now() - pg_postmaster_start_time())::float8 AS uptime_secs,
       current_setting('max_connections')::int8 AS max_connections,
       (SELECT count(*) FROM pg_catalog.pg_stat_activity
         WHERE backend_type = 'client backend' AND ($1::text IS NULL OR datname = $1::text))::int8 AS connections,
       (SELECT extract(epoch FROM now() - min(xact_start))::float8 FROM pg_catalog.pg_stat_activity
         WHERE xact_start IS NOT NULL AND pid <> pg_backend_pid()
           AND ($1::text IS NULL OR datname = $1::text)) AS oldest_xact_secs
```

`sql/activity.pg13.sql` (PG 13+; `leader_pid` var). `sql/activity.sql` (PG 10–12) aynıdır, yalnız `leader_pid` yerine `NULL::int4 AS leader_pid`:

```sql
SELECT pid::int4, leader_pid::int4 AS leader_pid,
       datname::text AS database, usename::text AS "user", application_name::text AS application,
       client_addr::text AS client_addr, backend_type::text AS backend_type, state::text AS state,
       wait_event_type::text, wait_event::text,
       extract(epoch FROM now() - backend_start)::float8 AS backend_age_secs,
       extract(epoch FROM now() - xact_start)::float8    AS xact_age_secs,
       extract(epoch FROM now() - query_start)::float8   AS query_age_secs,
       extract(epoch FROM now() - state_change)::float8  AS state_age_secs,
       left(query, 4096)::text AS query,
       (pid = pg_backend_pid()) AS is_self
FROM pg_catalog.pg_stat_activity
WHERE backend_type <> 'walsender' AND ($1::text IS NULL OR datname = $1::text)
ORDER BY (state = 'active') DESC NULLS LAST, query_start NULLS LAST, pid
LIMIT 5001
```

(Yetkisiz kullanıcıda başkalarının `query` değeri `<insufficient privilege>` gelir; `queries.rs` bu sabit metni `None`'a çevirir: PG metni yerelleştirilmediği için `== "<insufficient privilege>"` karşılaştırması yeterlidir. Arka plan işçileri `datname IS NULL` ile gelir; veritabanı süzgeci yalnızca istemci oturumlarını bırakır — bu beklenen davranıştır.)

- [ ] **Adım 3: Başarısız entegrasyon testlerini yaz** (`tests/queries.rs`; `servers()` yardımcısı `pgt-query/tests/session.rs`'tekiyle aynı: `PGT_TEST_URLS` yoksa `eprintln!` ile atla). Bağlantı: `ConnectionManager::connect(config, password)` + `open_dedicated(id, Target::Maintenance, SessionProfile::Dashboard)`.

```rust
#[tokio::test]
async fn sample_and_overview_run_on_every_version() {
    for s in servers() {
        let c = dash_conn(&s).await;                      // dedicated conn
        let v = c.info.version_num;
        let a = queries::sample(&c.client, None).await.unwrap();
        assert!(a.at > 1.0e9 && a.sessions_active >= 1, "kendi sorgumuz etkin sayılmalı (v{v})");
        let o = queries::overview(&c.client, None).await.unwrap();
        assert!(o.max_connections >= 1 && o.uptime_secs > 0.0 && o.version.starts_with("PostgreSQL"));
        let b = queries::sample(&c.client, None).await.unwrap();
        assert!(b.xact_commit >= a.xact_commit && b.at >= a.at);   // sayaçlar monoton
    }
}

#[tokio::test]
async fn activity_marks_self_and_filters_by_database() {
    for s in servers() {
        let c = dash_conn(&s).await;
        let rows = queries::activity(&c.client, c.info.version_num, None).await.unwrap();
        let me: Vec<_> = rows.iter().filter(|r| r.is_self).collect();
        assert_eq!(me.len(), 1);
        assert_eq!(me[0].application, "pg-tools dash");
        assert_eq!(me[0].state.as_deref(), Some("active"));
        let none = queries::activity(&c.client, c.info.version_num, Some("yok_boyle_bir_db")).await.unwrap();
        assert!(none.is_empty());
    }
}

#[tokio::test]
async fn activity_hides_other_users_queries_without_privilege() {
    // Yönetici istemciyle yetkisiz rol + başka oturum aç; dash o rolle bağlanır.
    // Beklenen: başka rolün satırında `query == None`, kendi satırında `Some`.
}
```

Üçüncü test (`activity_hides_...`): `admin_client` ile `CREATE ROLE pgt_dash_lowpriv LOGIN PASSWORD '<rastgele>'` (parola testte `uuid` ile üretilir, yazdırılmaz); yönetici oturumunda `SELECT pg_sleep(30)` arka plan görevi başlat (kendi yöneticisi başka rol); `ServerConfig` kopyasında `username` yetkisiz rol, parola o rol; `connect` + `open_dedicated`; `activity` satırlarında `pg_sleep` oturumunun (`application = "pg-tools-test"`) `query == None`. Test sonunda yönetici bağlantısı oturumu `pg_terminate_backend` ile bitirir ve `DROP ROLE` yapar. (PG 10–18'in hepsinde `<insufficient privilege>` davranışı aynıdır.)

- [ ] **Adım 4: Testleri çalıştır, başarısızlığı gör**, ardından `types.rs` + `queries.rs`'i uygula: her fonksiyon `client.query_one`/`query` ile sabit SQL'i (`include_str!("sql/…")`) çalıştırır, `Error::from_query` ile hata çevirir, sütunları **adıyla** okur (`row.get("xact_commit")`). `activity` içinde `r.query.filter(|q| q != "<insufficient privilege>")`.

- [ ] **Adım 5: Doğrula.** `cargo test -p pgt-dash` (Docker'sız: derlenir, testler atlanır); Docker'lı: tek bir geçici PG 18 ile `PGT_TEST_URLS` ayarlı komutla `cargo test -p pgt-dash`, sonra `scripts/test-matrix.ps1 -Versions 10,13,18` (PG 10 `leader_pid` varyantını, PG 13+ ise öteki dosyayı sınar). `cargo clippy -p pgt-dash -- -D warnings`.

- [ ] **Adım 6: Commit** (`feat(dash): pgt-dash iskeleti, örnek/genel bakış/oturum sorguları`). `src/ipc/bindings`'e eklenen yeni `Sample.ts`, `Overview.ts`, `ActivityRow.ts` dosyalarını ekle.

---

### Task 3: Kilitler, yapılandırma, hazır transaction'lar, sorgu istatistikleri

**Dosyalar:**
- Oluştur: `sql/locks_sessions.sql`, `sql/locks.sql`, `sql/settings.sql`, `sql/prepared.sql`, `sql/statements_ext.sql`, `sql/statements.sql` (PG 10–12), `sql/statements.pg13.sql`
- Değiştir: `types.rs`, `queries.rs`
- Test: `crates/pgt-dash/tests/queries_more.rs`

**Arayüzler:**
- Üretir (`types.rs`):

```rust
pub struct BlockSession {            // engelleme ilişkisine giren oturum (bekleyen ∪ engelleyen)
    pub pid: i32, pub user: Option<String>, pub application: String, pub state: Option<String>,
    pub query: Option<String>,       // ≤ 1024 karakter
    pub blocked_by: Vec<i32>,        // pg_blocking_pids
    #[ts(type = "number | null")] pub wait_secs: Option<f64>,   // state_change'ten bu yana (bekleyen için)
}
pub struct LockRow {
    pub pid: i32, pub locktype: String, pub mode: String, pub granted: bool,
    pub object: Option<String>,      // yalnızca bağlı veritabanındaki ilişki için regclass metni
    pub database_oid: Option<u32>,
}
pub struct LockReport { pub sessions: Vec<BlockSession>, pub locks: Vec<LockRow>, pub truncated: bool }
pub struct Setting {
    pub name: String, pub value: Option<String>, pub unit: Option<String>, pub category: String,
    pub description: String, pub source: String, pub context: String,
    pub boot_value: Option<String>, pub pending_restart: bool,
}
pub struct PreparedXact {
    pub gid: String, pub transaction: String, pub owner: String, pub database: String,
    #[ts(type = "number")] pub age_secs: f64,
}
pub enum StatementsOrder { TotalTime, MeanTime, Calls }          // serde snake_case
pub struct StatementRow {
    pub query_id: Option<String>, pub query: Option<String>,      // ≤ 2048
    #[ts(type = "number")] pub calls: i64, #[ts(type = "number")] pub total_ms: f64,
    #[ts(type = "number")] pub mean_ms: f64, #[ts(type = "number")] pub rows: i64,
    #[ts(type = "number")] pub shared_blks_hit: i64, #[ts(type = "number")] pub shared_blks_read: i64,
}
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Statements { Available { rows: Vec<StatementRow> }, Unavailable { reason: String } }
```

- Üretir (`queries.rs`): `locks(c, db) -> LockReport`, `settings(c) -> Vec<Setting>`, `prepared(c) -> Vec<PreparedXact>`, `statements(c, version_num, db, order) -> Statements`.

- [ ] **Adım 1: SQL dosyaları (tam metin).**

`sql/locks_sessions.sql` (`$1` veritabanı adı/NULL):

```sql
WITH b AS (
  SELECT a.pid, pg_blocking_pids(a.pid) AS blocked_by
  FROM pg_catalog.pg_stat_activity a
  WHERE a.pid <> pg_backend_pid() AND ($1::text IS NULL OR a.datname = $1::text)
), involved AS (
  SELECT pid FROM b WHERE cardinality(blocked_by) > 0
  UNION
  SELECT DISTINCT unnest(blocked_by) FROM b WHERE cardinality(blocked_by) > 0
)
SELECT a.pid::int4, a.usename::text AS "user", a.application_name::text AS application, a.state::text AS state,
       left(a.query, 1024)::text AS query,
       pg_blocking_pids(a.pid)::int4[] AS blocked_by,
       extract(epoch FROM now() - a.state_change)::float8 AS wait_secs
FROM pg_catalog.pg_stat_activity a
JOIN involved i ON i.pid = a.pid
ORDER BY a.pid
```

`sql/locks.sql` (`$1` veritabanı adı/NULL; son satır sınırı 1001 → `truncated`):

```sql
SELECT l.pid::int4, l.locktype::text, l.mode::text, l.granted,
       CASE WHEN l.relation IS NOT NULL
             AND l.database = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
            THEN l.relation::regclass::text END AS object,
       l.database::int8 AS database_oid
FROM pg_catalog.pg_locks l
JOIN pg_catalog.pg_stat_activity a ON a.pid = l.pid
WHERE l.pid <> pg_backend_pid() AND ($1::text IS NULL OR a.datname = $1::text)
ORDER BY l.granted, l.pid, l.locktype, l.mode
LIMIT 1001
```

`sql/settings.sql`:

```sql
SELECT name::text, setting::text AS value, unit::text, category::text, short_desc::text AS description,
       source::text, context::text, boot_val::text AS boot_value, pending_restart
FROM pg_catalog.pg_settings
ORDER BY name
```

`sql/prepared.sql`:

```sql
SELECT gid::text, transaction::text AS transaction, owner::text, database::text,
       extract(epoch FROM now() - prepared)::float8 AS age_secs
FROM pg_catalog.pg_prepared_xacts
ORDER BY prepared
```

`sql/statements_ext.sql`: `SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_stat_statements'`.

`sql/statements.pg13.sql` (PG 13+). PG 10–12 dosyası `total_exec_time`→`total_time`, `mean_exec_time`→`mean_time` ile aynıdır. **Sıralama sütunu SQL'e `{ORDER}` yer tutucusu olarak yazılmaz**; üç ayrı sabit dosya yerine `queries.rs` içinde `match order { TotalTime => "total_exec_time DESC", MeanTime => "mean_exec_time DESC", Calls => "calls DESC" }` sabit değeri `ORDER BY` bölümüne `format!` ile eklenir (değer yalnızca bu üç sabit dizeden biridir; kullanıcı metni asla girmez):

```sql
SELECT queryid::text AS query_id, left(query, 2048)::text AS query, calls::int8,
       total_exec_time::float8 AS total_ms, mean_exec_time::float8 AS mean_ms, rows::int8,
       shared_blks_hit::int8, shared_blks_read::int8
FROM pg_stat_statements
WHERE $1::text IS NULL OR dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = $1::text)
-- ORDER BY ve LIMIT 50 kodda eklenir
```

- [ ] **Adım 2: Başarısız testleri yaz** (`tests/queries_more.rs`):

```rust
#[tokio::test]
async fn blocking_chain_is_reported() {
    for s in servers() {
        // 1) admin ayrı DB'de t(id int) tablosu, oturum A: BEGIN; LOCK TABLE t IN ACCESS EXCLUSIVE MODE;
        // 2) oturum B (başka admin_client): tokio::spawn(client_b.batch_execute("SELECT * FROM t"))  -> bekler
        // 3) kısa bekleme döngüsü (≤5 sn): dash locks() içinde blocked_by dolu bir oturum görünene kadar
        let report = wait_for_block(&dash, &db).await;
        let blocked = report.sessions.iter().find(|x| !x.blocked_by.is_empty()).expect("bekleyen oturum");
        assert_eq!(blocked.blocked_by, vec![pid_a]);
        assert!(report.sessions.iter().any(|x| x.pid == pid_a && x.blocked_by.is_empty()), "kök engelleyici listede");
        assert!(report.locks.iter().any(|l| !l.granted && l.object.as_deref() == Some("t")));
        assert!(!report.truncated);
        // temizlik: A ROLLBACK, B biter, DB düşer (drop_database)
    }
}

#[tokio::test]
async fn settings_and_prepared_xacts() {
    for s in servers() {
        let c = dash_conn(&s).await;
        let all = queries::settings(&c.client).await.unwrap();
        assert!(all.len() > 200 && all.iter().any(|x| x.name == "max_connections"));
        // prepared: admin 'BEGIN; PREPARE TRANSACTION ...' ancak max_prepared_transactions=0 ise atla.
        let n: i64 = admin.query_one("SELECT current_setting('max_prepared_transactions')::int8", &[]).await.unwrap().get(0);
        if n > 0 {
            // gid = 'pgt_dash_test'; sonra queries::prepared(...) gid'i içerir; ROLLBACK PREPARED ile temizle
        }
        assert!(queries::prepared(&c.client).await.is_ok());
    }
}

#[tokio::test]
async fn statements_unavailable_without_extension() {
    for s in servers() {
        let c = dash_conn(&s).await;   // bakım DB'sinde eklenti yok
        match queries::statements(&c.client, c.info.version_num, None, StatementsOrder::TotalTime).await.unwrap() {
            Statements::Unavailable { reason } => assert!(!reason.is_empty()),
            Statements::Available { .. } => panic!("eklenti kurulu değil"),
        }
    }
}
```

`statements_available_with_extension`: Docker'daki test sunucusunda `shared_preload_libraries` kapalı olduğundan `CREATE EXTENSION pg_stat_statements` başarılı olur ama görünüm 55000 verir → bu durum da `Unavailable` (neden: "yüklenmemiş") olmalıdır; testi bu iki durumu kapsayacak şekilde yaz: eklentiyi (contrib kapsayıcıda vardır) oluşturup sonucun `Unavailable` ve nedeninin "yüklenmemiş" ifadesini içermesini doğrula, `DROP EXTENSION` ile temizle. Gerçek `Available` yolunu görmek için **bir kez elle** `postgres -c shared_preload_libraries=pg_stat_statements` ile çalışan geçici PG 18 kapsayıcısında ve (`statements.sql`'i sınamak için) bir PG 12 kapsayıcısında `Available` + `rows` doğrulanır; bu iki elle kontrol görev raporuna yazılır (kapsayıcılar silinir). Matris betiği bu bayrağı bilmediği için otomatik teste dönüştürme bu planın kapsamında değildir.

- [ ] **Adım 3: Uygula.** `queries.rs`:
  - `locks`: iki sorgu (`locks_sessions.sql`, `locks.sql`); `truncated = locks.len() > 1000` ise son satır atılır.
  - `statements`: önce `statements_ext.sql` (`query_opt`): `None` → `Unavailable{reason: "pg_stat_statements eklentisi bu veritabanında kurulu değil."}`. Sorgu `55000` ya da `42P01` ile hata verirse (`Error::Query{sqlstate}`) → `Unavailable{reason: "pg_stat_statements yüklenmemiş (shared_preload_libraries)."}`; başka hata aynen döner. Sürüm eşiği 13 (`pick`). `ORDER BY` + `LIMIT 50` eklenmesi yukarıdaki sabit eşleme ile.
  - Yetkisiz `pg_stat_statements` satırlarında `query` `<insufficient privilege>` gelir → `None`.
  - `settings`: `pending_restart` PG 10+'da vardır; doğrudan okunur.

- [ ] **Adım 4:** Docker'lı `cargo test -p pgt-dash` + `scripts/test-matrix.ps1 -Versions 10,12,13,18`; clippy temiz.

- [ ] **Adım 5: Commit** (`feat(dash): kilit/engelleme, ayarlar, hazır tx ve pg_stat_statements sorguları`), yeni bağlama dosyalarıyla.

---

### Task 4: `DashManager` — yaşam döngüsü, yeniden bağlanma, iptal/sonlandırma

**Dosyalar:**
- Oluştur: `crates/pgt-dash/src/manager.rs`
- Değiştir: `lib.rs` (`pub mod manager;` + yeniden dışa aktarım), `types.rs` (`SignalKind`, `SignalOutcome`, `DashInfo`)
- Test: `crates/pgt-dash/tests/manager.rs`

**Arayüzler:**
- Tüketir: `ConnectionManager::open_dedicated`, Görev 2–3'teki `queries::*`.
- Üretir:

```rust
pub type DashId = uuid::Uuid;
#[derive(Serialize, TS)] #[serde(rename_all = "camelCase")] #[ts(export)]
pub struct DashInfo { pub id: DashId, pub version_num: u32, pub self_pid: i32, pub database: Option<String> }
#[derive(Deserialize, TS)] #[serde(rename_all = "snake_case")] #[ts(export)]
pub enum SignalKind { Cancel, Terminate }
#[derive(Serialize, TS)] #[serde(rename_all = "snake_case")] #[ts(export)]
pub enum SignalOutcome { Sent, NotFound }

impl DashManager {
    pub fn new(manager: Arc<ConnectionManager>) -> Self;
    pub async fn open(&self, server: Uuid, database_oid: Option<u32>) -> Result<DashInfo>;
    pub fn close(&self, id: DashId);
    pub fn close_server(&self, server: Uuid);        // sunucu bağlantısı kesilince
    pub fn close_all(&self);
    pub async fn sample(&self, id: DashId) -> Result<Sample>;
    pub async fn overview(&self, id: DashId) -> Result<Overview>;
    pub async fn activity(&self, id: DashId) -> Result<Vec<ActivityRow>>;
    pub async fn locks(&self, id: DashId) -> Result<LockReport>;
    pub async fn settings(&self, id: DashId) -> Result<Vec<Setting>>;
    pub async fn prepared(&self, id: DashId) -> Result<Vec<PreparedXact>>;
    pub async fn statements(&self, id: DashId, order: StatementsOrder) -> Result<Statements>;
    pub async fn signal(&self, id: DashId, pid: i32, kind: SignalKind) -> Result<SignalOutcome>;
}
```

**Tasarım:** `DashSession { server: Uuid, target: Target, db_name: Option<String>, conn: tokio::sync::Mutex<Option<DedicatedConn>>, self_pid: AtomicI32, version_num: AtomicU32 }`. Tüm sorgular tek yardımcıdan geçer: `with_conn(|client, version| async { … })` — kilidi alır, `conn` `None` ya da `client.is_closed()` ise `manager.open_dedicated` ile yeniden açar (açılamazsa hata döner, oturum kayıtlı kalır), `self_pid`'i `SELECT pg_backend_pid()` ile yeniler (yeniden açıldığında değişir), ardından sorguyu çalıştırır; sorgu "bağlantı kapandı" tipi hatayla biterse `conn`'u `None` yapıp **bir kez** yeniden dener. Sorguların sürekli çakışmaması için `Mutex` zaten tek uçuşu zorlar. `database_oid = Some(oid)` ise `database` adı `open` sırasında bağlantı kurulduktan sonra `SELECT current_database()` ile okunur (süzgeç olarak `Some(ad)` kullanılır); `None` ise süzgeç `None`. `DedicatedConn.notices` alıcısı **düşürülür** (alıcı olmayan kanala gönderim hatası sürücüde yok sayılmalı; `connect_dedicated_once` içindeki gönderimi kontrol et, panik/log seli yapıyorsa alıcıyı bir görevde boşaltacak şekilde düzelt). `signal`: `pid == self_pid` → `Error::Invalid("Kendi dashboard oturumunuzu durduramazsınız.")`; aksi halde `SELECT pg_cancel_backend($1)` / `SELECT pg_terminate_backend($1)` (iki sabit sorgu); `true → Sent`, `false → NotFound`; 42501 hatası `Error::from_query` ile `Error::Query` olarak döner (arayüz "yetkiniz yok" gösterir). `signal` başarısızlığı bağlantıyı bozmaz.

- [ ] **Adım 1: Başarısız testler** (`tests/manager.rs`):

```rust
#[tokio::test]
async fn open_query_close_roundtrip() {
    for s in servers() {
        let (cm, id) = connected(&s).await;               // ConnectionManager + sunucu kimliği
        let dm = DashManager::new(cm.clone());
        let info = dm.open(id, None).await.unwrap();
        assert!(info.self_pid > 0 && info.database.is_none());
        assert!(dm.sample(info.id).await.is_ok() && dm.activity(info.id).await.unwrap().iter().any(|r| r.is_self));
        dm.close(info.id);
        assert!(dm.sample(info.id).await.is_err());       // kapalı kimlik: NotFound
    }
}

#[tokio::test]
async fn database_scope_filters_activity() {
    // admin ile geçici DB 'pgt_dash_scope' oluştur, oid'ini al; dm.open(id, Some(oid));
    // info.database == Some("pgt_dash_scope"); activity() satırlarının hepsi database == Some(...).
}

#[tokio::test]
async fn cancel_and_terminate_real_sessions() {
    for s in servers() {
        // admin_client A: spawn(a.batch_execute("SELECT pg_sleep(30)")); pid_a = bağlantı pid'i (ayrı sorguyla alınır)
        // dash.signal(pid_a, Cancel) -> Sent; A'nın sorgusu 57014 ile biter, oturum yaşar (A hâlâ sorgu atabilir)
        // admin_client B: dash.signal(pid_b, Terminate) -> Sent; B artık sorgu atamaz (client.is_closed() kısa sürede true)
        // dash.signal(2_000_000_000, Cancel) -> NotFound
        // dash.signal(info.self_pid, Terminate) -> Err(Invalid)
    }
}

#[tokio::test]
async fn signal_without_privilege_is_an_error_not_silence() {
    // Yetkisiz rol (pg_signal_backend üyesi değil) ile dash; başka rolün oturumuna Cancel -> Err(Query{sqlstate: Some("42501")}).
}

#[tokio::test]
async fn reconnects_after_the_dash_connection_is_killed() {
    for s in servers() {
        // dm.open; self_pid al; admin ile pg_terminate_backend(self_pid);
        // ardından (kısa döngü, ≤3 sn) dm.sample(id) başarılı olmalı ve info'dan farklı yeni self_pid ile çalışmalı.
    }
}
```

- [ ] **Adım 2: Çalıştır, başarısızlığı gör; uygula; geçir.** Kimlikler için `Mutex<HashMap<DashId, Arc<DashSession>>>` (std Mutex, kilit asla `.await` boyunca tutulmaz: `Arc` klonlanıp bırakılır). `close`/`close_server`/`close_all` yalnızca haritadan düşürür (bağlantı `Drop` ile kapanır). `close_server`: `session.server == server` olanları.

- [ ] **Adım 3:** Docker'lı `cargo test -p pgt-dash`, `scripts/test-matrix.ps1 -Versions 10,14,18`; clippy temiz.

- [ ] **Adım 4: Commit** (`feat(dash): DashManager (yaşam döngüsü, yeniden bağlanma, iptal/sonlandırma)`).

---

### Task 5: Tauri `dash_*` komutları ve entegrasyon

**Dosyalar:**
- Oluştur: `src-tauri/src/dash.rs`
- Değiştir: `src-tauri/Cargo.toml` (`pgt-dash` bağımlılığı), `src-tauri/src/state.rs` (`dash: Arc<DashManager>`), `src-tauri/src/main.rs` (modül, `invoke_handler` kaydı, yeniden yüklemede ve çıkışta kapatma), `src-tauri/src/commands.rs` (`disconnect_and_close` → `dash.close_server`)
- Test: `src-tauri/src/dash.rs` içindeki birim testleri + `commands.rs`'teki mevcut `disconnect_server_closes_query_sessions` kalıbında yeni bir test

**Arayüzler:**
- Tüketir: `DashManager` (Görev 4).
- Üretir (komut imzaları, ön yüz Plan 9'da bunları `commands.ts`'e ekler):

```rust
#[tauri::command] pub async fn dash_open(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>) -> CmdResult<DashInfo>;
#[tauri::command] pub async fn dash_close(state: State<'_, AppState>, id: Uuid) -> CmdResult<()>;
#[tauri::command] pub async fn dash_sample(state: State<'_, AppState>, id: Uuid) -> CmdResult<Sample>;
#[tauri::command] pub async fn dash_overview(state: State<'_, AppState>, id: Uuid) -> CmdResult<Overview>;
#[tauri::command] pub async fn dash_activity(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<ActivityRow>>;
#[tauri::command] pub async fn dash_locks(state: State<'_, AppState>, id: Uuid) -> CmdResult<LockReport>;
#[tauri::command] pub async fn dash_settings(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<Setting>>;
#[tauri::command] pub async fn dash_prepared(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<PreparedXact>>;
#[tauri::command] pub async fn dash_statements(state: State<'_, AppState>, id: Uuid, order: StatementsOrder) -> CmdResult<Statements>;
#[tauri::command] pub async fn dash_signal(state: State<'_, AppState>, id: Uuid, pid: i32, kind: SignalKind) -> CmdResult<SignalOutcome>;
```

Her gövde `Box::pin(state.dash.<yöntem>(…)).await.map_err(ErrorPayload::from)` (commands.rs'teki `CmdResult`/hata dönüşümünü aynen izle).

- [ ] **Adım 1: Başarısız testleri yaz.** (a) `state.rs`: `AppState` kurulunca `dash` boş (`close_all` panik etmez). (b) `commands.rs` kalıbında: sunucu bağlantısı kesilince o sunucunun dashboard oturumları kapanır (Docker'lı; `PGT_TEST_URLS` yoksa atlanır): `dash_open` → `disconnect_server` → `dash_sample` hata verir. (c) `dash.rs`: `dash_signal` kendi pid'ine `Invalid` kodunu verir (mevcut `ErrorPayload.code` = `"invalid"`).

- [ ] **Adım 2: Uygula.** `main.rs`: yeniden yükleme kancasında (`close_all_on_reload` çağrıldığı yer, satır ~31) `dash.close_all()` de çağrılır (webview yenilenince arayüzdeki kimlikler kaybolur; sahipsiz bağlantı kalmasın). Çıkış kancasında (`RunEvent::Exit`) `dash.close_all()`. `disconnect_and_close` (commands.rs ~71): `dash.close_server(id)`; imzaya `DashManager` eklenir ve tüm çağrı yerleri/testler güncellenir. Komutlar `invoke_handler`'a (`query::…` satırlarının altına) eklenir.

- [ ] **Adım 3: Doğrula.** `cargo test --workspace` (Docker'sız temiz; atlananlar sayılır), `cargo test -p pg-tools` bin testleri, `cargo clippy --workspace --all-targets -- -D warnings`, `bunx tsc --noEmit` (ts-rs bağlamalarıyla ön yüz tipleri bozulmamalı), `git checkout -- src/ipc/bindings` yalnızca satır sonu değişiklikleri için (yeni `.ts` dosyaları tutulur). Docker'lı: tek PG 18 ile workspace testleri.

- [ ] **Adım 4: Elle duman testi (release değil, geliştirme exe'siyle gerekmez):** bu planda ön yüz yok; doğrulama Docker'lı testlerle yapılır. Duman testi Plan 9'da.

- [ ] **Adım 5: Commit** (`feat(app): dash_* komutları; yeniden yükleme/çıkış/bağlantı kesmede dashboard oturumlarını kapat`).

---

## Kapanış kontrolü (Plan 8 bitişi)

- Matris: `scripts/test-matrix.ps1` PG 10–18 tümü geçer (kapsayıcılar silinir).
- `grep` kontrolü: `crates/pgt-dash` içinde `tracing::` çağrıları sorgu metni / parola / URL içermez.
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla** yapılır.
