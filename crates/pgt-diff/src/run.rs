//! Karşılaştırma yürütme: iki tarafı eşzamanlı yükler, eşleştirir, durumu belirler, öğe başına DDL üretir
//! ve sonucu (`DiffData`) bellekte tutar; ayrıntı ve betik istekleri bu veriden saf olarak yanıtlanır.
//!
//! Günlüğe yalnız sayımlar, süre ve sonuç yazılır (ad, SQL, yol yok).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use pgt_core::connection::ConnectionManager;
use pgt_ddl::build::BuildCtx;
use pgt_ddl::{ConstraintKind, ObjectDef, ObjectKind};

use crate::cancel::CancelToken;
use crate::generate::{GenFailure, Generated, append_mv_indexes, generate};
use crate::key::{KeyClass, Listed, Pair, class_of, match_objects};
use crate::linediff::{ddl_text, line_diff};
use crate::list::SchemaCheck;
use crate::model::{
    DiffDetail, DiffEvent, DiffItem, DiffOptions, DiffResult, DiffScript, DiffSide, ItemProblem, ItemStatus,
    SideTag,
};
use crate::normalize::{NormCtx, Prepared, prepare};
use crate::script::{ScriptHeader, ScriptItem, assemble, routine_type_refs, type_refs};
use crate::snapshot::{Loaded, SideSnapshot, SnapshotError, load_side};
use pgt_ddl::Risk;

const SCHEMA_NAME_WARNING: &str = "Gövdelerdeki nitelikli adları gözden geçirin: kaynak şemayı açıkça niteleyen adlar çevrilmez.";

/// Olay alıcısı. `false` dönerse istemci gitmiştir: iş iptal edilir.
pub trait DiffSink: Send {
    fn send(&mut self, e: DiffEvent) -> bool;
}

