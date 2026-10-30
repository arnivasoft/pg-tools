//! Gerçek `pg_dump`/`pg_restore`/`psql` ile Docker'lı gidiş-dönüş testleri (spec §4). `PGT_TEST_URLS` (sürüm
//! matrisi) ve istemci araçları gerekir: `PGT_TEST_PGBIN` ya da otomatik bulma (`C:\Program Files\PostgreSQL\18\bin`).
//! Biri yoksa test bunu yazar ve atlanır. Yedek dosyaları geçici klasöre yazılır, test sonunda silinir.
//! Adres ve parola hiçbir iletide yazılmaz.

mod common;
mod pgenv;

use std::path::Path;
use std::time::{Duration, Instant};

use common::VecSink;
use pgenv::{Env, TABLE_COUNT, envs, fixture, last_done, ok, restored, snapshot};
use pgt_backup::{
    BackupEvent, BackupOptions, BackupRequest, BackupScope, CancelToken, DumpFormat, JobKind, JobOutcome, PARTIAL_SUFFIX,
    RestoreOptions, Secret, SourceFormat, backup_invocation, inspect, run, sniff_path,
};
use pgt_ddl::Risk;

#[tokio::test]
async fn every_format_roundtrips_into_a_new_database() {
    for env in envs("every_format_roundtrips").await {
        let src = "pgt_bk_fmt";
        fixture(&env, src).await;
        let want = snapshot(&env, src).await;
        assert_eq!(want.len(), 5, "{want:?}");
        let tables: i32 = env.db(src).await.query_one(TABLE_COUNT, &[]).await.unwrap().get(0);
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (DumpFormat::Custom, "a.dump", SourceFormat::Custom),
            (DumpFormat::Plain, "a.sql", SourceFormat::Plain),
            (DumpFormat::Directory, "a.dir", SourceFormat::Directory),
            (DumpFormat::Tar, "a.tar", SourceFormat::Tar),
        ];
        for (format, file, source) in cases {
            let what = format!("{} {format:?}", env.version);
            let out = dir.path().join(file);
            let (s, sink) = env.backup(src, BackupScope::Database, format, &out).await;
            ok(&s, &sink, &what);
            assert!(out.exists() && !Path::new(&format!("{}{PARTIAL_SUFFIX}", out.display())).exists(), "{what}");
            // İlerleme toplamı (relkind = 'r') pg_dump'ın "dumping contents of table" satırlarıyla aynı sayıdadır.
            assert_eq!(last_done(&sink), tables as u32, "{what}: {:?}", sink.progress());
            assert_eq!(sniff_path(&out.to_string_lossy()).unwrap().0, source, "{what}");
            let src_info = inspect(&out.to_string_lossy(), &env.sets).await.unwrap();
            assert_eq!(src_info.format, source);
            if source == SourceFormat::Plain {
                assert!(src_info.archive.is_none());
            } else {
                let a = src_info.archive.unwrap();
                assert_eq!(a.db_name.as_deref(), Some(src), "{what}");
                assert!(a.items > 0 && a.counts.iter().any(|(k, _)| k == "TABLE DATA"), "{what}: {a:?}");
            }

            let tgt = "pgt_bk_fmt_tgt";
            env.fresh(tgt).await;
            let opts = env.restore_opts(RestoreOptions { exit_on_error: source != SourceFormat::Plain, ..Default::default() });
            let (s, sink, risk) = env.restore(tgt, &out, opts).await;
            assert_eq!(risk, if source == SourceFormat::Plain { Risk::DataLoss } else { Risk::Destructive });
            if source == SourceFormat::Plain && env.old_server() {
                // psql ilk satırda (SET transaction_timeout) durur: net hata ve ipucu, içerik denetlenmez.
                assert_eq!(s.outcome, JobOutcome::Failed, "{what}");
                // psql ipucu düz SQL'i anlatır: yedeği aynı ana sürüm pg_dump ile yeniden alın ya da custom.
                assert!(s.hint.as_deref().is_some_and(|h| h.contains("transaction_timeout") && h.contains("custom")), "{what}: {s:?}");
                eprintln!("{what}: sunucu 17'den eski, plain geri yükleme araç 18 ile yapılamıyor (ipucu doğrulandı)");
                env.drop(tgt).await;
                continue;
            }
            restored(&env, &s, &sink, &format!("{what} restore"));
            if source != SourceFormat::Plain {
                assert!(last_done(&sink) > 0, "{what}: {:?}", sink.progress());
            }
            assert_eq!(snapshot(&env, tgt).await, want, "{what}");
            env.drop(tgt).await;
        }
        env.drop(src).await;
    }
}

