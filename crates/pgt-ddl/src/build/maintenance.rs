//! Bakım işlemleri üreticisi (Plan 24): tek, transaction dışı VACUUM / ANALYZE / REINDEX / CLUSTER ifadesi.
//!
//! Hedef/işlem matrisi ve sürüm kapıları spec §1'e göre; geçersiz birleşim `op`, seçenek hatası `options.<alan>`,
//! düzenlenemeyen hedef `object` yolunda döner (sessiz atlama yok).

use super::{BuildCtx, check_old_ident, err, freesql::composed_is_clean, stmt_nt};
use crate::maintenance::*;
use crate::model::FieldError;
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident};

const LOCK_NOTE: &str = "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler.";
const REINDEX_NOTE: &str = "Tabloda SHARE, indekste ACCESS EXCLUSIVE kilit alır; yazma ve indeksi kullanan okumalar bekler.";
const REINDEX_MANY_NOTE: &str = " Tablolar sırayla kilitlenir.";
const COMPOSED_BROKEN: &str = "Üretilen ifade güvenli biçimde kurulamadı (ad tırnak/yorum sınırını bozuyor).";

/// Üretici girişi: doğrular, tek ifade üretir.
pub fn maintenance(ctx: &BuildCtx, target: &MaintenanceTarget, op: &MaintenanceOp) -> Result<Plan, Vec<FieldError>> {
    // Not: `ctx.concurrently`, `target.table` ve `target.unsupported` burada kullanılmaz (REINDEX CONCURRENTLY
    // `ReindexOptions`'tan gelir; indeks ifadesi şema+adla yazılır; `unsupported` yükleyici/arayüz içindir).
    let v = ctx.version_num;
    let mut errs = Vec::new();

    if !target.editable {
        errs.push(err("object", "Bu nesne için bakım işlemi yapılamıyor."));
    }
    // Yüklenmiş (eski taraf) adlar: yalnız boş/NUL denetimi; `quote_ident` yerleştirir.
    check_old_ident("object", &target.name, &mut errs);
    if let Some(s) = &target.schema {
        check_old_ident("object", s, &mut errs);
    }
    let needs_schema = matches!(
        target.kind,
        MaintenanceTargetKind::Table | MaintenanceTargetKind::MaterializedView | MaintenanceTargetKind::Index
    );
    if needs_schema && target.schema.is_none() {
        errs.push(err("object", "Hedefin şeması bilinmiyor."));
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let op_name = match op {
        MaintenanceOp::Vacuum(_) => "vacuum",
        MaintenanceOp::Analyze(_) => "analyze",
        MaintenanceOp::Reindex(_) => "reindex",
        MaintenanceOp::Cluster(_) => "cluster",
    };
    if !ops_for(target.kind, target.partitioned, v).contains(&op_name) {
        let part = target.partitioned
            && matches!(target.kind, MaintenanceTargetKind::Table | MaintenanceTargetKind::Index);
        let msg = if op_name == "cluster" && part {
            "Bölümlenmiş tabloda CLUSTER PostgreSQL 15 ve üzerini gerektirir."
        } else if op_name == "reindex" && part {
            "Bölümlenmiş tabloda/indekste REINDEX PostgreSQL 14 ve üzerini gerektirir."
        } else {
            "Bu hedef için bu işlem desteklenmiyor."
        };
        return Err(vec![err("op", msg)]);
    }

    let rel = || qualified(target.schema.as_deref().unwrap_or_default(), &target.name);
    let (sql, risk) = match op {
        MaintenanceOp::Vacuum(o) => {
            (vacuum(v, target, o, &rel, &mut errs), if o.full { Risk::Destructive } else { Risk::Safe })
        }
        MaintenanceOp::Analyze(o) => (analyze(v, target, o, &rel, &mut errs), Risk::Safe),
        MaintenanceOp::Reindex(o) => {
            let risk = if o.concurrently { Risk::Safe } else { Risk::Destructive };
            (reindex(v, target, o, &rel, &mut errs), risk)
        }
        MaintenanceOp::Cluster(o) => (cluster(target, o, &rel, &mut errs), Risk::Destructive),
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    if !composed_is_clean(&sql) {
        return Err(vec![err("object", COMPOSED_BROKEN)]);
    }
    let note = if risk != Risk::Destructive {
        None
    } else if let MaintenanceOp::Reindex(_) = op {
        let many = matches!(target.kind, MaintenanceTargetKind::Schema | MaintenanceTargetKind::Database);
        Some(if many { format!("{REINDEX_NOTE}{REINDEX_MANY_NOTE}") } else { REINDEX_NOTE.to_string() })
    } else {
        Some(LOCK_NOTE.to_string())
    };
    Ok(Plan { statements: vec![stmt_nt(sql, risk, note.as_deref())], warnings: vec![] })
}

fn gate(errs: &mut Vec<FieldError>, v: u32, min: u32, path: &str, what: &str, major: u32) {
    if v < min {
        errs.push(err(path, format!("{what} PostgreSQL {major} ve üzerini gerektirir.")));
    }
}

/// `VERB (opt, ...) [hedef]`; seçenek yoksa parantez yazılmaz, veritabanı hedefinde ad yazılmaz.
fn with_opts(verb: &str, opts: &[String], target: &MaintenanceTarget, rel: &dyn Fn() -> String) -> String {
    let mut s = verb.to_string();
    if !opts.is_empty() {
        s.push_str(&format!(" ({})", opts.join(", ")));
    }
    if target.kind != MaintenanceTargetKind::Database {
        s.push(' ');
        s.push_str(&rel());
    }
    s
}

fn vacuum(
    v: u32,
    t: &MaintenanceTarget,
    o: &VacuumOptions,
    rel: &dyn Fn() -> String,
    errs: &mut Vec<FieldError>,
) -> String {
    let mut opts = Vec::new();
    if o.full {
        opts.push("FULL".to_string());
    }
    if o.freeze {
        opts.push("FREEZE".into());
    }
    if o.analyze {
        opts.push("ANALYZE".into());
    }
    if o.verbose {
        opts.push("VERBOSE".into());
    }
    if o.disable_page_skipping {
        if o.full {
            errs.push(err("options.disablePageSkipping", "DISABLE_PAGE_SKIPPING, FULL ile birlikte kullanılamaz."));
        }
        opts.push("DISABLE_PAGE_SKIPPING".into());
    }
    if o.skip_locked {
        gate(errs, v, 120000, "options.skipLocked", "SKIP_LOCKED", 12);
        opts.push("SKIP_LOCKED".into());
    }
    if let Some(c) = o.index_cleanup {
        gate(errs, v, 120000, "options.indexCleanup", "INDEX_CLEANUP", 12);
        if c == IndexCleanup::Auto {
            gate(errs, v, 140000, "options.indexCleanup", "INDEX_CLEANUP AUTO", 14);
        }
        opts.push(
            match c {
                IndexCleanup::Auto => "INDEX_CLEANUP AUTO",
                IndexCleanup::On => "INDEX_CLEANUP ON",
                IndexCleanup::Off => "INDEX_CLEANUP OFF",
            }
            .into(),
        );
    }
    if let Some(n) = o.parallel {
        if n > 1024 {
            errs.push(err("options.parallel", "PARALLEL değeri 0–1024 aralığında olmalı."));
        } else if o.full {
            errs.push(err("options.parallel", "PARALLEL, FULL ile birlikte kullanılamaz."));
        } else {
            gate(errs, v, 130000, "options.parallel", "PARALLEL", 13);
        }
        opts.push(format!("PARALLEL {n}"));
    }
    with_opts("VACUUM", &opts, t, rel)
}

fn analyze(
    v: u32,
    t: &MaintenanceTarget,
    o: &AnalyzeOptions,
    rel: &dyn Fn() -> String,
    errs: &mut Vec<FieldError>,
) -> String {
    let mut opts = Vec::new();
    if o.verbose {
        opts.push("VERBOSE".to_string());
    }
    if o.skip_locked {
        gate(errs, v, 120000, "options.skipLocked", "SKIP_LOCKED", 12);
        opts.push("SKIP_LOCKED".into());
    }
    if v < 110000 {
        // PG 10: parantezli seçenek listesi yok; yalnız `ANALYZE VERBOSE [hedef]`.
        let mut s = String::from("ANALYZE");
        if o.verbose {
            s.push_str(" VERBOSE");
        }
        if t.kind != MaintenanceTargetKind::Database {
            s.push(' ');
            s.push_str(&rel());
        }
        return s;
    }
    with_opts("ANALYZE", &opts, t, rel)
}

fn reindex(
    v: u32,
    t: &MaintenanceTarget,
    o: &ReindexOptions,
    rel: &dyn Fn() -> String,
    errs: &mut Vec<FieldError>,
) -> String {
    if o.concurrently {
        gate(errs, v, 120000, "options.concurrently", "CONCURRENTLY", 12);
        if t.system {
            errs.push(err("options.concurrently", "Sistem nesnelerinde CONCURRENTLY kullanılamaz."));
        }
    }
    use MaintenanceTargetKind::*;
    let (kw, name) = match t.kind {
        Index => ("INDEX", rel()),
        Table | MaterializedView => ("TABLE", rel()),
        Schema => ("SCHEMA", quote_ident(&t.name)),
        Database => ("DATABASE", quote_ident(&t.name)),
    };
    // Seçenekler hedef sözcüğünden önce, CONCURRENTLY sonra (PG 10–18 ortak biçimi).
    let mut s = String::from("REINDEX");
    if o.verbose {
        s.push_str(" (VERBOSE)");
    }
    s.push(' ');
    s.push_str(kw);
    if o.concurrently {
        s.push_str(" CONCURRENTLY");
    }
    s.push(' ');
    s.push_str(&name);
    s
}

fn cluster(t: &MaintenanceTarget, o: &ClusterOptions, rel: &dyn Fn() -> String, errs: &mut Vec<FieldError>) -> String {
    if t.system {
        errs.push(err("op", "Sistem nesnelerinde CLUSTER kullanılamaz."));
    }
    match &o.using_index {
        Some(i) if !t.indexes.contains(i) => {
            errs.push(err("options.usingIndex", "İndeks bu tabloya ait değil."));
        }
        None if t.cluster_index.is_none() => {
            errs.push(err("options.usingIndex", "Tabloda kümeleme indeksi yok; bir indeks seçin."));
        }
        _ => {}
    }
    let mut s = String::from("CLUSTER");
    if o.verbose {
        s.push_str(" VERBOSE");
    }
    s.push(' ');
    s.push_str(&rel());
    if let Some(i) = &o.using_index {
        s.push_str(" USING ");
        s.push_str(&quote_ident(i));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{assert_single_statements, render};
    use MaintenanceTargetKind::*;

    fn ctx(v: u32) -> BuildCtx {
        BuildCtx { version_num: v, ..Default::default() }
    }
    fn tgt(kind: MaintenanceTargetKind) -> MaintenanceTarget {
        let (schema, name, table) = match kind {
            Table | MaterializedView => (Some("s".to_string()), "t", None),
            Index => (Some("s".to_string()), "i", Some("t".to_string())),
            Schema => (None, "s", None),
            Database => (None, "d", None),
        };
        MaintenanceTarget {
            kind,
            schema,
            name: name.into(),
            table,
            partitioned: false,
            system: false,
            cluster_index: None,
            indexes: vec!["i1".into(), "i2".into()],
            editable: true,
            unsupported: vec![],
        }
    }
    fn vac() -> VacuumOptions {
        VacuumOptions {
            full: false,
            freeze: false,
            analyze: false,
            verbose: false,
            disable_page_skipping: false,
            skip_locked: false,
            index_cleanup: None,
            parallel: None,
        }
    }
    fn ana() -> AnalyzeOptions {
        AnalyzeOptions { verbose: false, skip_locked: false }
    }
    fn rei() -> ReindexOptions {
        ReindexOptions { concurrently: false, verbose: false }
    }
    fn clu() -> ClusterOptions {
        ClusterOptions { verbose: false, using_index: None }
    }
    fn sql(v: u32, t: &MaintenanceTarget, op: MaintenanceOp) -> String {
        let p = maintenance(&ctx(v), t, &op).expect("geçerli");
        assert_single_statements(&p);
        assert_eq!(p.statements.len(), 1);
        assert!(!p.statements[0].transactional);
        p.statements[0].sql.clone()
    }
    fn errs(v: u32, t: &MaintenanceTarget, op: MaintenanceOp) -> Vec<FieldError> {
        maintenance(&ctx(v), t, &op).expect_err("hata beklenir")
    }
    fn paths(e: &[FieldError]) -> Vec<&str> {
        e.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn vacuum_matrix_snapshot() {
        let mut out = String::new();
        for k in [Table, MaterializedView, Database] {
            for o in [vac(), VacuumOptions { full: true, analyze: true, verbose: true, ..vac() }] {
                let p = maintenance(&ctx(170000), &tgt(k), &MaintenanceOp::Vacuum(o)).unwrap();
                out.push_str(&format!("{k:?}: {}", render(&p)));
            }
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn vacuum_all_options_snapshot() {
        let o = VacuumOptions {
            full: false,
            freeze: true,
            analyze: true,
            verbose: true,
            disable_page_skipping: true,
            skip_locked: true,
            index_cleanup: Some(IndexCleanup::Auto),
            parallel: Some(4),
        };
        let p = maintenance(&ctx(170000), &tgt(Table), &MaintenanceOp::Vacuum(o)).unwrap();
        insta::assert_snapshot!(render(&p));
        let o = VacuumOptions { index_cleanup: Some(IndexCleanup::Off), parallel: Some(0), ..vac() };
        assert_eq!(
            sql(130000, &tgt(Table), MaintenanceOp::Vacuum(o)),
            "VACUUM (INDEX_CLEANUP OFF, PARALLEL 0) s.t"
        );
    }

    #[test]
    fn vacuum_plain_has_no_parens_and_db_has_no_name() {
        assert_eq!(sql(160000, &tgt(Table), MaintenanceOp::Vacuum(vac())), "VACUUM s.t");
        assert_eq!(sql(160000, &tgt(Database), MaintenanceOp::Vacuum(vac())), "VACUUM");
        assert_eq!(
            sql(160000, &tgt(Database), MaintenanceOp::Vacuum(VacuumOptions { analyze: true, ..vac() })),
            "VACUUM (ANALYZE)"
        );
        assert_eq!(sql(100000, &tgt(Database), MaintenanceOp::Analyze(ana())), "ANALYZE");
    }

    #[test]
    fn analyze_snapshot() {
        let mut out = String::new();
        for k in [Table, MaterializedView, Database] {
            for o in [ana(), AnalyzeOptions { verbose: true, skip_locked: true }] {
                let p = maintenance(&ctx(170000), &tgt(k), &MaintenanceOp::Analyze(o)).unwrap();
                out.push_str(&format!("{k:?}: {}", render(&p)));
            }
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn reindex_matrix_snapshot() {
        let mut out = String::new();
        for k in [Index, Table, MaterializedView, Schema, Database] {
            for o in [
                rei(),
                ReindexOptions { concurrently: true, verbose: true },
                ReindexOptions { concurrently: true, verbose: false },
            ] {
                let p = maintenance(&ctx(170000), &tgt(k), &MaintenanceOp::Reindex(o)).unwrap();
                out.push_str(&format!("{k:?}: {}", render(&p)));
            }
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn reindex_old_versions_options_before_target() {
        assert_eq!(
            sql(100000, &tgt(Table), MaintenanceOp::Reindex(ReindexOptions { concurrently: false, verbose: true })),
            "REINDEX (VERBOSE) TABLE s.t"
        );
        assert_eq!(
            sql(120000, &tgt(Table), MaintenanceOp::Reindex(ReindexOptions { concurrently: true, verbose: true })),
            "REINDEX (VERBOSE) TABLE CONCURRENTLY s.t"
        );
        assert_eq!(sql(100000, &tgt(Database), MaintenanceOp::Reindex(rei())), "REINDEX DATABASE d");
    }

    #[test]
    fn cluster_snapshot() {
        let mut t = tgt(Table);
        t.cluster_index = Some("i2".into());
        let mut out = String::new();
        for o in [
            clu(),
            ClusterOptions { verbose: true, using_index: None },
            ClusterOptions { verbose: false, using_index: Some("i1".into()) },
            ClusterOptions { verbose: true, using_index: Some("i1".into()) },
        ] {
            let p = maintenance(&ctx(170000), &t, &MaintenanceOp::Cluster(o)).unwrap();
            out.push_str(&render(&p));
        }
        let mut t2 = tgt(Table);
        t2.partitioned = true;
        let o = ClusterOptions { verbose: false, using_index: Some("i1".into()) };
        out.push_str(&render(&maintenance(&ctx(150000), &t2, &MaintenanceOp::Cluster(o)).unwrap()));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn risk_and_notes() {
        let risk = |t: &MaintenanceTarget, op: MaintenanceOp| {
            let p = maintenance(&ctx(170000), t, &op).unwrap();
            (p.statements[0].risk, p.statements[0].note.clone(), p.warnings.len())
        };
        let lock = "Tablo boyunca ACCESS EXCLUSIVE kilit alır; okuma/yazma bekler.";
        let mut ct = tgt(Table);
        ct.cluster_index = Some("i1".into());
        assert_eq!(risk(&tgt(Table), MaintenanceOp::Vacuum(vac())), (Risk::Safe, None, 0));
        assert_eq!(
            risk(&tgt(Table), MaintenanceOp::Vacuum(VacuumOptions { full: true, ..vac() })),
            (Risk::Destructive, Some(lock.into()), 0)
        );
        assert_eq!(risk(&tgt(Table), MaintenanceOp::Analyze(ana())).0, Risk::Safe);
        for k in [Index, Table, Schema, Database] {
            let r = risk(&tgt(k), MaintenanceOp::Reindex(rei()));
            assert_eq!(r.0, Risk::Destructive, "{k:?}");
            let r = risk(&tgt(k), MaintenanceOp::Reindex(ReindexOptions { concurrently: true, verbose: false }));
            assert_eq!((r.0, r.1), (Risk::Safe, None), "{k:?}");
        }
        assert_eq!(risk(&ct, MaintenanceOp::Cluster(clu())), (Risk::Destructive, Some(lock.into()), 0));
        let rn = "Tabloda SHARE, indekste ACCESS EXCLUSIVE kilit alır; yazma ve indeksi kullanan okumalar bekler.";
        let r = risk(&tgt(Table), MaintenanceOp::Reindex(rei()));
        assert_eq!(r.1.as_deref(), Some(rn));
        for k in [Schema, Database] {
            let r = risk(&tgt(k), MaintenanceOp::Reindex(rei()));
            assert_eq!(r.1, Some(format!("{rn} Tablolar sırayla kilitlenir.")));
        }
    }

    #[test]
    fn ops_for_table() {
        assert_eq!(ops_for(Table, false, 100000), ["vacuum", "analyze", "reindex", "cluster"]);
        assert_eq!(ops_for(Table, true, 140000), ["vacuum", "analyze", "reindex"]);
        assert_eq!(ops_for(Table, true, 130000), ["vacuum", "analyze"]);
        assert!(ops_for(Index, true, 130000).is_empty());
        assert_eq!(ops_for(Index, true, 140000), ["reindex"]);
        assert_eq!(ops_for(Table, true, 150000), ["vacuum", "analyze", "reindex", "cluster"]);
        assert_eq!(ops_for(MaterializedView, false, 170000), ["vacuum", "analyze", "reindex"]);
        assert_eq!(ops_for(Index, false, 170000), ["reindex"]);
        assert_eq!(ops_for(Schema, false, 170000), ["reindex"]);
        assert_eq!(ops_for(Database, false, 170000), ["vacuum", "analyze", "reindex"]);
    }

    #[test]
    fn unsupported_combinations_error_on_op() {
        let cases = [
            (Index, MaintenanceOp::Vacuum(vac())),
            (Schema, MaintenanceOp::Vacuum(vac())),
            (Index, MaintenanceOp::Analyze(ana())),
            (Schema, MaintenanceOp::Analyze(ana())),
            (Index, MaintenanceOp::Cluster(clu())),
            (Schema, MaintenanceOp::Cluster(clu())),
            (Database, MaintenanceOp::Cluster(clu())),
            (MaterializedView, MaintenanceOp::Cluster(clu())),
        ];
        for (k, op) in cases {
            assert_eq!(paths(&errs(170000, &tgt(k), op)), ["op"], "{k:?}");
        }
    }

    #[test]
    fn not_editable_errors_on_object() {
        let mut t = tgt(Table);
        t.editable = false;
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Vacuum(vac()))), ["object"]);
    }

    #[test]
    fn vacuum_option_errors() {
        let o = VacuumOptions { full: true, parallel: Some(2), ..vac() };
        assert_eq!(paths(&errs(170000, &tgt(Table), MaintenanceOp::Vacuum(o))), ["options.parallel"]);
        let o = VacuumOptions { parallel: Some(1025), ..vac() };
        assert_eq!(paths(&errs(170000, &tgt(Table), MaintenanceOp::Vacuum(o))), ["options.parallel"]);
        let o = VacuumOptions { parallel: Some(1024), ..vac() };
        assert!(sql(170000, &tgt(Table), MaintenanceOp::Vacuum(o)).contains("PARALLEL 1024"));
    }

    #[test]
    fn version_gates() {
        let t = tgt(Table);
        let gate = |v_below: u32, v_ok: u32, path: &str, op: MaintenanceOp| {
            assert_eq!(paths(&errs(v_below, &t, op.clone())), [path], "{path} @{v_below}");
            assert!(maintenance(&ctx(v_ok), &t, &op).is_ok(), "{path} @{v_ok}");
        };
        gate(110000, 120000, "options.skipLocked", MaintenanceOp::Vacuum(VacuumOptions { skip_locked: true, ..vac() }));
        gate(
            110000,
            120000,
            "options.skipLocked",
            MaintenanceOp::Analyze(AnalyzeOptions { verbose: false, skip_locked: true }),
        );
        gate(
            110000,
            120000,
            "options.indexCleanup",
            MaintenanceOp::Vacuum(VacuumOptions { index_cleanup: Some(IndexCleanup::On), ..vac() }),
        );
        gate(120000, 130000, "options.parallel", MaintenanceOp::Vacuum(VacuumOptions { parallel: Some(2), ..vac() }));
        gate(
            130000,
            140000,
            "options.indexCleanup",
            MaintenanceOp::Vacuum(VacuumOptions { index_cleanup: Some(IndexCleanup::Auto), ..vac() }),
        );
        gate(
            110000,
            120000,
            "options.concurrently",
            MaintenanceOp::Reindex(ReindexOptions { concurrently: true, verbose: false }),
        );
        // ON/OFF PG 12'de serbest
        let off = VacuumOptions { index_cleanup: Some(IndexCleanup::Off), ..vac() };
        assert!(maintenance(&ctx(120000), &t, &MaintenanceOp::Vacuum(off)).is_ok());
        // bölümlenmiş CLUSTER PG 15+
        let mut p = tgt(Table);
        p.partitioned = true;
        p.cluster_index = Some("i1".into());
        assert_eq!(paths(&errs(140000, &p, MaintenanceOp::Cluster(clu()))), ["op"]);
        assert!(maintenance(&ctx(150000), &p, &MaintenanceOp::Cluster(clu())).is_ok());
    }

    #[test]
    fn analyze_pg10_has_no_parens() {
        let v = AnalyzeOptions { verbose: true, skip_locked: false };
        assert_eq!(sql(100000, &tgt(Table), MaintenanceOp::Analyze(v.clone())), "ANALYZE VERBOSE s.t");
        assert_eq!(sql(100000, &tgt(Database), MaintenanceOp::Analyze(v)), "ANALYZE VERBOSE");
        assert_eq!(sql(100000, &tgt(Table), MaintenanceOp::Analyze(ana())), "ANALYZE s.t");
        assert_eq!(
            sql(110000, &tgt(Table), MaintenanceOp::Analyze(AnalyzeOptions { verbose: true, skip_locked: false })),
            "ANALYZE (VERBOSE) s.t"
        );
    }

    #[test]
    fn reindex_partitioned_needs_pg14() {
        let mut t = tgt(Table);
        t.partitioned = true;
        assert_eq!(paths(&errs(130000, &t, MaintenanceOp::Reindex(rei()))), ["op"]);
        assert!(maintenance(&ctx(140000), &t, &MaintenanceOp::Reindex(rei())).is_ok());
        let mut i = tgt(Index);
        i.partitioned = true;
        let e = errs(130000, &i, MaintenanceOp::Reindex(rei()));
        assert_eq!(paths(&e), ["op"]);
        assert!(e[0].message.contains("PostgreSQL 14 ve üzerini gerektirir"));
        assert!(maintenance(&ctx(140000), &i, &MaintenanceOp::Reindex(rei())).is_ok());
    }

    #[test]
    fn disable_page_skipping_with_full_errors() {
        let o = VacuumOptions { full: true, disable_page_skipping: true, ..vac() };
        assert_eq!(paths(&errs(170000, &tgt(Table), MaintenanceOp::Vacuum(o))), ["options.disablePageSkipping"]);
    }

    #[test]
    fn cluster_index_rules() {
        let t = tgt(Table);
        // kümeleme indeksi yok, USING yok
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Cluster(clu()))), ["options.usingIndex"]);
        // bilinmeyen indeks
        let o = ClusterOptions { verbose: false, using_index: Some("yok".into()) };
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Cluster(o))), ["options.usingIndex"]);
        // kümeleme indeksi varsa USING'siz serbest
        let mut c = tgt(Table);
        c.cluster_index = Some("i2".into());
        assert_eq!(sql(170000, &c, MaintenanceOp::Cluster(clu())), "CLUSTER s.t");
    }

    #[test]
    fn system_objects() {
        let mut t = tgt(Table);
        t.schema = Some("pg_catalog".into());
        t.name = "pg_class".into();
        t.system = true;
        t.cluster_index = Some("i1".into());
        assert_eq!(sql(170000, &t, MaintenanceOp::Vacuum(vac())), "VACUUM pg_catalog.pg_class");
        assert_eq!(sql(170000, &t, MaintenanceOp::Analyze(ana())), "ANALYZE pg_catalog.pg_class");
        assert_eq!(sql(170000, &t, MaintenanceOp::Reindex(rei())), "REINDEX TABLE pg_catalog.pg_class");
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Cluster(clu()))), ["op"]);
        let o = ReindexOptions { concurrently: true, verbose: false };
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Reindex(o))), ["options.concurrently"]);
        let mut s = tgt(Schema);
        s.name = "pg_catalog".into();
        s.system = true;
        let o = ReindexOptions { concurrently: true, verbose: false };
        assert_eq!(paths(&errs(170000, &s, MaintenanceOp::Reindex(o))), ["options.concurrently"]);
    }

    #[test]
    fn hostile_names_are_quoted() {
        for n in ["a\"b", "ü ş 名", "x y;z", "a--b", "it's /* x"] {
            let mut t = tgt(Table);
            t.schema = Some(n.into());
            t.name = n.into();
            t.indexes = vec![n.into()];
            let s = sql(170000, &t, MaintenanceOp::Vacuum(vac()));
            assert!(s.starts_with("VACUUM "), "{s}");
            let o = ClusterOptions { verbose: false, using_index: Some(n.into()) };
            sql(170000, &t, MaintenanceOp::Cluster(o));
            for k in [Schema, Database, Index] {
                let mut t = tgt(k);
                t.name = n.into();
                t.schema = (k == Index).then(|| n.into());
                sql(170000, &t, MaintenanceOp::Reindex(rei()));
            }
        }
        let mut t = tgt(Table);
        t.name = "a\"b".into();
        assert_eq!(sql(170000, &t, MaintenanceOp::Analyze(ana())), "ANALYZE s.\"a\"\"b\"");
    }

    #[test]
    fn bad_loaded_names_rejected() {
        let mut t = tgt(Table);
        t.name = String::new();
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Vacuum(vac()))), ["object"]);
        let mut t = tgt(Table);
        t.name = "a\0b".into();
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Vacuum(vac()))), ["object"]);
        let mut t = tgt(Table);
        t.schema = None;
        assert_eq!(paths(&errs(170000, &t, MaintenanceOp::Vacuum(vac()))), ["object"]);
    }
}
