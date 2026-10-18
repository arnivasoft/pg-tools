//! Yürütücü testleri (sahte araçla; PostgreSQL gerekmez).

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{VecSink, inv, plan};
use pgt_backup::{
    BackupEvent, CancelToken, JobKind, JobOutcome, LineSeverity, MAX_LINE_BYTES, MAX_LOG_LINES, check_output, run,
};

fn finished_matches(sink: &VecSink, s: &pgt_backup::JobSummary) {
    assert_eq!(sink.events.last(), Some(&BackupEvent::from(s)), "son olay Finished olmalı");
    let finished = sink.events.iter().filter(|e| matches!(e, BackupEvent::Finished { .. })).count();
    assert_eq!(finished, 1);
}

#[tokio::test]
async fn streams_and_classifies_lines() {
    let i = inv(
        &[
            "--line",
            "pg_dump: warning: döngüsel yabancı anahtar",
            "--line",
            "pg_dump: error: query failed",
            "--crlf",
            "pg_dump: reading tables",
            "--tables",
            "2",
        ],
        None,
    );
    let preview = i.preview.clone();
    let mut sink = VecSink::default();
    let s = run(i, None, JobKind::Backup, Some(2), &mut sink, CancelToken::new()).await;
    assert_eq!(sink.events[0], BackupEvent::Started { preview });
    assert_eq!(
        sink.lines(),
        vec![
            (LineSeverity::Warning, "pg_dump: warning: döngüsel yabancı anahtar".to_string()),
            (LineSeverity::Error, "pg_dump: error: query failed".to_string()),
            (LineSeverity::Info, "pg_dump: reading tables".to_string()),
            (LineSeverity::Info, "pg_dump: dumping contents of table \"public.t0\"".to_string()),
            (LineSeverity::Info, "pg_dump: dumping contents of table \"public.t1\"".to_string()),
        ]
    );
    assert_eq!(sink.progress(), vec![(1, Some(2), None), (2, Some(2), None)]);
    assert_eq!((s.outcome, s.exit_code, s.warnings, s.errors), (JobOutcome::Success, Some(0), 1, 1));
    assert_eq!((s.output_path.as_deref(), s.hint.as_deref(), s.truncated_lines), (None, None, 0));
    finished_matches(&sink, &s);
}

