//! Gerçek araçlarla günlük gizliliği: yalnız iş türü, sonuç, çıkış kodu ve süre; parola, yol, ad yok. Genel
//! (süreç çapında) abone kurulduğu için ayrı test ikilisidir ve tek test içerir.

mod common;
mod pgenv;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use pgenv::{envs, fixture, ok};
use pgt_backup::{BackupScope, DumpFormat, JobOutcome, RestoreOptions};

/// `tracing` olaylarının tüm alanlarını metin olarak toplayan en küçük abone .
struct Capture(Arc<Mutex<String>>);

struct Visit<'a>(&'a mut String);

impl tracing::field::Visit for Visit<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        span.record(&mut Visit(&mut self.0.lock().unwrap()));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut Visit(&mut self.0.lock().unwrap()));
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut s = self.0.lock().unwrap();
        s.push_str(event.metadata().target());
        s.push(' ');
        event.record(&mut Visit(&mut s));
        s.push('\n');
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// Gerçek araçla yedek + geri yükleme günlüğü: yalnız tür, sonuç, çıkış kodu ve süre; parola, yol, ad yok.
#[tokio::test]
async fn logs_hold_no_password_path_or_names() {
    let envs = envs("log_privacy").await;
    let logs = Arc::new(Mutex::new(String::new()));
    tracing::subscriber::set_global_default(Capture(Arc::clone(&logs))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for env in &envs {
        let src = "pgt_bk_gizli_db";
        fixture(env, src).await;
        let out = dir.path().join("gizli-yedek-adı.dump");
        let scope = BackupScope::Table { schema: "app".into(), name: "Büyük Ad".into() };
        let (s, sink) = env.backup(src, scope, DumpFormat::Custom, &out).await;
        ok(&s, &sink, "yedek");
        // Başarısız geri yükleme (şema yok, hatada dur): stderr metni de günlüğe girmez.
        let tgt = "pgt_bk_gizli_tgt";
        env.fresh(tgt).await;
        let (s, _, _) = env.restore(tgt, &out, RestoreOptions { exit_on_error: true, ..Default::default() }).await;
        assert_eq!(s.outcome, JobOutcome::Failed);
        env.drop(tgt).await;
        env.drop(src).await;
    }
    if envs.is_empty() {
        return;
    }
    let logs = logs.lock().unwrap().clone();
    assert!(logs.contains("job=\"backup\"") && logs.contains("job=\"restore\"") && logs.contains("failed"), "günlükte beklenen alanlar yok");
    let dir_text = dir.path().to_string_lossy().into_owned();
    let mut secrets = vec!["gizli", "Büyük", "pgt_bk", "app.", "pg_dump", "pg_restore", "schema", dir_text.as_str()];
    let passwords: Vec<String> = envs.iter().filter_map(|e| e.ts.password.clone()).collect();
    secrets.extend(passwords.iter().map(String::as_str));
    for secret in secrets {
        assert!(!logs.contains(secret), "günlükte yasak metin var (uzunluk {})", secret.len());
    }
}
