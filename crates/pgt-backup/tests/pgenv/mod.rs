//! Docker'lı testlerin ortak ortamı: `PGT_TEST_URLS` sunucuları ve bulunan istemci araçları; örnek veritabanı ve
//! anlık görüntü. Adres ve parola hiçbir iletide yazılmaz.
#![allow(dead_code)]

use std::path::Path;

use crate::common::VecSink;
use pgt_backup::{
    BackupOptions, BackupRequest, BackupScope, CancelToken, Conn, DumpFormat, JobKind, JobOutcome, JobSummary, RestoreOptions,
    RestoreRequest, RestoreTarget, Secret, ToolKind, ToolSet, backup_invocation, restore_invocation, run, sniff_path, tools,
};
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use pgt_ddl::Risk;

pub struct Env {
    pub ts: TestServer,
    pub version: u32,
    pub dump: ToolSet,
    pub restore: ToolSet,
    pub psql: ToolSet,
    pub sets: Vec<ToolSet>,
}

/// Sunucu ve araç ortamları; eksikse neden atlandığını yazar.
pub async fn envs(test: &str) -> Vec<Env> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("{test}: PGT_TEST_URLS tanımlı değil, test atlanıyor");
        return Vec::new();
    }
    let setting = std::env::var("PGT_TEST_PGBIN").ok().filter(|s| !s.trim().is_empty());
    let sets = tools::detect(setting.as_deref()).await.sets;
    if !sets.iter().any(|s| s.pg_dump) || !sets.iter().any(|s| s.pg_restore) || !sets.iter().any(|s| s.psql) {
        eprintln!("{test}: PostgreSQL istemci araçları bulunamadı (PGT_TEST_PGBIN), test atlanıyor");
        return Vec::new();
    }
    let mut out = Vec::new();
    for ts in servers {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let v: String = admin.query_one("SHOW server_version_num", &[]).await.unwrap().get(0);
        let version: u32 = v.parse().unwrap();
        let dump = match tools::choose(&sets, ToolKind::PgDump, version) {
            Ok(t) => t.clone(),
            Err(m) => {
                // Sürüm uyumsuzluğunun gerçek hâli: sunucu araçtan yeni (saf testte de kapsanır).
                eprintln!("{test}: sunucu {version} için uygun pg_dump yok, atlanıyor: {m}");
                continue;
            }
        };
        // Uygulamayla aynı seçim: arşivi bu `pg_dump` üretir.
        let restore = tools::choose_restore(&sets, version, Some(dump.version.version_num)).unwrap().clone();
        let psql = tools::choose(&sets, ToolKind::Psql, 0).unwrap().clone();
        out.push(Env { ts, version, dump, restore, psql, sets: sets.clone() });
    }
    out
}

impl Env {
    pub async fn admin(&self) -> tokio_postgres::Client {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await
    }

    pub async fn db(&self, name: &str) -> tokio_postgres::Client {
        admin_client(&self.ts, name).await
    }

    pub fn conn(&self, db: &str) -> Conn {
        Conn::from_server(&self.ts.config, db, self.dump.version.version_num)
    }

    pub fn secret(&self) -> Option<Secret> {
        self.ts.password.clone().map(Secret::new)
    }

    /// Boş veritabanı (varsa silinip yeniden oluşturulur).
    pub async fn fresh(&self, name: &str) {
        let admin = self.admin().await;
        drop_database(&admin, name).await;
        admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
    }

    pub async fn drop(&self, name: &str) {
        drop_database(&self.admin().await, name).await;
    }

    pub async fn backup(&self, db: &str, scope: BackupScope, format: DumpFormat, out: &Path) -> (JobSummary, VecSink) {
        let req = BackupRequest { scope, format, options: BackupOptions::default(), output_path: out.to_string_lossy().into_owned() };
        let inv = backup_invocation(&self.dump, &self.conn(db), &req).unwrap();
        assert_eq!(inv.risk, Risk::Safe);
        let mut sink = VecSink::default();
        let s = run(inv, self.secret(), JobKind::Backup, None, &mut sink, CancelToken::new()).await;
        (s, sink)
    }

    pub async fn restore(&self, db: &str, path: &Path, options: RestoreOptions) -> (JobSummary, VecSink, Risk) {
        let path = path.to_string_lossy().into_owned();
        let (format, _) = sniff_path(&path).unwrap();
        let req = RestoreRequest { path, target: RestoreTarget::Existing, options };
        let inv =
            restore_invocation((&self.restore, &self.psql), &self.conn(db), &self.ts.config.maintenance_db, &req, format).unwrap();
        let risk = inv.risk;
        let mut sink = VecSink::default();
        let s = run(inv, self.secret(), JobKind::Restore, None, &mut sink, CancelToken::new()).await;
        (s, sink, risk)
    }
}

