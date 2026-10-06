use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

/// Süzgeç: `PGT_LOG` (varsayılan info) + `tokio_postgres=info`. tokio-postgres `debug` seviyesinde çalıştırdığı SQL
/// metnini `log` kaydı olarak yazar (`executing statement batch: …`). Sabit yönerge yalnızca AYNI hedefteki
/// (`tokio_postgres`) yönergelerin üzerine yazar; daha özel bir alt hedef (ör. `tokio_postgres::query=debug`) debug
/// kaydını yine açabilir. Asıl güvence köprünün yokluğudur: `tracing-log` kapalı, `log` -> `tracing` köprüsü
/// kurulmaz; tokio-postgres kayıtları günlük dosyasına ulaşamaz.
pub fn build_filter(env: Option<&str>) -> EnvFilter {
    let base = env.and_then(|e| EnvFilter::try_new(e).ok()).unwrap_or_else(|| EnvFilter::new("info"));
    base.add_directive("tokio_postgres=info".parse().expect("geçerli yönerge"))
}

/// `%LOCALAPPDATA%\pg-tools\logs\pg-tools.YYYY-MM-DD.log`, son 5 dosya. Seviye: `PGT_LOG` (varsayılan info).
pub fn init() -> Option<WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("pg-tools")
        .filename_suffix("log")
        .max_log_files(5)
        .build(crate::state::log_dir())
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(build_filter(std::env::var("PGT_LOG").ok().as_deref()))
        .init();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "panik");
        default_hook(info);
    }));
    Some(guard)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::build_filter;

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// `log` -> `tracing` köprüsü (LogTracer) AÇIKKEN bile tokio-postgres kayıtları süzülür; uygulama kodunun
    /// debug kaydı ise geçer (süzgecin her şeyi yutmadığını kanıtlar).
    #[test]
    fn tokio_postgres_sql_never_reaches_the_log_even_at_debug_or_trace() {
        use tracing_log::log;
        tracing_log::LogTracer::init().expect("bu test ikilisinde başka log köprüsü yok");
        for env in [None, Some("info"), Some("debug"), Some("trace"), Some("debug,tokio_postgres=trace"), Some("tokio_postgres=trace")] {
            let buf = Buf::default();
            let w = buf.clone();
            let subscriber = tracing_subscriber::fmt().with_writer(move || w.clone()).with_ansi(false).with_env_filter(build_filter(env)).finish();
            tracing::subscriber::with_default(subscriber, || {
                log::debug!(target: "tokio_postgres::query", "executing statement batch: CREATE TABLE gizli_tablo (id int)");
                log::trace!(target: "tokio_postgres::prepare", "preparing query s0: COMMENT ON SCHEMA gizli IS 'x'");
                tracing::debug!(target: "pg_tools::deneme", "uygulama_debug_kaydi");
            });
            let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
            for secret in ["CREATE", "COMMENT", "gizli_tablo", "executing statement", "preparing query"] {
                assert!(!out.contains(secret), "PGT_LOG={env:?}: günlükte {secret:?} var: {out}");
            }
            let debug_on = matches!(env, Some("debug") | Some("trace") | Some("debug,tokio_postgres=trace"));
            assert_eq!(out.contains("uygulama_debug_kaydi"), debug_on, "PGT_LOG={env:?}: {out}");
        }
    }
}