#[derive(Debug)]
pub enum RunError {
    SameSchema,
    Cancelled,
    Schema { side: SideTag, check: SchemaCheck },
    Core(pgt_core::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptError {
    /// Seçim boş.
    Empty,
    UnknownKey,
    NotSelectable,
}

/// Aynı sunucu, veritabanı oid'i ve şema adı (bayt düzeyinde, büyük/küçük harf duyarlı).
pub fn same_side(a: &DiffSide, b: &DiffSide) -> bool {
    a.server_id == b.server_id
        && a.database_oid == b.database_oid
        && a.schema.as_bytes() == b.schema.as_bytes()
}

/// Bir öğenin saklanan tüm durumu: hazırlanmış tanımlar, üretim sonucu ve bağımlılık anahtarları.
pub struct ItemEntry {
    pub prepared: Prepared,
    pub generated: Result<Generated, GenFailure>,
    pub kind: ObjectKind,
    pub target_kind: Option<ObjectKind>,
    pub name: String,
    pub table: Option<String>,
    /// Kaynakta bağımlı olunan öğe anahtarları (görünüm/MV, tip/domain).
    pub source_deps: Vec<String>,
    /// Hedefte bağımlı olunan öğe anahtarları (görünüm/MV, tablo FK).
    pub target_deps: Vec<String>,
    /// Rutin için: kaynak imzasında satır tipine başvurulan tablo/görünüm/MV anahtarları.
    pub rowtype_deps: Vec<String>,
}

/// Betik başlığı için taraf bilgileri (sunucu adresi/kullanıcı yok).
#[derive(Debug, Clone, Default)]
pub struct HeaderInfo {
    pub source_db: String,
    pub source_schema: String,
    pub target_db: String,
    pub target_schema: String,
    pub target_version: String,
}

/// Bitmiş karşılaştırma. `entries[i]`, `result.items[i]` öğesine karşılık gelir.
pub struct DiffData {
    result: DiffResult,
    entries: Vec<ItemEntry>,
    index: HashMap<String, usize>,
    header: HeaderInfo,
    ctx: BuildCtx,
    last_script: Mutex<Option<String>>,
}

impl DiffData {
    /// Sunucusuz kurulum (testler için). `result.items` ile `entries` aynı uzunlukta ve sırada olmalı.
    #[doc(hidden)]
    pub fn from_parts(
        result: DiffResult,
        entries: Vec<ItemEntry>,
        header: HeaderInfo,
        ctx: BuildCtx,
    ) -> Self {
        assert_eq!(result.items.len(), entries.len(), "öğe ve girdi sayısı eşit olmalı");
        let index = result
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.key.clone(), i))
            .collect();
        DiffData {
            result,
            entries,
            index,
            header,
            ctx,
            last_script: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self::from_parts(
            DiffResult {
                items: vec![],
                source_version: String::new(),
                target_version: String::new(),
                warnings: vec![],
                elapsed_ms: 0,
            },
            vec![],
            HeaderInfo::default(),
            BuildCtx::default(),
        )
    }

    pub fn result(&self) -> &DiffResult {
        &self.result
    }

    /// İki tarafın DDL metni (hedef sürüm bağlamıyla) ve satır farkı. Bilinmeyen anahtarda `None`.
    pub fn detail(&self, key: &str) -> Option<DiffDetail> {
        let e = &self.entries[*self.index.get(key)?];
        let text = |d: &Option<ObjectDef>| d.as_ref().map(|d| ddl_text(d, &self.ctx)).unwrap_or_default();
        let source_text = text(&e.prepared.source);
        let target_text = text(&e.prepared.target);
        let lines = line_diff(&source_text, &target_text);
        Some(DiffDetail {
            source_text,
            target_text,
            lines,
        })
    }

    /// Seçili öğelerin betiği. Başarıda `last_script` güncellenir.
    pub fn script(&self, keys: &[String], generated_at: &str) -> Result<DiffScript, ScriptError> {
        if keys.is_empty() {
            return Err(ScriptError::Empty);
        }
        let mut picked: Vec<usize> = vec![];
        let mut seen: HashSet<usize> = HashSet::new();
        for k in keys {
            let i = *self.index.get(k.as_str()).ok_or(ScriptError::UnknownKey)?;
            if !self.result.items[i].selectable || self.entries[i].generated.is_err() {
                return Err(ScriptError::NotSelectable);
            }
            if seen.insert(i) {
                picked.push(i);
            }
        }
        let selected: HashSet<&str> = picked.iter().map(|&i| self.result.items[i].key.as_str()).collect();
        fn deps<'a>(v: &'a [String], selected: &HashSet<&str>) -> Vec<&'a str> {
            v.iter().map(String::as_str).filter(|k| selected.contains(k)).collect()
        }
        let items: Vec<ScriptItem> = picked
            .iter()
            .filter_map(|&i| {
                let e = &self.entries[i];
                let generated = e.generated.as_ref().ok()?;
                Some(ScriptItem {
                    key: self.result.items[i].key.as_str(),
                    kind: e.kind,
                    target_kind: e.target_kind,
                    name: &e.name,
                    table: e.table.as_deref(),
                    generated,
                    source_deps: deps(&e.source_deps, &selected),
                    target_deps: deps(&e.target_deps, &selected),
                    rowtype_deps: deps(&e.rowtype_deps, &selected),
                })
            })
            .collect();
        let h = &self.header;
        let script = assemble(
            &ScriptHeader {
                source_db: &h.source_db,
                source_schema: &h.source_schema,
                target_db: &h.target_db,
                target_schema: &h.target_schema,
                target_version: &h.target_version,
                generated_at,
            },
            &items,
        );
        *self.last_script.lock().unwrap_or_else(|e| e.into_inner()) = Some(script.sql.clone());
        Ok(script)
    }

    /// Son başarıyla üretilen betik.
    pub fn last_script(&self) -> Option<String> {
        self.last_script.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// Nitelikli tip adından yalnız şema-dışı ad (`şema.ad` -> `ad`; şema bilinen şemalardan biriyse).
fn unqualified<'a>(name: &'a str, schemas: &[&str]) -> &'a str {
    for s in schemas {
        if let Some(rest) = name.strip_prefix(s).and_then(|r| r.strip_prefix('.')) {
            return rest;
        }
    }
    name
}

/// Bir tarafın bağımlılık çözümleyicisi: oid -> görünüm/MV anahtarı, ad -> sınıf anahtarı.
struct DepIndex {
    view_by_oid: HashMap<u32, String>,
    type_by_name: HashMap<String, String>,
    relation_by_name: HashMap<String, String>,
    /// Satır tipi olan ilişkiler (tablo, görünüm, MV): ad -> anahtar.
    rowtype_by_name: HashMap<String, String>,
}

impl DepIndex {
    fn new(listed: &[Listed]) -> Self {
        let mut d = DepIndex {
            view_by_oid: HashMap::new(),
            type_by_name: HashMap::new(),
            relation_by_name: HashMap::new(),
            rowtype_by_name: HashMap::new(),
        };
        for l in listed {
            let key = crate::key::item_key(l);
            if matches!(
                l.kind,
                ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
            ) {
                d.rowtype_by_name.entry(l.name.clone()).or_insert(key.clone());
            }
            match (l.kind, class_of(l.kind)) {
                (ObjectKind::View | ObjectKind::MaterializedView, _) => {
                    d.view_by_oid.insert(l.oid, key.clone());
                    d.relation_by_name.entry(l.name.clone()).or_insert(key);
                }
                (_, Some(KeyClass::Relation)) => {
                    d.relation_by_name.entry(l.name.clone()).or_insert(key);
                }
                (_, Some(KeyClass::Type)) => {
                    d.type_by_name.entry(l.name.clone()).or_insert(key);
                }
                _ => {}
            }
        }
        d
    }

