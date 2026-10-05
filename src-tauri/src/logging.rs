use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

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
        .with_env_filter(EnvFilter::try_from_env("PGT_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "panik");
        default_hook(info);
    }));
    Some(guard)
}