#[tokio::test]
async fn schema_and_table_scope() {
    for env in envs("schema_and_table_scope").await {
        let src = "pgt_bk_scope";
        fixture(&env, src).await;
        let all = snapshot(&env, src).await;
        let dir = tempfile::tempdir().unwrap();

        let out = dir.path().join("schema.dump");
        let (s, sink) = env.backup(src, BackupScope::Schema { name: "app".into() }, DumpFormat::Custom, &out).await;
        ok(&s, &sink, "şema");
        assert_eq!(last_done(&sink), 3, "{:?}", sink.progress());
        let tgt = "pgt_bk_scope_tgt";
        env.fresh(tgt).await;
        let (s, sink, _) = env.restore(tgt, &out, env.restore_opts(RestoreOptions { exit_on_error: true, ..Default::default() })).await;
        restored(&env, &s, &sink, "şema geri yükleme");
        let want: Vec<_> = all.iter().filter(|(n, _)| n.starts_with("app.")).cloned().collect();
        assert_eq!(snapshot(&env, tgt).await, want, "{}", env.version);

        // Tablo kapsamı: tırnaklı Unicode ad tam eşleşir; şema arşivde yoktur, hedefte önceden olmalı.
        let out = dir.path().join("table.dump");
        let scope = BackupScope::Table { schema: "app".into(), name: "Büyük Ad".into() };
        let (s, sink) = env.backup(src, scope, DumpFormat::Custom, &out).await;
        ok(&s, &sink, "tablo");
        assert_eq!(last_done(&sink), 1);
        env.fresh(tgt).await;
        env.db(tgt).await.batch_execute("CREATE SCHEMA app").await.unwrap();
        let (s, sink, _) = env.restore(tgt, &out, env.restore_opts(RestoreOptions { exit_on_error: true, ..Default::default() })).await;
        restored(&env, &s, &sink, "tablo geri yükleme");
        assert_eq!(snapshot(&env, tgt).await, vec![("app.\"Büyük Ad\"".to_string(), 7)]);
        env.drop(tgt).await;
        env.drop(src).await;
    }
}

#[tokio::test]
async fn clean_restore_replaces_existing_objects() {
    for env in envs("clean_restore").await {
        let src = "pgt_bk_clean";
        fixture(&env, src).await;
        let want = snapshot(&env, src).await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.dump");
        let (s, sink) = env.backup(src, BackupScope::Database, DumpFormat::Custom, &out).await;
        ok(&s, &sink, "yedek");
        let tgt = "pgt_bk_clean_tgt";
        env.fresh(tgt).await;
        let (s, sink, _) = env.restore(tgt, &out, env.restore_opts(RestoreOptions { exit_on_error: true, ..Default::default() })).await;
        restored(&env, &s, &sink, "ilk yükleme");
        env.db(tgt).await.batch_execute("DELETE FROM app.a WHERE id > 10; INSERT INTO other.b VALUES (99), (100);").await.unwrap();
        assert_ne!(snapshot(&env, tgt).await, want);
        let opts = env.restore_opts(RestoreOptions { clean: true, if_exists: true, exit_on_error: true, ..Default::default() });
        let (s, sink, risk) = env.restore(tgt, &out, opts).await;
        assert_eq!(risk, Risk::DataLoss);
        restored(&env, &s, &sink, "--clean");
        assert_eq!(snapshot(&env, tgt).await, want, "{}", env.version);
        env.drop(tgt).await;
        env.drop(src).await;
    }
}