    fn view_deps(&self, l: &Listed) -> Vec<String> {
        if !matches!(l.kind, ObjectKind::View | ObjectKind::MaterializedView) {
            return vec![];
        }
        l.deps.iter().filter_map(|o| self.view_by_oid.get(o).cloned()).collect()
    }

    fn type_deps(&self, def: Option<&ObjectDef>, schemas: &[&str]) -> Vec<String> {
        let Some(def) = def else { return vec![] };
        type_refs(def)
            .iter()
            .filter_map(|n| self.type_by_name.get(unqualified(n, schemas)).cloned())
            .collect()
    }

    /// Rutin imzasındaki (yüklenen `format_type` adları) satır tipi başvuruları.
    fn rowtype_deps(&self, def: Option<&ObjectDef>, schemas: &[&str]) -> Vec<String> {
        let Some(def) = def else { return vec![] };
        let mut out: Vec<String> = vec![];
        for n in routine_type_refs(def) {
            if let Some(k) = self.rowtype_by_name.get(unqualified(&n, schemas))
                && !out.contains(k)
            {
                out.push(k.clone());
            }
        }
        out
    }

    fn fk_deps(&self, def: Option<&ObjectDef>, schema: &str) -> Vec<String> {
        let Some(ObjectDef::Table(t)) = def else { return vec![] };
        t.constraints
            .iter()
            .filter_map(|c| match &c.kind {
                ConstraintKind::ForeignKey {
                    ref_schema,
                    ref_table,
                    ..
                } if ref_schema == schema => self.relation_by_name.get(ref_table).cloned(),
                _ => None,
            })
            .collect()
    }
}

fn major(version_num: u32) -> u32 {
    version_num / 10_000
}

struct Stats {
    source_objects: usize,
    target_objects: usize,
}

/// Karşılaştırmayı yürütür. İptal (`cancel` ya da `sink` false) -> `Cancelled`.
pub async fn run_diff(
    mgr: &ConnectionManager,
    source: &DiffSide,
    target: &DiffSide,
    options: DiffOptions,
    sink: &mut dyn DiffSink,
    cancel: &CancelToken,
) -> Result<DiffData, RunError> {
    let started = Instant::now();
    let mut stats = Stats {
        source_objects: 0,
        target_objects: 0,
    };
    let r = run_inner(mgr, source, target, options, sink, cancel, &mut stats, started).await;
    let outcome = match &r {
        Ok(_) => "ok",
        Err(RunError::Cancelled) => "cancelled",
        Err(_) => "failed",
    };
    let (items, problems) = r
        .as_ref()
        .map(|d| {
            let it = &d.result.items;
            (it.len(), it.iter().filter(|i| i.problem.is_some()).count())
        })
        .unwrap_or((0, 0));
    tracing::info!(
        job = "schema_diff",
        outcome,
        source_objects = stats.source_objects,
        target_objects = stats.target_objects,
        items,
        problems,
        elapsed_ms = started.elapsed().as_millis() as u64,
    );
    r
}

fn snap_err(side: SideTag, e: SnapshotError) -> RunError {
    match e {
        SnapshotError::Cancelled => RunError::Cancelled,
        SnapshotError::Schema(check) => RunError::Schema { side, check },
        SnapshotError::Core(e) => RunError::Core(e),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    mgr: &ConnectionManager,
    source: &DiffSide,
    target: &DiffSide,
    options: DiffOptions,
    sink: &mut dyn DiffSink,
    cancel: &CancelToken,
    stats: &mut Stats,
    started: Instant,
) -> Result<DiffData, RunError> {
    if same_side(source, target) {
        return Err(RunError::SameSchema);
    }
    let sink = Mutex::new(sink);
    let reporter = |side: SideTag| {
        let sink = &sink;
        move |loaded: u32, total: u32| -> bool {
            sink.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(DiffEvent::Progress { side, loaded, total })
        }
    };
    let (rs, rt) = (reporter(SideTag::Source), reporter(SideTag::Target));
    let with_acl = options.ownership;
    let (src, tgt) = tokio::try_join!(
        async {
            load_side(mgr, source, with_acl, &rs, cancel)
                .await
                .map_err(|e| snap_err(SideTag::Source, e))
        },
        async {
            load_side(mgr, target, with_acl, &rt, cancel)
                .await
                .map_err(|e| snap_err(SideTag::Target, e))
        },
    )?;
    stats.source_objects = src.objects.len();
    stats.target_objects = tgt.objects.len();
    build_data(source, target, options, src, tgt, cancel, started)
}

fn lookup(objects: &[Loaded]) -> HashMap<(ObjectKind, u32), &Loaded> {
    objects.iter().map(|l| ((l.listed.kind, l.listed.oid), l)).collect()
}

type Staged = (Prepared, Result<Generated, GenFailure>);

/// Kimlik: kaynak varsa kaynağın, yoksa hedefin listesi satırı.
fn identity<'a>(pair: &Pair<'a>) -> &'a Listed {
    pair.source.or(pair.target).expect("en az bir taraf dolu")
}

