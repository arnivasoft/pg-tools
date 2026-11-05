//! Betik birleştirme: seçili öğelerin aşamalı parçalarını spec §2.6 sırasıyla tek metne dizer
//! (başlık, oturum satırları, bölümler, transaction). Veritabanına dokunmaz; başlık üreticisi sunucu
//! adresi, kullanıcı ya da parola almaz.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::SystemTime;

use pgt_ddl::{FunctionReturn, ObjectDef, ObjectKind, Risk, TypeBody, quote_ident};

use crate::generate::{Generated, Phase};
use crate::model::DiffScript;

/// Tek satırlık SQL yorumuna güvenle girecek metin: denetim karakterleri (`\n`, `\r`, `\t` dahil) ve
/// U+2028/U+2029 `?` olur; böylece ileti yorumdan kaçıp ifade başlatamaz.
pub fn comment_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                '?'
            } else {
                c
            }
        })
        .collect()
}

/// Betiğe girecek tek öğe: kimlik bilgisi, üretilen parçalar ve seçili öğeler arası bağımlılıklar.
pub struct ScriptItem<'a> {
    pub key: &'a str,
    pub kind: ObjectKind,
    /// Tür değişiminde hedefteki nesnenin türü.
    pub target_kind: Option<ObjectKind>,
    pub name: &'a str,
    pub table: Option<&'a str>,
    pub generated: &'a Generated,
    /// Oluşturma için: kaynakta bağımlı olunan seçili öğe anahtarları (görünüm/MV, tip/domain).
    pub source_deps: Vec<&'a str>,
    /// Silme için: hedefte bağımlı olunan seçili öğe anahtarları (görünüm/MV, tablo FK).
    pub target_deps: Vec<&'a str>,
    /// Rutin için: imzası (argüman ya da dönüş tipi) satır tipine başvurulan seçili tablo/görünüm/MV anahtarları.
    pub rowtype_deps: Vec<&'a str>,
}

/// Başlık alanları; yalnız bunlar betik başlığına girer (sunucu kaydı, adres, kullanıcı yok).
pub struct ScriptHeader<'a> {
    pub source_db: &'a str,
    pub source_schema: &'a str,
    pub target_db: &'a str,
    pub target_schema: &'a str,
    pub target_version: &'a str,
    pub generated_at: &'a str,
}

const TX_WARNING: &str = "Betik transaction'sız; hata olursa kısmen uygulanmış kalır.";

/// Bağımlılık önce; eşitlikte giriş sırası; döngüde kalanlar giriş sırasıyla sona. Girişte olmayan ve
/// kendine olan bağımlılıklar yok sayılır.
pub fn topo_order<'a>(keys: &[&'a str], deps: &dyn Fn(&str) -> Vec<&'a str>) -> Vec<&'a str> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        index.entry(*k).or_insert(i);
    }
    let n = keys.len();
    let mut pending = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, k) in keys.iter().enumerate() {
        if index.get(*k) != Some(&i) {
            continue; // yinelenen anahtar: ilk görülen temsil eder
        }
        let mut seen = HashSet::new();
        for d in deps(k) {
            if let Some(&j) = index.get(d)
                && j != i
                && seen.insert(j)
            {
                pending[i] += 1;
                dependents[j].push(i);
            }
        }
    }
    let mut ready: BTreeSet<usize> = (0..n)
        .filter(|&i| index.get(keys[i]) == Some(&i) && pending[i] == 0)
        .collect();
    let mut done = vec![false; n];
    let mut out = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        done[i] = true;
        out.push(keys[i]);
        for &d in &dependents[i] {
            pending[d] -= 1;
            if pending[d] == 0 {
                ready.insert(d);
            }
        }
    }
    // Döngüdekiler (ve yinelenen anahtarlar) giriş sırasıyla sona.
    for i in 0..n {
        if !done[i] {
            out.push(keys[i]);
        }
    }
    out
}

/// `SystemTime` -> `"2026-10-31 00:00:00 UTC"` (epoch öncesi epoch sayılır).
pub fn utc_timestamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Hinnant'ın civil_from_days algoritması.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Tip ifadesinden başvurulan tipin adı: `(…)` parçaları ve `[…]` sonrası atılır, tırnaklar çözülür;
/// nitelikli ad noktayla birleşik kalır.
fn base_type_name(t: &str) -> String {
    let mut parts: Vec<String> = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut depth = 0usize;
    let mut chars = t.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '"' if depth == 0 => quoted = true,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            '[' => break,
            '.' => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur.trim().to_string());
    parts.join(".")
}

/// Tanımın başvurduğu tipler: composite öznitelik tipleri, domain temel tipi, range alt tipi.
pub fn type_refs(def: &ObjectDef) -> Vec<String> {
    match def {
        ObjectDef::Type(t) => match &t.body {
            TypeBody::Composite { attributes } => attributes
                .iter()
                .map(|a| base_type_name(&a.type_name))
                .collect(),
            TypeBody::Range { subtype, .. } => vec![base_type_name(subtype)],
            TypeBody::Enum { .. } => vec![],
        },
        ObjectDef::Domain(d) => vec![base_type_name(&d.base_type)],
        _ => vec![],
    }
}

/// Rutin imzasının başvurduğu tipler: argüman tipleri (tüm kipler) ve dönüş tipi (`SETOF` dahil, `TABLE` kolonları).
pub fn routine_type_refs(def: &ObjectDef) -> Vec<String> {
    let (ObjectDef::Function(f) | ObjectDef::Procedure(f)) = def else {
        return vec![];
    };
    let mut v: Vec<String> = f.args.iter().map(|a| base_type_name(&a.type_name)).collect();
    match &f.returns {
        Some(FunctionReturn::Type { type_name, .. }) => v.push(base_type_name(type_name)),
        Some(FunctionReturn::Table { columns }) => {
            v.extend(columns.iter().map(|c| base_type_name(&c.type_name)));
        }
        _ => {}
    }
    v
}