#[tokio::test]
async fn plain_single_transaction_rolls_back_on_error() {
    for env in envs("plain_single_transaction").await {
        let tgt = "pgt_bk_plain_tx";
        let dir = tempfile::tempdir().unwrap();
        let sql = dir.path().join("hatali.sql");
        std::fs::write(&sql, "CREATE TABLE ok_t (id int);\nINSERT INTO ok_t VALUES (1);\nSELECT * FROM yok_tablo;\n").unwrap();
        let exists = |c: tokio_postgres::Client| async move {
            c.query_one("SELECT to_regclass('public.ok_t') IS NOT NULL", &[]).await.unwrap().get::<_, bool>(0)
        };
        for single in [true, false] {
            env.fresh(tgt).await;
            let (s, sink, risk) = env.restore(tgt, &sql, RestoreOptions { single_transaction: single, ..Default::default() }).await;
            assert_eq!(risk, Risk::DataLoss);
            assert_eq!(s.outcome, JobOutcome::Failed, "{:?}", sink.lines());
            assert!(s.errors >= 1 && s.exit_code.is_some_and(|c| c != 0), "{s:?}");
            // Tek transaction: hata her şeyi geri alır; değilse önceki ifadeler kalır (ON_ERROR_STOP ile durulur).
            assert_eq!(exists(env.db(tgt).await).await, !single, "single={single} {}", env.version);
        }
        env.drop(tgt).await;
    }
}