/// Yeniden kurulan (DROP + CREATE ya da tür değişimi) MV'lerin değişmeyen indekslerini MV öğesinin betiğine ekler:
/// DROP MATERIALIZED VIEW indeksleri de düşürür, `Same` indeks öğeleri ise betiğe girmez.
fn recreate_mv_indexes(pairs: &[Pair], staged: &mut [Staged], ctx: &BuildCtx) {
    for i in 0..staged.len() {
        let mv = identity(&pairs[i]);
        let (p, g) = &staged[i];
        let rebuilt = mv.kind == ObjectKind::MaterializedView
            && p.status == ItemStatus::Different
            && g.as_ref().is_ok_and(|g| g.rebuilt);
        if !rebuilt {
            continue;
        }
        let indexes: Vec<Prepared> = pairs
            .iter()
            .zip(staged.iter())
            .filter(|(pair, (ip, _))| {
                let l = identity(pair);
                l.kind == ObjectKind::Index
                    && l.table.as_deref() == Some(mv.name.as_str())
                    && ip.status == ItemStatus::Same
            })
            .map(|(_, (ip, _))| ip.clone())
            .collect();
        if indexes.is_empty() {
            continue;
        }
        let refs: Vec<&Prepared> = indexes.iter().collect();
        let g = &mut staged[i].1;
        if let Ok(gen_) = g
            && let Err(f) = append_mv_indexes(gen_, &refs, ctx)
        {
            *g = Err(f);
        }
    }
}