fn kind_label(k: ObjectKind) -> &'static str {
    match k {
        ObjectKind::Table => "Tablo",
        ObjectKind::View => "Görünüm",
        ObjectKind::MaterializedView => "Materialized view",
        ObjectKind::Sequence => "Sequence",
        ObjectKind::Index => "İndeks",
        ObjectKind::Trigger => "Trigger",
        ObjectKind::Function => "Fonksiyon",
        ObjectKind::Procedure => "Prosedür",
        ObjectKind::Type => "Tip",
        ObjectKind::Domain => "Domain",
        ObjectKind::Schema => "Şema",
        ObjectKind::Extension => "Eklenti",
        ObjectKind::Database => "Veritabanı",
        ObjectKind::Role => "Rol",
    }
}

/// Aşama içi varsayılan sıra için tür sırası (`ObjectKind` sıralı değildir).
fn kind_rank(k: ObjectKind) -> u8 {
    match k {
        ObjectKind::Trigger => 0,
        ObjectKind::Index => 1,
        ObjectKind::View => 2,
        ObjectKind::MaterializedView => 3,
        ObjectKind::Function => 4,
        ObjectKind::Procedure => 5,
        ObjectKind::Table => 6,
        ObjectKind::Sequence => 7,
        ObjectKind::Domain => 8,
        ObjectKind::Type => 9,
        ObjectKind::Schema => 10,
        ObjectKind::Extension => 11,
        ObjectKind::Database => 12,
        ObjectKind::Role => 13,
    }
}

fn risk_label(r: Risk) -> &'static str {
    match r {
        Risk::Safe => "Güvenli",
        Risk::Destructive => "Yıkıcı",
        Risk::DataLoss => "Veri kaybı olabilir",
    }
}

const ALL_PHASES: [Phase; 20] = [
    Phase::DropTrigger,
    Phase::DropIndex,
    Phase::DropView,
    Phase::DropRoutine,
    Phase::DropTable,
    Phase::DropSequence,
    Phase::DropDomain,
    Phase::DropType,
    Phase::Types,
    Phase::Sequences,
    Phase::Routines,
    Phase::Tables,
    Phase::RoutinesAfterTables,
    Phase::OwnedBy,
    Phase::ForeignKeys,
    Phase::Views,
    Phase::RoutinesAfterViews,
    Phase::Indexes,
    Phase::Triggers,
    Phase::Privileges,
];

#[derive(PartialEq, Eq, Clone, Copy, Hash)]
enum Section {
    Drops,
    Creates,
    Grants,
}

fn section_of(p: Phase) -> Section {
    match p {
        Phase::DropTrigger
        | Phase::DropIndex
        | Phase::DropView
        | Phase::DropRoutine
        | Phase::DropTable
        | Phase::DropSequence
        | Phase::DropDomain
        | Phase::DropType => Section::Drops,
        Phase::Privileges => Section::Grants,
        _ => Section::Creates,
    }
}

/// Yalnız-hedef dizi, sahibi olan tabloyla ya da kolonla birlikte betikte zaten düşüyor mu?
fn auto_dropped(it: &ScriptItem, items: &[ScriptItem]) -> bool {
    let Some((t, c)) = &it.generated.sequence_owner else {
        return false;
    };
    items.iter().any(|o| {
        o.table.is_none()
            && o.target_kind.unwrap_or(o.kind) == ObjectKind::Table
            && o.name == t
            && (o.generated.drops_table || o.generated.dropped_columns.iter().any(|d| d == c))
    })
}

/// Rutin öğesinin `Routines` parçalarının yazılacağı aşama: imzasındaki satır tiplerinden biri bu betikte
/// oluşturulan (ya da yeniden kurulan) bir tabloya aitse tablolardan, görünüme/MV'ye aitse görünümlerden sonra;
/// aksi halde `Routines`.
fn routine_phase(it: &ScriptItem, items: &[ScriptItem], by_key: &HashMap<&str, usize>) -> Phase {
    it.rowtype_deps
        .iter()
        .filter_map(|k| by_key.get(k).map(|&j| &items[j]))
        .filter(|d| d.generated.rebuilt)
        .map(|d| match d.kind {
            ObjectKind::Table => Phase::RoutinesAfterTables,
            ObjectKind::View | ObjectKind::MaterializedView => Phase::RoutinesAfterViews,
            _ => Phase::Routines,
        })
        .max()
        .unwrap_or(Phase::Routines)
}

/// Aşamada yer alan öğelerin yazım sırası (öğe indisleri).
fn phase_order(phase: Phase, idx: &[usize], items: &[ScriptItem]) -> Vec<usize> {
    debug_assert!(
        idx.iter().map(|&i| items[i].key).collect::<HashSet<_>>().len() == idx.len(),
        "aşamada yinelenen öğe anahtarı"
    );
    let mut base = idx.to_vec();
    base.sort_by(|&a, &b| {
        let (x, y) = (&items[a], &items[b]);
        (kind_rank(x.kind), x.name, x.table, x.key).cmp(&(kind_rank(y.kind), y.name, y.table, y.key))
    });
    let by_key: HashMap<&str, usize> = base.iter().map(|&i| (items[i].key, i)).collect();
    let keys: Vec<&str> = base.iter().map(|&i| items[i].key).collect();
    let ordered = |source: bool| -> Vec<usize> {
        let deps = |k: &str| {
            let it = &items[by_key[k]];
            if source {
                it.source_deps.clone()
            } else {
                it.target_deps.clone()
            }
        };
        topo_order(&keys, &deps)
            .into_iter()
            .map(|k| by_key[k])
            .collect()
    };
    match phase {
        Phase::Views | Phase::Types => ordered(true),
        Phase::DropView | Phase::DropTable => {
            let mut v = ordered(false);
            v.reverse();
            v
        }
        _ => base,
    }
}