/// PG 10 uyumlu örnek: iki şema, Unicode/tırnaklı ad, bölümlenmiş tablo ve MV.
pub const FIXTURE: &str = r#"
CREATE SCHEMA app;
CREATE SCHEMA other;
CREATE TABLE app.a (id int PRIMARY KEY, t text);
INSERT INTO app.a SELECT g, 'x' || g FROM generate_series(1, 100) g;
CREATE TABLE app."Büyük Ad" (id int);
INSERT INTO app."Büyük Ad" SELECT generate_series(1, 7);
CREATE TABLE other.b (id int);
INSERT INTO other.b SELECT generate_series(1, 5);
CREATE TABLE app.p (id int, k int) PARTITION BY RANGE (k);
CREATE TABLE app.p1 PARTITION OF app.p FOR VALUES FROM (0) TO (10);
INSERT INTO app.p SELECT g, g % 10 FROM generate_series(1, 20) g;
CREATE MATERIALIZED VIEW app.mv AS SELECT count(*) AS c FROM app.a;
"#;

/// Uygulamanın ilerleme toplamı sorgusuyla aynı süzgeç (`relkind = 'r'`, sistem şemaları ve yapılandırma tablosu
/// olmayan uzantı üyeleri hariç; `src-tauri/src/backup.rs` `count_base!`). 10–18'de sözdizimi de böylece sınanır.
pub const TABLE_COUNT: &str = "SELECT count(*)::int4 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind = 'r' AND n.nspname <> 'information_schema' AND n.nspname !~ '^pg_' \
     AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid = d.refobjid \
     WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = c.oid \
     AND d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype = 'e' \
     AND NOT COALESCE(c.oid = ANY (e.extconfig), false))";

pub async fn fixture(env: &Env, name: &str) {
    env.fresh(name).await;
    env.db(name).await.batch_execute(FIXTURE).await.unwrap();
}

/// Tablo ve MV başına satır sayıları (ada göre sıralı).
pub async fn snapshot(env: &Env, db: &str) -> Vec<(String, i64)> {
    let c = env.db(db).await;
    let rels = c
        .query(
            "SELECT format('%I.%I', n.nspname, c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'm') AND n.nspname <> 'information_schema' AND n.nspname !~ '^pg_' ORDER BY 1",
            &[],
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    for r in rels {
        let name: String = r.get(0);
        let n: i64 = c.query_one(&format!("SELECT count(*) FROM {name}"), &[]).await.unwrap().get(0);
        out.push((name, n));
    }
    out
}

pub fn last_done(sink: &VecSink) -> u32 {
    sink.progress().last().map(|p| p.0).unwrap_or(0)
}

pub fn ok(s: &JobSummary, sink: &VecSink, what: &str) {
    assert_eq!(s.outcome, JobOutcome::Success, "{what}: {:?}", sink.lines());
    assert_eq!(s.errors, 0, "{what}: {:?}", sink.lines());
}

impl Env {
    /// 17+ araç `SET transaction_timeout = 0` yazar (plain çıktıda pg_dump, arşivde pg_restore); 17'den eski sunucu
    /// bunu tanımaz (ölçüldü). Bu sunucularda pg_restore "hatada dur" kapalı çalışır ve yalnız bu satırı atlar;
    /// psql ise ON_ERROR_STOP ile ilk satırda durur.
    pub fn old_server(&self) -> bool {
        self.version < 170_000 && self.restore.version.version_num >= 170_000
    }

    pub fn restore_opts(&self, mut o: RestoreOptions) -> RestoreOptions {
        if self.old_server() {
            o.exit_on_error = false;
        }
        o
    }
}

/// Geri yükleme sonucu: yeni sunucuda temiz başarı; eski sunucuda yalnız `transaction_timeout` hatası,
/// `completed_with_errors` (pg_restore hatayı atlayıp 1 ile çıkar) ve ipucu.
pub fn restored(env: &Env, s: &JobSummary, sink: &VecSink, what: &str) {
    if !env.old_server() {
        return ok(s, sink, what);
    }
    let errors: Vec<String> = sink.lines().into_iter().filter(|(sev, _)| *sev == pgt_backup::LineSeverity::Error).map(|(_, t)| t).collect();
    assert!(!errors.is_empty() && errors.iter().all(|t| t.contains("transaction_timeout")), "{what}: {:?}", sink.lines());
    assert_eq!(s.outcome, JobOutcome::CompletedWithErrors, "{what}: {s:?}");
    assert!(s.hint.as_deref().is_some_and(|h| h.contains("transaction_timeout")), "{what}: {s:?}");
}