fn build_data(
    source: &DiffSide,
    target: &DiffSide,
    options: DiffOptions,
    src: SideSnapshot,
    tgt: SideSnapshot,
    cancel: &CancelToken,
    started: Instant,
) -> Result<DiffData, RunError> {
    let src_listed: Vec<Listed> = src.objects.iter().map(|l| l.listed.clone()).collect();
    let tgt_listed: Vec<Listed> = tgt.objects.iter().map(|l| l.listed.clone()).collect();
    let src_by = lookup(&src.objects);
    let tgt_by = lookup(&tgt.objects);
    let src_deps = DepIndex::new(&src_listed);
    let tgt_deps = DepIndex::new(&tgt_listed);
    let cx = NormCtx {
        source_schema: &source.schema,
        target_schema: &target.schema,
        options,
    };
    let ctx = BuildCtx {
        version_num: tgt.version_num,
        concurrently: false,
    };
    let schemas = [source.schema.as_str(), target.schema.as_str()];
    let mut items: Vec<DiffItem> = vec![];
    let mut entries: Vec<ItemEntry> = vec![];
    let pairs = match_objects(&src_listed, &tgt_listed);
    let mut staged: Vec<Staged> = Vec::with_capacity(pairs.len());
    for pair in &pairs {
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let sl = pair.source.and_then(|l| src_by.get(&(l.kind, l.oid)).copied());
        let tl = pair.target.and_then(|l| tgt_by.get(&(l.kind, l.oid)).copied());
        let prepared = prepare(pair.key.clone(), sl, tl, &cx);
        let generated = generate(&prepared, &ctx, &tgt.viewer);
        staged.push((prepared, generated));
    }
    recreate_mv_indexes(&pairs, &mut staged, &ctx);
    for (pair, (prepared, generated)) in pairs.iter().zip(staged) {
        let id = identity(pair);
        let target_kind = if prepared.kind_changed {
            pair.target.map(|l| l.kind)
        } else {
            None
        };
        let mut notes = prepared.notes.clone();
        let mut errors = prepared.errors.clone();
        let (problem, risk) = match &generated {
            Ok(g) => {
                for n in g.notes.iter().chain(&g.warnings) {
                    push_unique(&mut notes, n);
                }
                (prepared.problem, g.risk)
            }
            Err(f) => {
                if prepared.problem.is_none() {
                    match f.problem {
                        ItemProblem::BuildFailed => errors = f.messages.clone(),
                        _ => {
                            for m in &f.messages {
                                push_unique(&mut notes, m);
                            }
                        }
                    }
                }
                (Some(f.problem), Risk::Safe)
            }
        };
        let mut item = DiffItem {
            key: pair.key.clone(),
            kind: id.kind,
            target_kind,
            name: id.name.clone(),
            table: id.table.clone(),
            signature: id.signature.clone(),
            status: prepared.status,
            problem,
            risk,
            notes,
            errors,
            selectable: false,
            default_selected: false,
        };
        item.finalize_selection();
        let mut source_deps = pair.source.map(|l| src_deps.view_deps(l)).unwrap_or_default();
        source_deps.extend(src_deps.type_deps(prepared.source.as_ref(), &schemas));
        let mut target_deps = pair.target.map(|l| tgt_deps.view_deps(l)).unwrap_or_default();
        target_deps.extend(tgt_deps.fk_deps(prepared.target.as_ref(), &target.schema));
        let rowtype_deps = src_deps.rowtype_deps(prepared.source.as_ref(), &schemas);
        entries.push(ItemEntry {
            kind: item.kind,
            target_kind,
            name: item.name.clone(),
            table: item.table.clone(),
            prepared,
            generated,
            source_deps,
            target_deps,
            rowtype_deps,
        });
        items.push(item);
    }
    let mut warnings = vec![];
    if source.schema.as_bytes() != target.schema.as_bytes() {
        warnings.push(SCHEMA_NAME_WARNING.to_string());
    }
    if major(src.version_num) != major(tgt.version_num) {
        warnings.push(format!(
            "Kaynak PostgreSQL {}, hedef {}: ifadeler hedef sürüme göre üretilir.",
            src.version, tgt.version
        ));
    }
    let result = DiffResult {
        items,
        source_version: src.version.clone(),
        target_version: tgt.version.clone(),
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    let header = HeaderInfo {
        source_db: src.database,
        source_schema: source.schema.clone(),
        target_db: tgt.database,
        target_schema: target.schema.clone(),
        target_version: tgt.version,
    };
    Ok(DiffData::from_parts(result, entries, header, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Phase;
    use crate::model::ItemStatus;
    use pgt_ddl::Statement;
    use uuid::Uuid;

    fn side(schema: &str) -> DiffSide {
        DiffSide {
            server_id: Uuid::nil(),
            database_oid: 5,
            schema: schema.into(),
        }
    }

    #[test]
    fn same_side_is_case_sensitive_and_full() {
        assert!(same_side(&side("s"), &side("s")));
        assert!(!same_side(&side("S"), &side("s")));
        let mut other_db = side("s");
        other_db.database_oid = 6;
        assert!(!same_side(&side("s"), &other_db));
        let mut other_srv = side("s");
        other_srv.server_id = Uuid::from_u128(1);
        assert!(!same_side(&side("s"), &other_srv));
    }

    #[test]
    fn unqualified_strips_known_schemas_only() {
        assert_eq!(unqualified("a.t", &["a", "b"]), "t");
        assert_eq!(unqualified("c.t", &["a", "b"]), "c.t");
        assert_eq!(unqualified("t", &["a"]), "t");
    }

    fn prepared(key: &str) -> Prepared {
        Prepared {
            key: key.into(),
            status: ItemStatus::OnlySource,
            problem: None,
            notes: vec![],
            errors: vec![],
            kind_changed: false,
            source: None,
            target: None,
            source_acl: None,
            target_acl: None,
            target_owner: None,
            target_comment: None,
        }
    }

    fn generated(sql: &str) -> Generated {
        Generated {
            parts: vec![(
                Phase::Tables,
                vec![Statement {
                    sql: sql.into(),
                    risk: Risk::Safe,
                    transactional: true,
                    note: None,
                    redacted: false,
                }],
            )],
            warnings: vec![],
            risk: Risk::Safe,
            notes: vec![],
            drops_table: false,
            dropped_columns: vec![],
            sequence_owner: None,
            rebuilt: false,
        }
    }

    fn entry(key: &str, gen_: Result<Generated, GenFailure>) -> ItemEntry {
        ItemEntry {
            prepared: prepared(key),
            generated: gen_,
            kind: ObjectKind::Table,
            target_kind: None,
            name: key.into(),
            table: None,
            source_deps: vec![],
            target_deps: vec![],
            rowtype_deps: vec![],
        }
    }

    fn item(key: &str, status: ItemStatus, problem: Option<ItemProblem>) -> DiffItem {
        let mut it = DiffItem {
            key: key.into(),
            kind: ObjectKind::Table,
            target_kind: None,
            name: key.into(),
            table: None,
            signature: None,
            status,
            problem,
            risk: Risk::Safe,
            notes: vec![],
            errors: vec![],
            selectable: false,
            default_selected: false,
        };
        it.finalize_selection();
        it
    }

    fn data() -> DiffData {
        let items = vec![
            item("ok", ItemStatus::OnlySource, None),
            item("same", ItemStatus::Same, None),
            item("man", ItemStatus::OnlySource, Some(ItemProblem::Manual)),
        ];
        let entries = vec![
            entry("ok", Ok(generated("CREATE TABLE ok ()"))),
            entry("same", Ok(generated("-- yok"))),
            entry(
                "man",
                Err(GenFailure {
                    problem: ItemProblem::Manual,
                    messages: vec!["elle".into()],
                }),
            ),
        ];
        DiffData::from_parts(
            DiffResult {
                items,
                source_version: "16.1".into(),
                target_version: "16.2".into(),
                warnings: vec![],
                elapsed_ms: 0,
            },
            entries,
            HeaderInfo {
                source_db: "a".into(),
                source_schema: "s".into(),
                target_db: "b".into(),
                target_schema: "t".into(),
                target_version: "16.2".into(),
            },
            BuildCtx {
                version_num: 160002,
                concurrently: false,
            },
        )
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn script_errors() {
        let d = data();
        assert_eq!(d.script(&[], "t").unwrap_err(), ScriptError::Empty);
        assert_eq!(d.script(&keys(&["yok"]), "t").unwrap_err(), ScriptError::UnknownKey);
        assert_eq!(d.script(&keys(&["same"]), "t").unwrap_err(), ScriptError::NotSelectable);
        assert_eq!(d.script(&keys(&["man"]), "t").unwrap_err(), ScriptError::NotSelectable);
        assert_eq!(
            d.script(&keys(&["ok", "man"]), "t").unwrap_err(),
            ScriptError::NotSelectable
        );
        assert!(d.last_script().is_none(), "hatada last_script dolmaz");
    }

    #[test]
    fn script_success_fills_last_script_and_dedups() {
        let d = data();
        let s = d.script(&keys(&["ok", "ok"]), "2026-01-01 00:00:00 UTC").unwrap();
        assert_eq!(s.statements, 1);
        assert!(s.sql.contains("CREATE TABLE ok ()"));
        assert_eq!(d.last_script().as_deref(), Some(s.sql.as_str()));
    }

    #[test]
    fn detail_unknown_key_is_none() {
        let d = data();
        assert!(d.detail("yok").is_none());
        let det = d.detail("ok").unwrap();
        assert_eq!(det.source_text, "");
        assert_eq!(det.target_text, "");
    }

    fn listed(kind: ObjectKind, oid: u32, name: &str, deps: Vec<u32>) -> Listed {
        Listed {
            kind,
            oid,
            name: name.into(),
            table: None,
            signature: None,
            is_partition: false,
            deps,
        }
    }

    #[test]
    fn view_deps_map_oids_to_view_and_mv_keys_per_side() {
        let side_a = [
            listed(ObjectKind::View, 10, "v1", vec![]),
            listed(ObjectKind::MaterializedView, 11, "m1", vec![10]),
            listed(ObjectKind::Table, 12, "t", vec![]),
            listed(ObjectKind::View, 13, "v2", vec![10, 11, 12, 99]),
        ];
        // Öbür tarafta aynı adlar başka oid'lerle: oid -> anahtar çözümü tarafa özgüdür.
        let side_b = [
            listed(ObjectKind::View, 20, "v1", vec![]),
            listed(ObjectKind::View, 21, "v2", vec![20, 10]),
        ];
        let (a, b) = (DepIndex::new(&side_a), DepIndex::new(&side_b));
        let key = |l: &Listed| crate::key::item_key(l);
        assert_eq!(
            a.view_deps(&side_a[3]),
            [key(&side_a[0]), key(&side_a[1])],
            "tablo ve bilinmeyen oid atlanır"
        );
        assert_eq!(a.view_deps(&side_a[1]), [key(&side_a[0])]);
        assert_eq!(b.view_deps(&side_b[1]), [key(&side_b[0])], "10 bu tarafta yok");
        assert!(a.view_deps(&side_a[2]).is_empty(), "görünüm olmayan öğenin bağımlılığı yok");
    }

    fn type_def(name: &str, attr_types: &[&str]) -> ObjectDef {
        ObjectDef::Type(pgt_ddl::TypeDef {
            schema: "s".into(),
            name: name.into(),
            owner: None,
            comment: None,
            body: pgt_ddl::TypeBody::Composite {
                attributes: attr_types
                    .iter()
                    .map(|t| pgt_ddl::CompositeAttr {
                        name: "x".into(),
                        type_name: (*t).into(),
                        collation: None,
                        original: None,
                    })
                    .collect(),
            },
            editable: true,
            unsupported: vec![],
        })
    }

    #[test]
    fn type_deps_strip_known_schema_prefix_and_quotes() {
        let l = [
            listed(ObjectKind::Type, 1, "b", vec![]),
            listed(ObjectKind::Domain, 2, "Dom X", vec![]),
            listed(ObjectKind::Table, 3, "tab", vec![]),
        ];
        let idx = DepIndex::new(&l);
        let schemas = ["s", "t"];
        let def = type_def("a", &["b", "s.b", "t.\"Dom X\"", "other.b", "tab", "b(10)[]", "int4"]);
        let kb = crate::key::item_key(&l[0]);
        let kd = crate::key::item_key(&l[1]);
        assert_eq!(
            idx.type_deps(Some(&def), &schemas),
            [kb.clone(), kb.clone(), kd, kb],
            "bilinmeyen şema, tablo ve yerleşik tipler atlanır"
        );
        assert!(idx.type_deps(None, &schemas).is_empty());
    }

    fn fk_con(ref_schema: &str, ref_table: &str) -> pgt_ddl::ConstraintDef {
        pgt_ddl::ConstraintDef {
            oid: None,
            name: Some("fk".into()),
            kind: ConstraintKind::ForeignKey {
                columns: vec!["c".into()],
                ref_schema: ref_schema.into(),
                ref_table: ref_table.into(),
                ref_columns: vec!["id".into()],
                match_type: pgt_ddl::FkMatch::Simple,
                on_update: pgt_ddl::FkAction::NoAction,
                on_delete: pgt_ddl::FkAction::NoAction,
            },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        }
    }

    #[test]
    fn fk_deps_only_for_target_schema_references() {
        let l = [
            listed(ObjectKind::Table, 1, "p", vec![]),
            listed(ObjectKind::View, 2, "pv", vec![]),
        ];
        let idx = DepIndex::new(&l);
        let table = ObjectDef::Table(pgt_ddl::TableDef {
            schema: "tgt".into(),
            name: "c".into(),
            owner: None,
            tablespace: None,
            persistence: pgt_ddl::Persistence::Permanent,
            columns: vec![],
            constraints: vec![
                fk_con("tgt", "p"),
                fk_con("baska", "p"),
                fk_con("tgt", "yok"),
                fk_con("tgt", "p"),
            ],
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        });
        let kp = crate::key::item_key(&l[0]);
        assert_eq!(idx.fk_deps(Some(&table), "tgt"), [kp.clone(), kp]);
        assert!(idx.fk_deps(Some(&table), "baska2").is_empty());
        assert!(idx.fk_deps(None, "tgt").is_empty());
        assert!(idx.fk_deps(Some(&type_def("a", &[])), "tgt").is_empty());
    }

    // --- build_data: satır tipine bağlı rutinler ve yeniden kurulan MV'nin indeksleri ---

    fn loaded(kind: ObjectKind, oid: u32, name: &str, table: Option<&str>, sig: Option<&str>, deps: Vec<u32>, def: ObjectDef) -> Loaded {
        Loaded {
            listed: Listed {
                kind,
                oid,
                name: name.into(),
                table: table.map(Into::into),
                signature: sig.map(Into::into),
                is_partition: false,
                deps,
            },
            def: Ok(def),
            acl: None,
        }
    }

    fn snap(objects: Vec<Loaded>) -> SideSnapshot {
        SideSnapshot {
            version_num: 160004,
            version: "16.4".into(),
            database: "db".into(),
            viewer: "app".into(),
            objects,
        }
    }

    fn rt_table(schema: &str) -> ObjectDef {
        ObjectDef::Table(pgt_ddl::TableDef {
            schema: schema.into(),
            name: "rt".into(),
            owner: Some("app".into()),
            tablespace: None,
            persistence: pgt_ddl::Persistence::Permanent,
            columns: vec![pgt_ddl::ColumnDef {
                attnum: Some(1),
                name: "id".into(),
                data_type: "integer".into(),
                not_null: false,
                default: None,
                identity: None,
                generated: None,
                collation: None,
                comment: None,
                using: None,
            }],
            constraints: vec![],
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        })
    }

    fn rt_view(schema: &str) -> ObjectDef {
        ObjectDef::View(pgt_ddl::ViewDef {
            schema: schema.into(),
            name: "rv".into(),
            owner: Some("app".into()),
            query: " SELECT rt.id FROM rt".into(),
            check_option: None,
            security_barrier: false,
            security_invoker: false,
            comment: None,
            recreate: false,
            columns: vec![],
            unsupported: vec![],
        })
    }

    fn rt_func(schema: &str, name: &str, args: &[&str], returns: pgt_ddl::FunctionReturn) -> ObjectDef {
        ObjectDef::Function(pgt_ddl::FunctionDef {
            schema: schema.into(),
            name: name.into(),
            owner: Some("app".into()),
            comment: None,
            language: "sql".into(),
            args: args
                .iter()
                .map(|t| pgt_ddl::FunctionArg {
                    mode: pgt_ddl::ArgMode::In,
                    name: None,
                    type_name: (*t).into(),
                    default: None,
                })
                .collect(),
            returns: Some(returns),
            body: "SELECT 1".into(),
            volatility: pgt_ddl::Volatility::Volatile,
            strict: false,
            security_definer: false,
            leakproof: false,
            parallel: pgt_ddl::Parallel::Unsafe,
            cost: None,
            rows: None,
            settings: vec![],
            recreate: false,
            editable: true,
            unsupported: vec![],
        })
    }

    fn rt_mv(schema: &str, query: &str, owner: &str) -> ObjectDef {
        ObjectDef::MaterializedView(pgt_ddl::MatViewDef {
            schema: schema.into(),
            name: "mv".into(),
            owner: Some(owner.into()),
            query: query.into(),
            tablespace: None,
            fillfactor: None,
            with_data: true,
            comment: None,
            refresh: None,
            columns: vec![],
            unsupported: vec!["INDEX mv_i".into()],
        })
    }

    fn rt_mv_index(schema: &str) -> ObjectDef {
        ObjectDef::Index(pgt_ddl::IndexDef {
            schema: schema.into(),
            table: "mv".into(),
            name: "mv_i".into(),
            method: "btree".into(),
            unique: false,
            columns: vec![pgt_ddl::IndexColumn {
                key: pgt_ddl::IndexKey::Column { name: "a".into() },
                opclass: None,
                collation: None,
                descending: false,
                nulls_first: None,
            }],
            include: vec![],
            where_clause: None,
            tablespace: None,
            fillfactor: None,
            comment: None,
            constraint: None,
            valid: true,
        })
    }

    #[test]
    fn build_data_satir_tipi_rutin_sirasi_ve_mv_indeksleri() {
        use ObjectKind::*;
        use pgt_ddl::FunctionReturn;
        let setof_rt = FunctionReturn::Type { type_name: "rt".into(), setof: true };
        let text = FunctionReturn::Type { type_name: "text".into(), setof: false };
        let src = snap(vec![
            loaded(Table, 1, "rt", None, None, vec![], rt_table("a")),
            loaded(View, 2, "rv", None, None, vec![1], rt_view("a")),
            loaded(Function, 3, "list_rt", None, Some(""), vec![], rt_func("a", "list_rt", &[], setof_rt)),
            loaded(Function, 4, "v_label", None, Some("rv"), vec![], rt_func("a", "v_label", &["a.rv"], text.clone())),
            loaded(Function, 5, "plain", None, Some("integer"), vec![], rt_func("a", "plain", &["integer"], text)),
            loaded(MaterializedView, 6, "mv", None, None, vec![], rt_mv("a", " SELECT 2 AS a", "app")),
            loaded(Index, 7, "mv_i", Some("mv"), None, vec![], rt_mv_index("a")),
        ]);
        let tgt = snap(vec![
            loaded(MaterializedView, 16, "mv", None, None, vec![], rt_mv("b", " SELECT 1 AS a", "mv_owner")),
            loaded(Index, 17, "mv_i", Some("mv"), None, vec![], rt_mv_index("b")),
        ]);
        let mut sd = side("a");
        sd.database_oid = 1;
        let d = build_data(&sd, &side("b"), DiffOptions::default(), src, tgt, &CancelToken::new(), Instant::now())
            .expect("veri kurulmalı");
        let st = |n: &str| d.result().items.iter().find(|i| i.name == n).unwrap().clone();
        assert_eq!(st("mv_i").status, ItemStatus::Same);
        assert_eq!(st("mv").status, ItemStatus::Different);
        let keys: Vec<String> = d.result().items.iter().filter(|i| i.selectable).map(|i| i.key.clone()).collect();
        assert_eq!(keys.len(), 6, "{:?}", d.result().items);
        let sql = d.script(&keys, "t").unwrap().sql;
        let pos = |p: &str| sql.find(p).unwrap_or_else(|| panic!("yok: {p}\n{sql}"));
        assert!(pos("FUNCTION b.plain") < pos("CREATE TABLE b.rt"), "{sql}");
        assert!(pos("CREATE TABLE b.rt") < pos("FUNCTION b.list_rt"), "{sql}");
        assert!(pos("FUNCTION b.list_rt") < pos("CREATE VIEW b.rv"), "{sql}");
        assert!(pos("CREATE VIEW b.rv") < pos("FUNCTION b.v_label"), "{sql}");
        // MV yeniden kurulur; indeksi (Same) MV'den sonra yeniden oluşturulur, hedef sahibi korunur.
        assert!(pos("DROP MATERIALIZED VIEW b.mv") < pos("CREATE MATERIALIZED VIEW b.mv"), "{sql}");
        assert!(pos("CREATE MATERIALIZED VIEW b.mv") < pos("CREATE INDEX mv_i ON b.mv"), "{sql}");
        assert!(sql.contains("ALTER MATERIALIZED VIEW b.mv OWNER TO mv_owner"), "{sql}");
        assert_eq!(sql.matches("CREATE INDEX").count(), 1, "{sql}");
    }

    #[test]
    fn major_version() {
        assert_eq!(major(100000), 10);
        assert_eq!(major(160004), 16);
        assert_eq!(major(110022), 11);
    }
}