/// Parçaları birleştirip betiği kurar.
pub fn assemble(h: &ScriptHeader, items: &[ScriptItem]) -> DiffScript {
    // Otomatik düşen diziler DROP SEQUENCE yerine açıklama satırı alır.
    let skipped: Vec<bool> = items.iter().map(|it| auto_dropped(it, items)).collect();
    let is_skipped = |i: usize, p: Phase| p == Phase::DropSequence && skipped[i];
    // Satır tipine bağlı rutinler kendi aşamasından sonraya taşınır.
    let by_key: HashMap<&str, usize> = items.iter().enumerate().map(|(i, it)| (it.key, i)).collect();
    let moved: Vec<Phase> = items.iter().map(|it| routine_phase(it, items, &by_key)).collect();
    let phase_of = |i: usize, p: Phase| if p == Phase::Routines { moved[i] } else { p };

    let mut by_phase: HashMap<Phase, Vec<usize>> = HashMap::new();
    let mut statements = 0u32;
    let mut transactional = true;
    for (i, it) in items.iter().enumerate() {
        for (p, stmts) in &it.generated.parts {
            if stmts.is_empty() {
                continue;
            }
            let v = by_phase.entry(phase_of(i, *p)).or_default();
            if !v.contains(&i) {
                v.push(i);
            }
            if is_skipped(i, *p) {
                continue;
            }
            for s in stmts {
                statements += 1;
                transactional &= s.transactional;
            }
        }
    }
    let risk = items
        .iter()
        .map(|it| it.generated.risk)
        .max()
        .unwrap_or(Risk::Safe);

    let mut warnings: Vec<String> = vec![];
    for w in items.iter().flat_map(|it| &it.generated.warnings) {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    if !transactional {
        warnings.push(TX_WARNING.to_string());
    }

    let mut out: Vec<String> = vec![
        "-- pg-tools şema karşılaştırma betiği".into(),
        format!(
            "-- Kaynak: {}.{}",
            comment_safe(h.source_db),
            comment_safe(h.source_schema)
        ),
        format!(
            "-- Hedef: {}.{} (PostgreSQL {})",
            comment_safe(h.target_db),
            comment_safe(h.target_schema),
            comment_safe(h.target_version)
        ),
        format!("-- Oluşturuldu: {}", comment_safe(h.generated_at)),
        format!(
            "-- Öğe: {}, ifade: {}, en yüksek risk: {}",
            items.len(),
            statements,
            risk_label(risk)
        ),
        "-- Bağımlılıklar tamamlanmaz: seçilmeyen nesnelere bağlı öğeler hedefte hata verebilir."
            .into(),
    ];
    if !transactional {
        out.push(format!("-- UYARI: {TX_WARNING}"));
    }
    out.push(String::new());
    if transactional {
        out.push("BEGIN;".into());
    }
    let local = if transactional { "LOCAL " } else { "" };
    out.push(format!("SET {local}check_function_bodies = off;"));
    out.push(format!(
        "SET {local}search_path TO pg_catalog, {};",
        quote_ident(h.target_schema)
    ));

    let mut section: Option<Section> = None;
    // Öğe etiketi, ardışık ifadeler arasında öğe değiştiğinde (ve her bölüm başında) yeniden yazılır.
    let mut last_labelled: Option<usize> = None;
    for phase in ALL_PHASES {
        let Some(idx) = by_phase.get(&phase) else {
            continue;
        };
        let sec = section_of(phase);
        for i in phase_order(phase, idx, items) {
            let it = &items[i];
            if section != Some(sec) {
                section = Some(sec);
                last_labelled = None;
                out.push(String::new());
                out.push(
                    match sec {
                        Section::Drops => "-- ==== Silmeler ====",
                        Section::Creates => "-- ==== Oluşturma ve değişiklikler ====",
                        Section::Grants => "-- ==== Yetkiler ====",
                    }
                    .into(),
                );
            }
            if last_labelled != Some(i) {
                last_labelled = Some(i);
                let kind = if sec == Section::Drops {
                    it.target_kind.unwrap_or(it.kind)
                } else {
                    it.kind
                };
                let qual = it.table.map(|t| format!("{t}.")).unwrap_or_default();
                out.push(format!(
                    "-- {} {}",
                    kind_label(kind),
                    comment_safe(&format!("{qual}{}", it.name))
                ));
            }
            for (_, stmts) in it.generated.parts.iter().filter(|(p, _)| phase_of(i, *p) == phase) {
                if is_skipped(i, phase) {
                    if !stmts.is_empty() {
                        out.push(format!(
                            "-- {}: bağlı olduğu kolonla birlikte silinir",
                            comment_safe(it.name)
                        ));
                    }
                    continue;
                }
                for s in stmts {
                    if let Some(n) = &s.note {
                        out.push(format!("-- not: {}", comment_safe(n)));
                    }
                    out.push(format!("{};", s.sql));
                }
            }
        }
    }
    if transactional {
        out.push(String::new());
        out.push("COMMIT;".into());
    }
    let mut sql = out.join("\n");
    sql.push('\n');
    DiffScript {
        sql,
        transactional,
        warnings,
        risk,
        statements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgt_ddl::{CompositeAttr, DomainDef, Statement, TypeDef};
    use std::time::Duration;

    fn st(sql: &str) -> Statement {
        Statement {
            sql: sql.into(),
            risk: Risk::Safe,
            transactional: true,
            note: None,
            redacted: false,
        }
    }

    fn gen_parts(parts: Vec<(Phase, Vec<Statement>)>) -> Generated {
        Generated {
            parts,
            warnings: vec![],
            risk: Risk::Safe,
            notes: vec![],
            drops_table: false,
            dropped_columns: vec![],
            sequence_owner: None,
            rebuilt: false,
        }
    }

    fn item<'a>(key: &'a str, kind: ObjectKind, name: &'a str, g: &'a Generated) -> ScriptItem<'a> {
        ScriptItem {
            key,
            kind,
            target_kind: None,
            name,
            table: None,
            generated: g,
            source_deps: vec![],
            target_deps: vec![],
            rowtype_deps: vec![],
        }
    }

    const H: ScriptHeader<'static> = ScriptHeader {
        source_db: "kaynak",
        source_schema: "public",
        target_db: "hedef",
        target_schema: "Hedef Şema",
        target_version: "16.4",
        generated_at: "2026-10-31 00:00:00 UTC",
    };

    #[test]
    fn comment_safe_denetim_karakterlerini_soru_isaretine_cevirir() {
        assert_eq!(comment_safe("a\nb\r\u{2028}c\td"), "a?b??c?d");
    }

    #[test]
    fn tam_senaryo_asama_ve_bolum_sirasi() {
        let g_trg = gen_parts(vec![(Phase::DropTrigger, vec![st("DROP TRIGGER t1 ON a")])]);
        let g_idx = gen_parts(vec![(Phase::DropIndex, vec![st("DROP INDEX i1")])]);
        let g_view = gen_parts(vec![(Phase::DropView, vec![st("DROP VIEW v0")])]);
        let g_fn = gen_parts(vec![(Phase::DropRoutine, vec![st("DROP FUNCTION f0()")])]);
        let g_tab = gen_parts(vec![(Phase::DropTable, vec![st("DROP TABLE t0")])]);
        let g_seq = gen_parts(vec![(Phase::DropSequence, vec![st("DROP SEQUENCE s0")])]);
        let g_dom = gen_parts(vec![(Phase::DropDomain, vec![st("DROP DOMAIN d0")])]);
        let g_typ = gen_parts(vec![(Phase::DropType, vec![st("DROP TYPE y0")])]);
        let g_new_t = gen_parts(vec![(Phase::Types, vec![st("CREATE TYPE y1 AS ENUM ()")])]);
        let g_new_s = gen_parts(vec![
            (Phase::Sequences, vec![st("CREATE SEQUENCE s1")]),
            (Phase::OwnedBy, vec![st("ALTER SEQUENCE s1 OWNED BY t1.id")]),
        ]);
        let g_new_f = gen_parts(vec![(Phase::Routines, vec![st("CREATE FUNCTION f1() ...")])]);
        let g_new_tab = gen_parts(vec![
            (Phase::Tables, vec![st("CREATE TABLE t1 (id int)")]),
            (Phase::ForeignKeys, vec![st("ALTER TABLE t1 ADD CONSTRAINT fk ...")]),
            (Phase::Privileges, vec![st("GRANT SELECT ON t1 TO r")]),
        ]);
        let g_new_v = gen_parts(vec![(Phase::Views, vec![st("CREATE VIEW v1 AS SELECT 1")])]);
        let g_new_i = gen_parts(vec![(Phase::Indexes, vec![st("CREATE INDEX i2 ON t1 (id)")])]);
        let g_new_g = gen_parts(vec![(Phase::Triggers, vec![st("CREATE TRIGGER tg ...")])]);
        let mut items = vec![
            item("k_priv_tab", ObjectKind::Table, "t1", &g_new_tab),
            item("k_trg", ObjectKind::Trigger, "tg", &g_new_g),
            item("k_idx", ObjectKind::Index, "i2", &g_new_i),
            item("k_view", ObjectKind::View, "v1", &g_new_v),
            item("k_fn", ObjectKind::Function, "f1", &g_new_f),
            item("k_seq", ObjectKind::Sequence, "s1", &g_new_s),
            item("k_typ", ObjectKind::Type, "y1", &g_new_t),
            item("d_typ", ObjectKind::Type, "y0", &g_typ),
            item("d_dom", ObjectKind::Domain, "d0", &g_dom),
            item("d_seq", ObjectKind::Sequence, "s0", &g_seq),
            item("d_tab", ObjectKind::Table, "t0", &g_tab),
            item("d_fn", ObjectKind::Function, "f0", &g_fn),
            item("d_view", ObjectKind::View, "v0", &g_view),
            item("d_idx", ObjectKind::Index, "i1", &g_idx),
            item("d_trg", ObjectKind::Trigger, "t1", &g_trg),
        ];
        items[1].table = Some("t1");
        items[2].table = Some("t1");
        items[14].table = Some("a");
        insta::assert_snapshot!("tam_senaryo", assemble(&H, &items).sql);
    }

    #[test]
    fn fkli_iki_yeni_tablo_create_sonra_add_constraint() {
        let ga = gen_parts(vec![
            (Phase::Tables, vec![st("CREATE TABLE a (id int)")]),
            (Phase::ForeignKeys, vec![st("ALTER TABLE a ADD CONSTRAINT a_fk FOREIGN KEY (id) REFERENCES b (id)")]),
        ]);
        let gb = gen_parts(vec![
            (Phase::Tables, vec![st("CREATE TABLE b (id int)")]),
            (Phase::ForeignKeys, vec![st("ALTER TABLE b ADD CONSTRAINT b_fk FOREIGN KEY (id) REFERENCES a (id)")]),
        ]);
        let items = [item("a", ObjectKind::Table, "a", &ga), item("b", ObjectKind::Table, "b", &gb)];
        let s = assemble(&H, &items).sql;
        let pos = |p: &str| s.find(p).unwrap_or_else(|| panic!("yok: {p}"));
        assert!(pos("CREATE TABLE a") < pos("CREATE TABLE b"));
        assert!(pos("CREATE TABLE b") < pos("ADD CONSTRAINT a_fk"));
        assert!(pos("ADD CONSTRAINT a_fk") < pos("ADD CONSTRAINT b_fk"));
        insta::assert_snapshot!("fkli_iki_tablo", s);
    }

    #[test]
    fn gorunum_zinciri_olusturmada_v1_v2_silmede_v2_v1() {
        let c1 = gen_parts(vec![(Phase::Views, vec![st("CREATE VIEW v1")])]);
        let c2 = gen_parts(vec![(Phase::Views, vec![st("CREATE VIEW v2")])]);
        let d1 = gen_parts(vec![(Phase::DropView, vec![st("DROP VIEW v1")])]);
        let d2 = gen_parts(vec![(Phase::DropView, vec![st("DROP VIEW v2")])]);
        // Ad sırası (v0 < v1) topolojiyi bozmasın diye bağımlı olan "a_v2" adlı.
        let mut v2c = item("v2", ObjectKind::View, "a_v2", &c2);
        v2c.source_deps = vec!["v1"];
        let create = [v2c, item("v1", ObjectKind::View, "z_v1", &c1)];
        let s = assemble(&H, &create).sql;
        assert!(s.find("CREATE VIEW v1").unwrap() < s.find("CREATE VIEW v2").unwrap());

        let mut v2d = item("v2", ObjectKind::View, "a_v2", &d2);
        v2d.target_deps = vec!["v1"];
        let drop = [item("v1", ObjectKind::View, "z_v1", &d1), v2d];
        let s = assemble(&H, &drop).sql;
        assert!(s.find("DROP VIEW v2").unwrap() < s.find("DROP VIEW v1").unwrap());
    }

    fn type_item(name: &str, body: pgt_ddl::TypeBody) -> ObjectDef {
        ObjectDef::Type(TypeDef {
            schema: "public".into(),
            name: name.into(),
            owner: None,
            comment: None,
            body,
            editable: true,
            unsupported: vec![],
        })
    }

    #[test]
    fn composite_ve_domain_tip_bagimliligi_tip_once() {
        let comp = type_item(
            "a",
            TypeBody::Composite {
                attributes: vec![CompositeAttr {
                    name: "x".into(),
                    type_name: "b".into(),
                    collation: None,
                    original: None,
                }],
            },
        );
        let dom = ObjectDef::Domain(DomainDef {
            schema: "public".into(),
            name: "d".into(),
            owner: None,
            comment: None,
            base_type: "b".into(),
            collation: None,
            default: None,
            not_null: false,
            constraints: vec![],
            editable: true,
            unsupported: vec![],
        });
        assert_eq!(type_refs(&comp), ["b"]);
        assert_eq!(type_refs(&dom), ["b"]);

        let ga = gen_parts(vec![(Phase::Types, vec![st("CREATE TYPE a AS (x b)")])]);
        let gb = gen_parts(vec![(Phase::Types, vec![st("CREATE TYPE b AS ENUM ('1')")])]);
        let gd = gen_parts(vec![(Phase::Types, vec![st("CREATE DOMAIN d AS b")])]);
        let mut a = item("ka", ObjectKind::Type, "a", &ga);
        a.source_deps = vec!["kb"];
        let mut d = item("kd", ObjectKind::Domain, "d", &gd);
        d.source_deps = vec!["kb"];
        let items = [a, d, item("kb", ObjectKind::Type, "b", &gb)];
        let s = assemble(&H, &items).sql;
        let pos = |p: &str| s.find(p).unwrap();
        assert!(pos("CREATE TYPE b") < pos("CREATE TYPE a"));
        assert!(pos("CREATE TYPE b") < pos("CREATE DOMAIN d"));
    }

    #[test]
    fn silme_sirasi_trigger_indeks_gorunum_rutin_tablo_sequence_domain_type() {
        let mk = |p: Phase, sql: &str| gen_parts(vec![(p, vec![st(sql)])]);
        let gs = [
            mk(Phase::DropType, "DROP TYPE"),
            mk(Phase::DropDomain, "DROP DOMAIN"),
            mk(Phase::DropSequence, "DROP SEQUENCE"),
            mk(Phase::DropTable, "DROP TABLE"),
            mk(Phase::DropRoutine, "DROP FUNCTION"),
            mk(Phase::DropView, "DROP VIEW"),
            mk(Phase::DropIndex, "DROP INDEX"),
            mk(Phase::DropTrigger, "DROP TRIGGER"),
        ];
        let kinds = [
            ObjectKind::Type,
            ObjectKind::Domain,
            ObjectKind::Sequence,
            ObjectKind::Table,
            ObjectKind::Function,
            ObjectKind::View,
            ObjectKind::Index,
            ObjectKind::Trigger,
        ];
        let keys = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let items: Vec<_> = (0..8).map(|i| item(keys[i], kinds[i], "n", &gs[i])).collect();
        let s = assemble(&H, &items).sql;
        let order: Vec<_> = s
            .lines()
            .filter(|l| l.starts_with("DROP "))
            .map(|l| l.trim_end_matches(';'))
            .collect();
        assert_eq!(
            order,
            ["DROP TRIGGER", "DROP INDEX", "DROP VIEW", "DROP FUNCTION", "DROP TABLE", "DROP SEQUENCE", "DROP DOMAIN", "DROP TYPE"]
        );
    }

    #[test]
    fn birbirine_fkli_tablo_silmede_basvuran_once() {
        let ga = gen_parts(vec![(Phase::DropTable, vec![st("DROP TABLE a")])]);
        let gb = gen_parts(vec![(Phase::DropTable, vec![st("DROP TABLE b")])]);
        // a, b'ye başvurur (a'nın hedef bağımlılığı b).
        let mut a = item("ka", ObjectKind::Table, "a", &ga);
        a.target_deps = vec!["kb"];
        let items = [item("kb", ObjectKind::Table, "b", &gb), a];
        let s = assemble(&H, &items).sql;
        assert!(s.find("DROP TABLE a").unwrap() < s.find("DROP TABLE b").unwrap());
    }

    #[test]
    fn topo_order_dongu_giris_sirasiyla_kayipsiz() {
        let deps = |k: &str| -> Vec<&'static str> {
            match k {
                "a" => vec!["b"],
                "b" => vec!["a"],
                "c" => vec!["a"],
                _ => vec![],
            }
        };
        assert_eq!(topo_order(&["c", "a", "b", "d"], &deps), ["d", "c", "a", "b"]);
        let line = |k: &str| -> Vec<&'static str> {
            match k {
                "x" => vec!["y", "yok", "x"],
                _ => vec![],
            }
        };
        assert_eq!(topo_order(&["x", "y", "z"], &line), ["y", "x", "z"]);
        assert!(topo_order(&[], &line).is_empty());
    }

    #[test]
    fn otomatik_dusen_dizi_drop_sequence_yazilmaz() {
        let mut gt = gen_parts(vec![(Phase::DropTable, vec![st("DROP TABLE t")])]);
        gt.drops_table = true;
        let mut gc = gen_parts(vec![(Phase::Tables, vec![st("ALTER TABLE u DROP COLUMN c")])]);
        gc.dropped_columns = vec!["c".into()];
        let mut other = gen_parts(vec![(Phase::Tables, vec![st("ALTER TABLE w ADD COLUMN z int")])]);
        other.dropped_columns = vec![];
        let seq = |owner: (&str, &str)| {
            let mut g = gen_parts(vec![(Phase::DropSequence, vec![st("DROP SEQUENCE s")])]);
            g.sequence_owner = Some((owner.0.into(), owner.1.into()));
            g
        };
        let (s1, s2, s3) = (seq(("t", "id")), seq(("u", "c")), seq(("w", "id")));
        let items = [
            item("t", ObjectKind::Table, "t", &gt),
            item("s1", ObjectKind::Sequence, "t_id_seq", &s1),
            item("u", ObjectKind::Table, "u", &gc),
            item("s2", ObjectKind::Sequence, "u_c_seq", &s2),
            item("w", ObjectKind::Table, "w", &other),
            item("s3", ObjectKind::Sequence, "w_id_seq", &s3),
        ];
        let r = assemble(&H, &items);
        assert!(r.sql.contains("-- t_id_seq: bağlı olduğu kolonla birlikte silinir"));
        assert!(r.sql.contains("-- u_c_seq: bağlı olduğu kolonla birlikte silinir"));
        assert!(!r.sql.contains("-- w_id_seq: bağlı"));
        assert_eq!(r.sql.matches("DROP SEQUENCE s;").count(), 1);
        // 6 öğeden 2'si atlandı: ifade sayısı 4.
        assert_eq!(r.statements, 4);
        insta::assert_snapshot!("otomatik_dusen_dizi", r.sql);
    }

    #[test]
    fn transaction_siz_ifade_begin_yok_set_local_yok_uyari_var() {
        let mut s = st("ALTER TYPE e ADD VALUE 'x'");
        s.transactional = false;
        let mut g = gen_parts(vec![(Phase::Types, vec![s])]);
        g.warnings = vec!["öğe uyarısı".into()];
        let r = assemble(&H, &[item("e", ObjectKind::Type, "e", &g)]);
        assert!(!r.transactional);
        assert!(!r.sql.contains("BEGIN;") && !r.sql.contains("COMMIT;") && !r.sql.contains("LOCAL"));
        assert!(r.sql.contains("-- UYARI: Betik transaction'sız; hata olursa kısmen uygulanmış kalır."));
        assert_eq!(
            r.warnings,
            ["öğe uyarısı", "Betik transaction'sız; hata olursa kısmen uygulanmış kalır."]
        );
        insta::assert_snapshot!("transaction_siz", r.sql);
    }

    #[test]
    fn transaction_li_begin_set_local_commit_ve_search_path() {
        let mut s = st("DROP TABLE t");
        s.risk = Risk::DataLoss;
        s.note = Some("not\nmetni".into());
        let mut g = gen_parts(vec![(Phase::DropTable, vec![s])]);
        g.risk = Risk::DataLoss;
        g.warnings = vec!["w".into(), "w".into()];
        let g2 = Generated { warnings: vec!["w".into(), "v".into()], ..gen_parts(vec![]) };
        let r = assemble(&H, &[item("t", ObjectKind::Table, "t", &g), item("u", ObjectKind::Table, "u", &g2)]);
        assert!(r.transactional);
        assert_eq!(r.risk, Risk::DataLoss);
        assert_eq!(r.warnings, ["w", "v"]);
        assert_eq!(r.statements, 1);
        assert!(r.sql.contains("BEGIN;\nSET LOCAL check_function_bodies = off;\nSET LOCAL search_path TO pg_catalog, \"Hedef Şema\";"));
        assert!(r.sql.contains("-- not: not?metni\nDROP TABLE t;"));
        assert!(r.sql.trim_end().ends_with("COMMIT;"));
        assert!(r.sql.contains("en yüksek risk: Veri kaybı olabilir"));
        insta::assert_snapshot!("transaction_li", r.sql);
    }

    #[test]
    fn baslik_guvenligi_satir_sayisi_sabit() {
        let normal = assemble(&H, &[]).sql;
        let h = ScriptHeader {
            source_db: "a\nDROP TABLE x; --",
            source_schema: "s\u{2028}x",
            target_db: "t\r\nSELECT 1",
            target_schema: "p",
            target_version: "16\n",
            generated_at: "z\u{2029}",
        };
        let s = assemble(&h, &[]).sql;
        let head = |t: &str| t.lines().take_while(|l| !l.starts_with("BEGIN")).count();
        assert_eq!(head(&s), head(&normal));
        for l in s.lines().take_while(|l| !l.starts_with("BEGIN")) {
            assert!(l.is_empty() || l.starts_with("--"), "satır: {l}");
        }
    }

    #[test]
    fn baslik_yalniz_verilen_alanlari_icerir() {
        // Alan listesi imzada sabit: eklenen/çıkarılan alan bu testi derlemez.
        let ScriptHeader {
            source_db: _,
            source_schema: _,
            target_db: _,
            target_schema: _,
            target_version: _,
            generated_at: _,
        } = H;
        let s = assemble(&H, &[]).sql;
        let head: Vec<_> = s.lines().take_while(|l| !l.is_empty()).collect();
        assert_eq!(
            head,
            [
                "-- pg-tools şema karşılaştırma betiği",
                "-- Kaynak: kaynak.public",
                "-- Hedef: hedef.Hedef Şema (PostgreSQL 16.4)",
                "-- Oluşturuldu: 2026-10-31 00:00:00 UTC",
                "-- Öğe: 0, ifade: 0, en yüksek risk: Güvenli",
                "-- Bağımlılıklar tamamlanmaz: seçilmeyen nesnelere bağlı öğeler hedefte hata verebilir.",
            ]
        );
    }

    #[test]
    fn utc_timestamp_bilinen_degerler() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_404_800);
        assert_eq!(utc_timestamp(t), "2026-10-31 00:00:00 UTC");
        assert_eq!(utc_timestamp(SystemTime::UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_251_199 + 1 + 3661);
        assert_eq!(utc_timestamp(leap), "2024-03-01 01:01:01 UTC");
    }

    #[test]
    fn her_ifade_kendi_ogesinin_etiketi_altinda() {
        let ga = gen_parts(vec![
            (Phase::Tables, vec![st("CREATE TABLE a (id int)")]),
            (Phase::ForeignKeys, vec![st("ALTER TABLE a ADD CONSTRAINT a_fk")]),
        ]);
        let gb = gen_parts(vec![(Phase::Tables, vec![st("CREATE TABLE b (id int)")])]);
        let items = [item("a", ObjectKind::Table, "a", &ga), item("b", ObjectKind::Table, "b", &gb)];
        let s = assemble(&H, &items).sql;
        let lines: Vec<_> = s.lines().collect();
        let at = |p: &str| lines.iter().position(|l| l.starts_with(p)).unwrap();
        // Hem CREATE'ler hem FK, kendi etiketinin hemen altında.
        assert_eq!(lines[at("ALTER TABLE a ADD CONSTRAINT") - 1], "-- Tablo a");
        assert_eq!(lines[at("CREATE TABLE b") - 1], "-- Tablo b");
        assert_eq!(lines[at("CREATE TABLE a") - 1], "-- Tablo a");
    }

    #[test]
    fn satir_sonlu_ad_etiket_ve_otomatik_dusen_satirinda_yorumda_kalir() {
        let mut gt = gen_parts(vec![(Phase::DropTable, vec![st("DROP TABLE \"t\"")])]);
        gt.drops_table = true;
        let mut gs = gen_parts(vec![(Phase::DropSequence, vec![st("DROP SEQUENCE s")])]);
        gs.sequence_owner = Some(("t\nDROP TABLE x; --".into(), "id".into()));
        let mut t = item("t", ObjectKind::Table, "t\nDROP TABLE x; --", &gt);
        t.table = None;
        let s_item = item("s", ObjectKind::Sequence, "s\nDROP SCHEMA y; --", &gs);
        let sql = assemble(&H, &[t, s_item]).sql;
        for l in sql.lines() {
            assert!(!l.starts_with("DROP SCHEMA") && !l.starts_with("x;"), "satır: {l}");
        }
        assert!(sql.contains("-- Sequence s?DROP SCHEMA y; --"));
        assert!(sql.contains("-- s?DROP SCHEMA y; --: bağlı olduğu kolonla birlikte silinir"));
    }

    #[test]
    fn satir_tipine_bagli_rutin_iliskisinden_sonra() {
        let rebuilt = |p: Phase, sql: &str| Generated { rebuilt: true, ..gen_parts(vec![(p, vec![st(sql)])]) };
        let g_tab = rebuilt(Phase::Tables, "CREATE TABLE t (id int)");
        let g_view = rebuilt(Phase::Views, "CREATE VIEW v AS SELECT 1");
        let g_idx = gen_parts(vec![(Phase::Indexes, vec![st("CREATE INDEX i ON t (id)")])]);
        let g_f = gen_parts(vec![
            (Phase::Routines, vec![st("CREATE FUNCTION f() RETURNS SETOF t")]),
            (Phase::Privileges, vec![st("GRANT EXECUTE ON FUNCTION f() TO r")]),
        ]);
        let g_g = gen_parts(vec![(Phase::Routines, vec![st("CREATE FUNCTION g(r v)")])]);
        let g_h = gen_parts(vec![(Phase::Routines, vec![st("CREATE FUNCTION h(a t, b v)")])]);
        let g_plain = gen_parts(vec![(Phase::Routines, vec![st("CREATE FUNCTION plain()")])]);
        let mut f = item("kf", ObjectKind::Function, "a_f", &g_f);
        f.rowtype_deps = vec!["kt"];
        let mut g = item("kg", ObjectKind::Function, "a_g", &g_g);
        g.rowtype_deps = vec!["kv"];
        let mut h = item("kh", ObjectKind::Function, "a_h", &g_h);
        h.rowtype_deps = vec!["kt", "kv"];
        let items = [
            h,
            g,
            f,
            item("kp", ObjectKind::Function, "z_plain", &g_plain),
            item("ki", ObjectKind::Index, "i", &g_idx),
            item("kv", ObjectKind::View, "v", &g_view),
            item("kt", ObjectKind::Table, "t", &g_tab),
        ];
        let s = assemble(&H, &items).sql;
        let pos = |p: &str| s.find(p).unwrap_or_else(|| panic!("yok: {p}"));
        assert!(pos("CREATE FUNCTION plain") < pos("CREATE TABLE t"), "bağımsız rutin yerinde");
        assert!(pos("CREATE TABLE t") < pos("CREATE FUNCTION f()"));
        assert!(pos("CREATE FUNCTION f()") < pos("CREATE VIEW v"), "tablo satır tipi: görünümlerden önce");
        assert!(pos("CREATE VIEW v") < pos("CREATE FUNCTION g("));
        assert!(pos("CREATE VIEW v") < pos("CREATE FUNCTION h("), "en geç ilişkiden sonra");
        assert!(pos("CREATE FUNCTION g(") < pos("CREATE INDEX i"), "indekslerden önce");
        assert!(pos("CREATE INDEX i") < pos("GRANT EXECUTE"), "yetkiler yerinde");
        assert_eq!(lines_before(&s, "CREATE FUNCTION f()"), "-- Fonksiyon a_f");
        insta::assert_snapshot!("satir_tipi_rutin", s);
    }

    fn lines_before<'a>(s: &'a str, needle: &str) -> &'a str {
        let lines: Vec<&str> = s.lines().collect();
        let at = lines.iter().position(|l| l.starts_with(needle)).unwrap();
        lines[at - 1]
    }

    #[test]
    fn satir_tipi_iliskisi_yeniden_kurulmuyorsa_rutin_yerinde() {
        // Tablo yalnız değişiyor (ALTER): rutin kendi aşamasında, tablolardan önce kalır.
        let g_tab = gen_parts(vec![(Phase::Tables, vec![st("ALTER TABLE t ADD COLUMN x int")])]);
        let g_f = gen_parts(vec![(Phase::Routines, vec![st("CREATE FUNCTION f() RETURNS SETOF t")])]);
        let mut f = item("kf", ObjectKind::Function, "f", &g_f);
        f.rowtype_deps = vec!["kt", "secilmemis"];
        let s = assemble(&H, &[item("kt", ObjectKind::Table, "t", &g_tab), f]).sql;
        assert!(s.find("CREATE FUNCTION f()").unwrap() < s.find("ALTER TABLE t").unwrap());
    }

    #[test]
    fn routine_type_refs_arguman_ve_donus() {
        use pgt_ddl::{ArgMode, FunctionArg, FunctionDef, Parallel, ReturnColumn, Volatility};
        let arg = |t: &str, mode: ArgMode| FunctionArg { mode, name: None, type_name: t.into(), default: None };
        let mut f = FunctionDef {
            schema: "s".into(),
            name: "f".into(),
            owner: None,
            comment: None,
            language: "sql".into(),
            args: vec![arg("\"Row T\"", ArgMode::In), arg("s.v[]", ArgMode::Inout), arg("integer", ArgMode::Out)],
            returns: Some(FunctionReturn::Type { type_name: "t".into(), setof: true }),
            body: "SELECT 1".into(),
            volatility: Volatility::Volatile,
            strict: false,
            security_definer: false,
            leakproof: false,
            parallel: Parallel::Unsafe,
            cost: None,
            rows: None,
            settings: vec![],
            recreate: false,
            editable: true,
            unsupported: vec![],
        };
        assert_eq!(routine_type_refs(&ObjectDef::Function(f.clone())), ["Row T", "s.v", "integer", "t"]);
        f.returns = Some(FunctionReturn::Table {
            columns: vec![ReturnColumn { name: "a".into(), type_name: "u".into() }],
        });
        f.args.clear();
        assert_eq!(routine_type_refs(&ObjectDef::Function(f.clone())), ["u"]);
        f.returns = None;
        assert!(routine_type_refs(&ObjectDef::Procedure(f)).is_empty());
        assert!(routine_type_refs(&type_item("e", TypeBody::Enum { values: vec![] })).is_empty());
    }

    #[test]
    fn type_refs_ornekleri() {
        let comp = type_item(
            "c",
            TypeBody::Composite {
                attributes: ["b[]", "\"My T\"", "numeric(10,2)", "s.\"T\"", "\"a\"\"b\"[][]"]
                    .iter()
                    .map(|t| CompositeAttr {
                        name: "x".into(),
                        type_name: (*t).into(),
                        collation: None,
                        original: None,
                    })
                    .collect(),
            },
        );
        assert_eq!(type_refs(&comp), ["b", "My T", "numeric", "s.T", "a\"b"]);
        let range = type_item(
            "r",
            TypeBody::Range {
                subtype: "timestamp(3)".into(),
                subtype_opclass: None,
                collation: None,
                canonical: None,
                subtype_diff: None,
                multirange_type_name: None,
            },
        );
        assert_eq!(type_refs(&range), ["timestamp"]);
        assert!(type_refs(&type_item("e", TypeBody::Enum { values: vec![] })).is_empty());
    }
}