#[tokio::test]
async fn caps_line_count_and_length() {
    let mut sink = VecSink::default();
    let s = run(
        inv(&["--line", "pg_dump: warning: önce", "--tables", "2500", "--line", "pg_dump: warning: son"], None),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    let lines = sink.lines();
    // 2000 satır + taşmayı bildiren son satır.
    assert_eq!(lines.len(), MAX_LOG_LINES + 1);
    let (sev, last) = lines.last().unwrap();
    assert_eq!(*sev, LineSeverity::Info);
    assert!(last.contains("502"), "{last}");
    assert_eq!(s.truncated_lines, 502);
    // Gösterilmeyen satırlar da sayılır (ilerleme ve uyarı).
    assert_eq!(sink.progress().last().unwrap().0, 2500);
    assert_eq!(s.warnings, 2);
    finished_matches(&sink, &s);

    let mut sink = VecSink::default();
    run(inv(&["--long", "3000"], None), None, JobKind::Backup, None, &mut sink, CancelToken::new()).await;
    let (_, text) = &sink.lines()[0];
    assert!(text.ends_with('…'), "{}", &text[text.len() - 10..]);
    let body = text.trim_end_matches('…');
    assert!(body.len() <= MAX_LINE_BYTES && body.len() > MAX_LINE_BYTES - 4, "{}", body.len());
    assert!(body.chars().all(|c| c == 'ğ'));
}

#[tokio::test]
async fn success_moves_partial_file_to_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.dump");
    std::fs::write(&target, b"eski").unwrap();
    let p = plan(&target, false);
    let partial = p.partial_path.clone();
    let mut sink = VecSink::default();
    let s = run(
        inv(&["--file", &partial, "--sleep-ms", "2500"], Some(p)),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    assert_eq!(s.outcome, JobOutcome::Success, "{:?}", sink.lines());
    assert_eq!(std::fs::read(&target).unwrap(), vec![b'F'; 1000]);
    assert!(!Path::new(&partial).exists());
    assert_eq!(s.output_path.as_deref(), Some(target.to_string_lossy().as_ref()));
    // Geçici dosyanın boyutu 1 sn aralıkla bildirilir.
    assert!(sink.progress().contains(&(0, None, Some(1000))), "{:?}", sink.progress());
    finished_matches(&sink, &s);
}

#[tokio::test]
async fn directory_format_atomic_output() {
    let dir = tempfile::tempdir().unwrap();
    // Hedef yoksa.
    let target = dir.path().join("d");
    let p = plan(&target, true);
    let partial = p.partial_path.clone();
    let mut sink = VecSink::default();
    let s = run(
        inv(&["--format=directory", "--file", &partial, "--sleep-ms", "2500"], Some(p)),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    assert_eq!(s.outcome, JobOutcome::Success, "{:?}", sink.lines());
    assert!(target.join("toc.dat").is_file());
    assert!(!Path::new(&partial).exists());
    assert!(sink.progress().contains(&(0, None, Some(500))), "{:?}", sink.progress());

    // Hedef boş klasörse yerine geçer; önceki çöküşten kalan geçici klasör silinir.
    let target = dir.path().join("e");
    std::fs::create_dir(&target).unwrap();
    let p = plan(&target, true);
    let partial = p.partial_path.clone();
    std::fs::create_dir(&partial).unwrap();
    std::fs::write(Path::new(&partial).join("eski.dat"), b"x").unwrap();
    let mut sink = VecSink::default();
    let s = run(
        inv(&["--format=directory", "--file", &partial], Some(p)),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    assert_eq!(s.outcome, JobOutcome::Success, "{:?}", sink.lines());
    assert!(target.join("toc.dat").is_file() && !target.join("eski.dat").exists());

    // Hedef dolu klasörse araç hiç başlatılmaz.
    let p = plan(&target, true);
    assert!(check_output(&p).is_err());
    let partial = p.partial_path.clone();
    let mut sink = VecSink::default();
    let s = run(
        inv(&["--format=directory", "--file", &partial], Some(p)),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    assert_eq!((s.outcome, s.exit_code, s.errors), (JobOutcome::Failed, None, 1));
    assert!(!Path::new(&partial).exists());
    assert_eq!(sink.lines().last().unwrap().0, LineSeverity::Error);
    finished_matches(&sink, &s);

    // Dosya formatında hedef bir klasör olamaz; klasör formatında hedef bir dosya olamaz.
    assert!(check_output(&plan(&target, false)).is_err());
    let file = dir.path().join("f.sql");
    std::fs::write(&file, b"x").unwrap();
    assert!(check_output(&plan(&file, true)).is_err());
    assert!(check_output(&plan(&file, false)).is_ok());
    assert!(check_output(&plan(&dir.path().join("yok"), true)).is_ok());
}

#[tokio::test]
async fn failure_removes_partial_and_keeps_old_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.sql");
    std::fs::write(&target, b"eski").unwrap();
    let p = plan(&target, false);
    let partial = p.partial_path.clone();
    let mut sink = VecSink::default();
    // pg_dump plain dosyayı bağlanmadan önce oluşturur; bağlantı hatasında da temizlenmeli.
    let s = run(
        inv(
            &[
                "--file",
                &partial,
                "--line",
                "pg_dump: error: connection to server failed: FATAL:  password authentication failed for user \"u\"",
                "--exit",
                "1",
            ],
            Some(p),
        ),
        None,
        JobKind::Backup,
        None,
        &mut sink,
        CancelToken::new(),
    )
    .await;
    assert_eq!((s.outcome, s.exit_code, s.errors), (JobOutcome::Failed, Some(1), 1));
    assert!(s.hint.as_deref().unwrap().contains("parola"));
    assert!(!Path::new(&partial).exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"eski");
    assert_eq!(s.output_path, None);
    finished_matches(&sink, &s);
}

#[tokio::test]
async fn success_without_output_is_failure() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan(&dir.path().join("a.dump"), false);
    let mut sink = VecSink::default();
    let s = run(inv(&[], Some(p)), None, JobKind::Backup, None, &mut sink, CancelToken::new()).await;
    assert_eq!((s.outcome, s.exit_code), (JobOutcome::Failed, Some(0)));
    assert_eq!(s.output_path, None);
}

/// pg_restore "hatada dur" kapalıyken: atlanan hatalarla çıkış 1 ve sayı satırı → `completed_with_errors`.
fn restore_inv(lines: &[&str], exit: &str, tolerates: bool, tool: pgt_backup::ToolKind) -> pgt_backup::Invocation {
    let mut args: Vec<&str> = Vec::new();
    for l in lines {
        args.extend(["--line", l]);
    }
    args.extend(["--exit", exit]);
    let mut i = inv(&args, None);
    i.tool = tool;
    i.tolerates_ignored_errors = tolerates;
    i
}

#[tokio::test]
async fn ignored_restore_errors_complete_with_errors() {
    use pgt_backup::ToolKind::{PgRestore, Psql};
    let tt = r#"pg_restore: error: could not execute query: ERROR:  unrecognized configuration parameter "transaction_timeout""#;
    let tail = "pg_restore: warning: errors ignored on restore: 1";
    let go = |i| async move {
        let mut sink = VecSink::default();
        let s = run(i, None, JobKind::Restore, None, &mut sink, CancelToken::new()).await;
        finished_matches(&sink, &s);
        s
    };
    let s = go(restore_inv(&[tt, tail], "1", true, PgRestore)).await;
    assert_eq!((s.outcome, s.exit_code, s.errors, s.warnings), (JobOutcome::CompletedWithErrors, Some(1), 1, 1));
    // İpucu bu sonuçta da verilir (ipucu 4, pg_restore metni).
    assert!(s.hint.as_deref().is_some_and(|h| h.contains("transaction_timeout") && h.contains("pg_restore")), "{s:?}");
    // Sayı satırı yoksa (erken çıkış) ya da "hatada dur" açıksa ya da çıkış kodu farklıysa: failed.
    assert_eq!(go(restore_inv(&[tt], "1", true, PgRestore)).await.outcome, JobOutcome::Failed);
    assert_eq!(go(restore_inv(&[tt, tail], "1", false, PgRestore)).await.outcome, JobOutcome::Failed);
    assert_eq!(go(restore_inv(&[tt, tail], "2", true, PgRestore)).await.outcome, JobOutcome::Failed);
    // psql işi bu kurala girmez; ipucu 4 düz SQL metnidir.
    let p = go(restore_inv(&[r#"psql:a.sql:9: ERROR:  unrecognized configuration parameter "transaction_timeout""#], "3", false, Psql)).await;
    assert_eq!(p.outcome, JobOutcome::Failed);
    assert!(p.hint.as_deref().is_some_and(|h| h.contains("pg_dump ile yeniden")), "{p:?}");
    // Yedekleme işinde aynı satır sonucu değiştirmez.
    let mut sink = VecSink::default();
    let b = run(restore_inv(&[tail], "1", true, PgRestore), None, JobKind::Backup, None, &mut sink, CancelToken::new()).await;
    assert_eq!(b.outcome, JobOutcome::Failed);
}

#[tokio::test]
async fn spawn_failure_is_reported() {
    let mut i = inv(&[], None);
    i.exe = std::env::temp_dir().join("pgt-yok-boyle-bir-arac.exe").to_string_lossy().into_owned();
    let mut sink = VecSink::default();
    let s = run(i, None, JobKind::Restore, None, &mut sink, CancelToken::new()).await;
    assert_eq!((s.outcome, s.exit_code, s.errors), (JobOutcome::Failed, None, 1));
    assert_eq!(sink.lines().len(), 1);
    finished_matches(&sink, &s);
}

async fn cancelled_run(sink: &mut VecSink, token: CancelToken) {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.dump");
    let p = plan(&target, false);
    let partial = p.partial_path.clone();
    let started = Instant::now();
    let s = run(
        inv(&["--file", &partial, "--line", "hazır", "--sleep-ms", "30000"], Some(p)),
        None,
        JobKind::Backup,
        None,
        sink,
        token,
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(15));
    assert_eq!((s.outcome, s.exit_code, s.hint.as_deref()), (JobOutcome::Cancelled, None, None));
    assert!(!Path::new(&partial).exists() && !target.exists());
    finished_matches(sink, &s);
}

#[tokio::test]
async fn cancel_kills_child_and_cleans_up() {
    let token = CancelToken::new();
    let t = token.clone();
    let mut sink = VecSink::with(move |line| {
        if line == "hazır" {
            t.cancel();
        }
        true
    });
    cancelled_run(&mut sink, token).await;
}

#[tokio::test]
async fn sink_gone_cancels() {
    let mut sink = VecSink::with(|line| line != "hazır");
    cancelled_run(&mut sink, CancelToken::new()).await;
}

#[tokio::test]
async fn pre_cancelled_token_does_not_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan(&dir.path().join("a.dump"), false);
    let partial = p.partial_path.clone();
    let token = CancelToken::new();
    token.cancel();
    let mut sink = VecSink::default();
    let s = run(inv(&["--file", &partial], Some(p)), None, JobKind::Backup, None, &mut sink, token).await;
    assert_eq!(s.outcome, JobOutcome::Cancelled);
    assert!(!Path::new(&partial).exists());
}

#[tokio::test]
async fn failed_move_keeps_partial_and_says_next_backup_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.dump");
    let p = plan(&target, false);
    let partial = p.partial_path.clone();
    let target_s = target.to_string_lossy().into_owned();
    let mut sink = VecSink::default();
    // Araç bitmeden hedef, içi dolu bir klasör olur: taşıma başarısız olur.
    let s = run(inv(&["--file", &partial, "--mkdir", &target_s], Some(p)), None, JobKind::Backup, None, &mut sink, CancelToken::new())
        .await;
    assert_eq!(s.outcome, JobOutcome::Failed);
    assert!(Path::new(&partial).is_file(), "geçici çıktı korunmalı");
    let msg = sink.lines().into_iter().map(|(_, t)| t).find(|t| t.contains("geçici konumda")).expect("ileti");
    assert!(msg.contains("bir sonraki") && msg.contains("üzerine yaz"), "{msg}");
    finished_matches(&sink, &s);
}

/// Araç sessizken (satır yok, bayt değişmiyor) her 1 sn'de bir `Progress` gider: alıcının gittiği böyle anlaşılır.
#[tokio::test]
async fn heartbeat_detects_gone_sink_while_tool_is_silent() {
    struct GoneOnProgress(u32);
    impl pgt_backup::EventSink for GoneOnProgress {
        fn send(&mut self, e: pgt_backup::BackupEvent) -> bool {
            if matches!(e, pgt_backup::BackupEvent::Progress { .. }) {
                self.0 += 1;
                return false;
            }
            true
        }
    }
    let started = Instant::now();
    let mut sink = GoneOnProgress(0);
    let s = run(inv(&["--sleep-ms", "30000"], None), None, JobKind::Restore, None, &mut sink, CancelToken::new()).await;
    assert_eq!(s.outcome, JobOutcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(sink.0, 1);
}

/// Yürütücü son savunma hattıdır: kod sayfasında birebir temsil edilemeyen argüman ya da parola (Windows, ACP
/// UTF-8 değilse) aracı hiç başlatmaz. Diğer sistemlerde ve UTF-8 ACP'de araç çalışır.
#[tokio::test]
async fn runner_refuses_best_fit_values_before_spawning() {
    let refuses = cfg!(windows) && !pgt_backup::ensure_ansi_roundtrip("\u{ff02}");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("a.dump");
    let p = plan(&out, false);
    let partial = p.partial_path.clone();
    let cases = [
        (inv(&["--file", &partial, "--line", "x\u{ff02} -h evil \u{ff02}"], Some(p.clone())), None),
        (inv(&["--file", &partial], Some(p)), Some(pgt_backup::Secret::new("a\u{ff02}b".into()))),
    ];
    for (i, password) in cases {
        let mut sink = VecSink::default();
        let s = run(i, password, JobKind::Backup, None, &mut sink, CancelToken::new()).await;
        if refuses {
            assert_eq!((s.outcome, s.exit_code), (JobOutcome::Failed, None));
            assert!(sink.lines().iter().any(|(_, t)| t.contains("kod sayfasında")), "{:?}", sink.lines());
            assert!(!sink.lines().iter().any(|(_, t)| t.contains("a\u{ff02}b")));
            assert!(!Path::new(&partial).exists() && !out.exists(), "araç çalışmamalıydı");
        } else {
            assert_eq!(s.outcome, JobOutcome::Success, "{:?}", sink.lines());
            std::fs::remove_file(&out).unwrap();
        }
    }
}