/// Bu testin pg_dump oturumları: `datname` ile sınırlıdır. Aynı ikilideki diğer testler aynı sunucuda eşzamanlı
/// yedek alır (`application_name` aynı); yalnız uygulama adına göre sayım onların oturumlarını da sayar (ölçüldü).
async fn backup_pids(env: &Env, db: &str, lock_wait_only: bool) -> Vec<i32> {
    env.admin()
        .await
        .query(
            "SELECT pid FROM pg_stat_activity WHERE application_name = 'pg-tools backup' AND datname = $1              AND (NOT $2 OR wait_event_type = 'Lock') ORDER BY pid",
            &[&db, &lock_wait_only],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

async fn alive(env: &Env, pids: &[i32]) -> Vec<i32> {
    env.admin()
        .await
        .query("SELECT pid FROM pg_stat_activity WHERE pid = ANY($1) ORDER BY pid", &[&pids])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

#[tokio::test]
async fn cancel_removes_partial_output_and_ends_the_tool() {
    for env in envs("cancel").await {
        let src = "pgt_bk_cancel";
        fixture(&env, src).await;
        let db = env.db(src).await;
        db.batch_execute("CREATE TABLE app.big AS SELECT g, repeat('x', 100) AS t FROM generate_series(1, 200000) g").await.unwrap();
        // Büyük tablo kilitli tutulur: pg_dump ACCESS SHARE kilidinde bekler, iş iptale dek bitmez (zamanlamadan bağımsız).
        let holder = env.db(src).await;
        let dir = tempfile::tempdir().unwrap();
        for (format, file) in [(DumpFormat::Custom, "big.dump"), (DumpFormat::Directory, "big.dir")] {
            let out = dir.path().join(file);
            let partial = format!("{}{PARTIAL_SUFFIX}", out.display());
            let req = BackupRequest {
                scope: BackupScope::Database,
                format,
                options: BackupOptions::default(),
                output_path: out.to_string_lossy().into_owned(),
            };
            let inv = backup_invocation(&env.dump, &env.conn(src), &req).unwrap();
            holder.batch_execute("BEGIN; LOCK TABLE app.big IN ACCESS EXCLUSIVE MODE").await.unwrap();
            let token = CancelToken::new();
            let t = token.clone();
            let p = partial.clone();
            let canceller = async {
                // Geçici çıktı oluşsun (custom/directory'de pg_dump onu bağlanmadan önce açar) ve pg_dump'ın sunucu
                // oturumu kilitte beklesin; sonra iptal et. Zamanlamadan bağımsızdır: iptal anında oturum kesin kilitte.
                let started = Instant::now();
                let mut pids = Vec::new();
                while started.elapsed() < Duration::from_secs(20) {
                    if Path::new(&p).exists() {
                        pids = backup_pids(&env, src, true).await;
                        if !pids.is_empty() {
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let seen = Path::new(&p).exists();
                t.cancel();
                (seen, pids)
            };
            let mut sink = VecSink::default();
            let started = Instant::now();
            let (s, (seen, pids)) = tokio::join!(run(inv, env.secret(), JobKind::Backup, None, &mut sink, token), canceller);
            assert!(seen, "geçici çıktı hiç oluşmadı: {:?}", sink.lines());
            assert!(!pids.is_empty(), "{format:?}: pg_dump oturumu kilitte beklemedi: {:?}", sink.lines());
            assert_eq!(s.outcome, JobOutcome::Cancelled, "{:?}", sink.lines());
            assert!(started.elapsed() < Duration::from_secs(30));
            assert!(!Path::new(&partial).exists() && !out.exists(), "{format:?}: geçici çıktı kaldı");
            assert!(matches!(sink.events.last(), Some(BackupEvent::Finished { .. })));
            // Kilitte bekleyen sunucu oturumu istemcinin gittiğini ancak kilit bırakılınca fark eder (ölçüldü: iptalden
            // sonra hâlâ kilitte, bırakılınca ilk yoklamada yok, 10–18). Kilit bırakılınca tam bu oturumlar kapanmalı ve
            // çıktı oluşmamalı: pg_dump süreci gerçekten sonlandı. 10 sn yalnız yavaş CI için paydır.
            holder.batch_execute("ROLLBACK").await.unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !alive(&env, &pids).await.is_empty() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert_eq!(alive(&env, &pids).await, Vec::<i32>::new(), "{format:?}: pg_dump oturumu kaldı");
            assert!(!Path::new(&partial).exists() && !out.exists(), "{format:?}: iptalden sonra çıktı oluştu");
        }
        drop((db, holder));
        env.drop(src).await;
    }
}

#[tokio::test]
async fn wrong_password_gives_hint() {
    for env in envs("wrong_password").await {
        if env.ts.password.is_none() {
            eprintln!("wrong_password: test sunucusu parolasız, atlanıyor");
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.dump");
        let req = BackupRequest {
            scope: BackupScope::Database,
            format: DumpFormat::Custom,
            options: BackupOptions::default(),
            output_path: out.to_string_lossy().into_owned(),
        };
        let inv = backup_invocation(&env.dump, &env.conn(&env.ts.config.maintenance_db), &req).unwrap();
        let mut sink = VecSink::default();
        let wrong = "yanlis-parola-0xBEEF";
        let s = run(inv, Some(Secret::new(wrong.into())), JobKind::Backup, None, &mut sink, CancelToken::new()).await;
        assert_eq!(s.outcome, JobOutcome::Failed, "{:?}", sink.lines());
        assert!(s.hint.as_deref().is_some_and(|h| h.contains("Parola")), "{s:?}");
        assert!(sink.lines().iter().all(|(_, t)| !t.contains(wrong)));
        assert!(!out.exists() && !Path::new(&format!("{}{PARTIAL_SUFFIX}", out.display())).exists());
    }
}

/// Windows'ta araçlar ortam değerlerini ANSI kod sayfasına çevirir: ASCII dışı veritabanı adına bağlanılamaz.
/// Uygulama bunu plan hatası yapar (`windows_name_problems`); bu test o kararın gerekçesini ölçer.
#[cfg(windows)]
#[tokio::test]
async fn non_ascii_database_name_cannot_be_used_by_tools_on_windows() {
    for env in envs("non_ascii_database").await {
        let db = "pgt_bk_şema_ğ";
        env.fresh(db).await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.dump");
        assert!(!pgt_backup::windows_name_problems(&env.conn(db)).is_empty());
        if pgt_backup::ensure_ansi_roundtrip(db) {
            let (s, sink) = env.backup(db, BackupScope::Database, DumpFormat::Custom, &out).await;
            assert_eq!(s.outcome, JobOutcome::Failed, "{}: araç ASCII dışı adla bağlanabildi: {:?}", env.version, sink.lines());
        } else {
            // Ad kod sayfasında hiç temsil edilemiyor (ör. 1252'de `ş`/`ğ`): araç çalışmadan plan hatası.
            let req = pgt_backup::BackupRequest {
                scope: BackupScope::Database,
                format: DumpFormat::Custom,
                options: Default::default(),
                output_path: out.to_string_lossy().into_owned(),
            };
            let e = pgt_backup::backup_invocation(&env.dump, &env.conn(db), &req).unwrap_err();
            assert!(e.iter().any(|e| e.path == "database"), "{e:?}");
        }
        env.drop(db).await;
    }
}
