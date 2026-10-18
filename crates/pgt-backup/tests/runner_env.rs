//! Çocuk sürecin ortamı ve günlük gizliliği. Süreç ortamını değiştirdiği için ayrı test ikilisidir ve tek test
//! içerir (başka iş parçacığı ortamı okurken `set_var` çağrılmaz).

mod common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use common::{VecSink, inv, plan};
use pgt_backup::{CancelToken, JobKind, JobOutcome, Secret, run};

/// `tracing` olaylarının tüm alanlarını metin olarak toplayan en küçük abone.
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

fn env_lines(sink: &VecSink) -> Vec<String> {
    let mut v: Vec<String> = sink
        .lines()
        .into_iter()
        .filter_map(|(_, t)| t.strip_prefix("ENV ").map(str::to_string))
        // Windows'ta ortam adları harf duyarsızdır: miras `pghost` yerine verilen değer mirastaki yazımla gelebilir.
        .map(|t| match t.split_once('=') {
            Some((k, v)) if cfg!(windows) => format!("{}={v}", k.to_uppercase()),
            _ => t,
        })
        .filter(|t| t.to_uppercase().starts_with("PG") || t.to_uppercase().starts_with("LC_ALL") || t.starts_with("LANGUAGE"))
        .collect();
    v.sort();
    v
}

#[test]
fn child_env_and_log_privacy() {
    // SAFETY: bu ikilide tek test vardır ve henüz başka iş parçacığı (çalışma zamanı dahil) başlatılmadı.
    unsafe {
        std::env::set_var("PGPASSWORD", "miras-parola");
        std::env::set_var("PGSERVICE", "miras-servis");
        std::env::set_var("PGPASSFILE", "C:/miras/pgpass.conf");
        std::env::set_var("PGOPTIONS", "-c search_path=x");
        // Windows'ta ortam adları harf duyarsızdır; küçük harfli ad da temizlenmeli.
        std::env::set_var("pghost", "miras-host");
        std::env::set_var("LC_ALL", "tr_TR.UTF-8");
        std::env::set_var("LANGUAGE", "tr");
    }
    let logs = Arc::new(Mutex::new(String::new()));
    let _sub = tracing::subscriber::set_default(Capture(Arc::clone(&logs)));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    // Parolasız: miras PG* anahtarları yok, yalnız çağrının ortamı var.
    let mut sink = VecSink::default();
    let s = rt.block_on(run(inv(&["--print-env"], None), None, JobKind::Restore, None, &mut sink, CancelToken::new()));
    assert_eq!(s.outcome, JobOutcome::Success);
    assert_eq!(env_lines(&sink), vec!["PGHOST=h.example".to_string()]);

    // Parolayla: yalnız verilen parola.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("gizli-hedef-adı.dump");
    let p = plan(&target, false);
    let partial = p.partial_path.clone();
    let mut sink = VecSink::default();
    let secret = Secret::new("s3cret-değer".to_string());
    assert!(!format!("{secret:?}").contains("s3cret"));
    assert!(!serde_json::to_string(&secret).unwrap().contains("s3cret"));
    let s = rt.block_on(run(
        inv(&["--print-env", "--file", &partial, "--line", "pg_dump: error: gizli-stderr-metni", "--exit", "3"], Some(p)),
        Some(secret),
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    ));
    assert_eq!((s.outcome, s.exit_code), (JobOutcome::Failed, Some(3)));
    assert_eq!(env_lines(&sink), vec!["PGHOST=h.example".to_string(), "PGPASSWORD=s3cret-değer".to_string()]);

    // Günlükte yalnız tür, sonuç, çıkış kodu ve süre: parola, yol, argüman ve stderr metni yok.
    let logs = logs.lock().unwrap().clone();
    assert!(logs.contains("outcome") && logs.contains("failed"), "{logs}");
    for secret in ["s3cret", "gizli-hedef", "gizli-stderr", "--print-env", "pg_dump", "miras"] {
        assert!(!logs.contains(secret), "{secret} günlükte: {logs}");
    }
}
