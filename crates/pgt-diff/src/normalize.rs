//! Karşılaştırma öncesi saf adımlar: normalizasyon (şema/seçenek/yükleme artıkları), kimlik eşlemesi
//! (`align_to_target`) ve nesne başına durum kararı (`prepare`). Veritabanına dokunmaz.

use std::collections::BTreeSet;
use std::mem::discriminant;

use pgt_ddl::{
    AclDef, AclGrant, ConstraintKind, IndexKey, ObjectDef, PrivilegeGrant, TypeBody,
    privileges_for, quote_ident, sort_grants,
};

use crate::model::{DiffOptions, ItemProblem, ItemStatus, SideTag};
use crate::snapshot::Loaded;

/// Normalizasyon bağlamı: kaynak/hedef şema adları ve karşılaştırma seçenekleri.
pub struct NormCtx<'a> {
    pub source_schema: &'a str,
    pub target_schema: &'a str,
    pub options: DiffOptions,
}

/// Kaynak taraftaki şema adını hedef şemaya çeviren küçük yardımcı.
struct SchemaMap<'a> {
    from: &'a str,
    to: &'a str,
    active: bool,
}

impl SchemaMap<'_> {
    fn apply(&self, s: &mut String) {
        if self.active && s == self.from {
            *s = self.to.to_string();
        }
    }
}

fn schema_map<'a>(side: SideTag, cx: &NormCtx<'a>) -> SchemaMap<'a> {
    SchemaMap {
        from: cx.source_schema,
        to: cx.target_schema,
        active: side == SideTag::Source && cx.source_schema != cx.target_schema,
    }
}

/// Tanımın `unsupported` listesi (ham, süzülmemiş); olmayan türlerde boş.
fn unsupported_field(def: &ObjectDef) -> &[String] {
    match def {
        ObjectDef::Table(d) => &d.unsupported,
        ObjectDef::View(d) => &d.unsupported,
        ObjectDef::MaterializedView(d) => &d.unsupported,
        ObjectDef::Function(d) | ObjectDef::Procedure(d) => &d.unsupported,
        ObjectDef::Trigger(d) => &d.unsupported,
        ObjectDef::Type(d) => &d.unsupported,
        ObjectDef::Domain(d) => &d.unsupported,
        _ => &[],
    }
}

/// Karşılaştırmaya giren `unsupported` girdileri: MV `INDEX ...` girdileri her zaman, görünüm/MV
/// `COMMENT ...` girdileri yorum seçeneği kapalıyken atılır.
pub fn unsupported_for_compare(def: &ObjectDef, options: &DiffOptions) -> Vec<String> {
    let is_mv = matches!(def, ObjectDef::MaterializedView(_));
    let is_view = is_mv || matches!(def, ObjectDef::View(_));
    unsupported_field(def)
        .iter()
        .filter(|u| !(is_mv && u.starts_with("INDEX ")))
        .filter(|u| !(is_view && !options.comments && u.starts_with("COMMENT ")))
        .cloned()
        .collect()
}

/// Tanımı karşılaştırılabilir biçime getirir (spec §2.3): kaynak şemayı hedefe çevirir, kapalı
/// seçeneklerin alanlarını boşaltır, yalnız yüklemede dolan alanları sıfırlar. Serbest SQL'e dokunmaz.
pub fn normalize(def: &ObjectDef, side: SideTag, cx: &NormCtx) -> ObjectDef {
    let mut d = def.clone();
    let tr = schema_map(side, cx);
    let o = cx.options;
    let us = unsupported_for_compare(def, &o);
    match &mut d {
        ObjectDef::Table(t) => {
            tr.apply(&mut t.schema);
            for c in &mut t.constraints {
                if let ConstraintKind::ForeignKey { ref_schema, .. } = &mut c.kind {
                    tr.apply(ref_schema);
                }
            }
            for c in &mut t.columns {
                c.using = None;
                if !o.comments {
                    c.comment = None;
                }
            }
            if !o.ownership {
                t.owner = None;
            }
            if !o.tablespace {
                t.tablespace = None;
            }
            if !o.comments {
                t.comment = None;
            }
            t.unsupported = us;
        }
        ObjectDef::Index(i) => {
            tr.apply(&mut i.schema);
            i.valid = true;
            if !o.tablespace {
                i.tablespace = None;
            }
            if !o.comments {
                i.comment = None;
            }
        }
        ObjectDef::Trigger(t) => {
            tr.apply(&mut t.schema);
            tr.apply(&mut t.function_schema);
            if !o.comments {
                t.comment = None;
            }
            t.unsupported = us;
        }
        ObjectDef::Sequence(s) => {
            tr.apply(&mut s.schema);
            if let Some(ob) = &mut s.owned_by {
                tr.apply(&mut ob.schema);
            }
            if !o.ownership {
                s.owner = None;
            }
            if !o.comments {
                s.comment = None;
            }
        }
        ObjectDef::View(v) => {
            tr.apply(&mut v.schema);
            v.columns = vec![];
            v.recreate = false;
            if !o.ownership {
                v.owner = None;
            }
            if !o.comments {
                v.comment = None;
            }
            v.unsupported = us;
        }
        ObjectDef::MaterializedView(m) => {
            tr.apply(&mut m.schema);
            m.columns = vec![];
            m.refresh = None;
            if !o.ownership {
                m.owner = None;
            }
            if !o.tablespace {
                m.tablespace = None;
            }
            if !o.comments {
                m.comment = None;
            }
            m.unsupported = us;
        }
        ObjectDef::Function(f) | ObjectDef::Procedure(f) => {
            tr.apply(&mut f.schema);
            f.recreate = false;
            if !o.ownership {
                f.owner = None;
            }
            if !o.comments {
                f.comment = None;
            }
            f.unsupported = us;
        }
        ObjectDef::Type(t) => {
            tr.apply(&mut t.schema);
            if !o.ownership {
                t.owner = None;
            }
            if !o.comments {
                t.comment = None;
            }
            t.unsupported = us;
        }
        ObjectDef::Domain(dm) => {
            tr.apply(&mut dm.schema);
            if !o.ownership {
                dm.owner = None;
            }
            if !o.comments {
                dm.comment = None;
            }
            dm.unsupported = us;
        }
        ObjectDef::Schema(s) => {
            if !o.ownership {
                s.owner = None;
            }
        }
        ObjectDef::Database(db) => {
            if !o.ownership {
                db.owner = None;
            }
        }
        ObjectDef::Extension(_) | ObjectDef::Role(_) => {}
    }
    d
}

/// ACL'nin nesne şemasını hedefe çevirir (yalnız kaynak tarafta).
pub fn normalize_acl(acl: &AclDef, side: SideTag, cx: &NormCtx) -> AclDef {
    let mut a = acl.clone();
    if let Some(s) = &mut a.schema {
        schema_map(side, cx).apply(s);
    }
    a
}

/// Karşılaştırma için: `unsupported` ve `editable` (yüklenen, ayrıca karşılaştırılan) alanlar nötrlenir.
fn comparable(def: &ObjectDef) -> ObjectDef {
    let mut d = def.clone();
    match &mut d {
        ObjectDef::Table(t) => t.unsupported.clear(),
        ObjectDef::View(v) => v.unsupported.clear(),
        ObjectDef::MaterializedView(m) => m.unsupported.clear(),
        ObjectDef::Function(f) | ObjectDef::Procedure(f) => {
            f.unsupported.clear();
            f.editable = true;
        }
        ObjectDef::Trigger(t) => {
            t.unsupported.clear();
            t.editable = true;
        }
        ObjectDef::Type(t) => {
            t.unsupported.clear();
            t.editable = true;
        }
        ObjectDef::Domain(dm) => {
            dm.unsupported.clear();
            dm.editable = true;
        }
        _ => {}
    }
    d
}

/// Kaynağı hedefin kimlikleriyle hizalar (kolon `attnum`, kısıt `oid`, enum/composite/domain
/// `original`, MV `with_data`); aynı tür değilse olduğu gibi döner. Döndürülen notlar kullanıcıya gider.
pub fn align_to_target(source: &ObjectDef, target: &ObjectDef) -> (ObjectDef, Vec<String>) {
    let mut s = source.clone();
    let mut notes = vec![];
    match (&mut s, target) {
        (ObjectDef::Table(sd), ObjectDef::Table(td)) => {
            let own_order: Vec<String> = sd.columns.iter().map(|c| c.name.clone()).collect();
            let mut aligned = Vec::with_capacity(sd.columns.len());
            for tc in &td.columns {
                if let Some(c) = sd.columns.iter().find(|c| c.name == tc.name) {
                    let mut c = c.clone();
                    c.attnum = tc.attnum;
                    aligned.push(c);
                }
            }
            for c in &sd.columns {
                if !td.columns.iter().any(|tc| tc.name == c.name) {
                    let mut c = c.clone();
                    c.attnum = None;
                    aligned.push(c);
                }
            }
            if own_order != aligned.iter().map(|c| c.name.clone()).collect::<Vec<_>>() {
                notes.push(
                    "Kolon sırası farklı (ALTER ile değiştirilemez; yeni kolonlar sona eklenir)."
                        .to_string(),
                );
            }
            sd.columns = aligned;
            for c in &mut sd.constraints {
                c.oid = c.name.as_ref().and_then(|n| {
                    td.constraints
                        .iter()
                        .find(|tc| tc.name.as_ref() == Some(n))
                        .and_then(|tc| tc.oid)
                });
            }
        }
        (ObjectDef::Type(sd), ObjectDef::Type(td)) => match (&mut sd.body, &td.body) {
            (TypeBody::Enum { values }, TypeBody::Enum { values: tv }) => {
                for v in values {
                    v.original = tv
                        .iter()
                        .any(|t| t.label == v.label)
                        .then(|| v.label.clone());
                }
            }
            (TypeBody::Composite { attributes }, TypeBody::Composite { attributes: ta }) => {
                for a in attributes {
                    a.original = ta.iter().any(|t| t.name == a.name).then(|| a.name.clone());
                }
            }
            _ => {}
        },
        (ObjectDef::Domain(sd), ObjectDef::Domain(td)) => {
            for c in &mut sd.constraints {
                c.original = c.name.as_ref().and_then(|n| {
                    td.constraints
                        .iter()
                        .any(|t| t.name.as_ref() == Some(n))
                        .then(|| n.clone())
                });
            }
        }
        (ObjectDef::MaterializedView(sd), ObjectDef::MaterializedView(td)) => {
            sd.with_data = td.with_data;
        }
        _ => {}
    }
    (s, notes)
}

/// `text` içinde `pat`, başında tanımlayıcı karakteri ya da `"` olmadan geçiyor mu?
fn has_ref(text: &str, pat: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(pat) {
        let at = from + i;
        let prev = text[..at].chars().next_back();
        if prev.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '"')) {
            return true;
        }
        from = at + pat.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn text_mentions(text: &str, schema: &str) -> bool {
    // Düz, `quote_ident` ve her zaman tırnaklı biçim (`"src".t` düz yazılabilen adlarda da geçerli).
    let forced = format!("\"{}\"", schema.replace('"', "\"\""));
    [
        format!("{schema}."),
        format!("{}.", quote_ident(schema)),
        format!("{forced}."),
    ]
    .iter()
    .any(|p| has_ref(text, p))
}

/// Serbest SQL alanlarından biri `schema.` ile başvuruyor mu (görünüm sorgusu, gövde, varsayılan, CHECK,
/// indeks ifadesi/WHERE, trigger WHEN, domain varsayılanı/CHECK'i).
pub fn mentions_schema(def: &ObjectDef, schema: &str) -> bool {
    let m = |t: &str| text_mentions(t, schema);
    let mo = |t: &Option<String>| t.as_deref().is_some_and(m);
    match def {
        ObjectDef::View(v) => m(&v.query),
        ObjectDef::MaterializedView(v) => m(&v.query),
        ObjectDef::Function(f) | ObjectDef::Procedure(f) => {
            m(&f.body) || f.args.iter().any(|a| mo(&a.default))
        }
        ObjectDef::Table(t) => {
            t.columns
                .iter()
                .any(|c| mo(&c.default) || c.generated.as_ref().is_some_and(|g| m(&g.expr)))
                || t.constraints
                    .iter()
                    .any(|c| matches!(&c.kind, ConstraintKind::Check { expr, .. } if m(expr)))
        }
        ObjectDef::Index(i) => {
            mo(&i.where_clause)
                || i.columns
                    .iter()
                    .any(|c| matches!(&c.key, IndexKey::Expression { sql } if m(sql)))
        }
        ObjectDef::Trigger(t) => mo(&t.when),
        ObjectDef::Domain(d) => mo(&d.default) || d.constraints.iter().any(|c| m(&c.expression)),
        _ => false,
    }
}

/// Alıcı bazında birleştirilmiş (aynı alıcı tek satır, yetkiler birleşir, `grant_option` OR'lanır),
/// kanonik sıralı grant listesi. `rename`: bu alıcı yerine geçecek ad (hedef sahibi -> kaynak sahibi).
pub(crate) fn canonical_grants(acl: &AclDef, rename: Option<(&str, &str)>) -> Vec<AclGrant> {
    let mut out: Vec<AclGrant> = vec![];
    for g in &acl.grants {
        let grantee = match (&g.grantee, rename) {
            (Some(n), Some((from, to))) if n == from => Some(to.to_string()),
            _ => g.grantee.clone(),
        };
        let slot = match out.iter().position(|o| o.grantee == grantee) {
            Some(i) => i,
            None => {
                out.push(AclGrant {
                    grantee,
                    privileges: vec![],
                });
                out.len() - 1
            }
        };
        for p in &g.privileges {
            match out[slot]
                .privileges
                .iter_mut()
                .find(|e| e.privilege == p.privilege)
            {
                Some(e) => e.grant_option |= p.grant_option,
                None => out[slot].privileges.push(PrivilegeGrant {
                    privilege: p.privilege,
                    grant_option: p.grant_option,
                }),
            }
        }
    }
    sort_grants(&mut out, privileges_for(acl.kind, 170000));
    out
}

/// İki ACL'nin sahip yetkileri dışındaki doğrudan grant'leri eşit mi? Hedef sahibinin satırı kaynak
/// sahibine çevrilir; `foreign`/`columns` karşılaştırılmaz.
pub fn acl_equal(source: &AclDef, target: &AclDef) -> bool {
    canonical_grants(source, None)
        == canonical_grants(target, Some((target.owner.as_str(), source.owner.as_str())))
}

/// `prepare` çıktısı: bir nesnenin durumu ve (üretim için) hazırlanmış tanımları.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub key: String,
    pub status: ItemStatus,
    pub problem: Option<ItemProblem>,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
    pub kind_changed: bool,
    /// Normalize + hedefe hizalanmış istenen durum.
    pub source: Option<ObjectDef>,
    /// Normalize edilmiş mevcut durum.
    pub target: Option<ObjectDef>,
    pub source_acl: Option<AclDef>,
    pub target_acl: Option<AclDef>,
}

fn side_parts(l: &Loaded, side: SideTag, cx: &NormCtx) -> Option<(ObjectDef, Option<AclDef>)> {
    let def = normalize(l.def.as_ref().ok()?, side, cx);
    let acl = match &l.acl {
        Some(Ok(a)) => Some(normalize_acl(a, side, cx)),
        _ => None,
    };
    Some((def, acl))
}

/// Kaynak tarafın "elle" gerekçeleri: modelin dışı özellikler, bölüm/kalıtım/bölümlü tablo.
/// Bölüm alt tablosu / kalıtım çocuğu gerekçeleri.
fn structure_reasons(def: &ObjectDef) -> Vec<String> {
    let mut r = vec![];
    if let ObjectDef::Table(t) = def {
        if t.is_partition {
            r.push("Bölüm alt tablosu (ATTACH modellenmez)".to_string());
        }
        if t.inherits {
            r.push("Kalıtım çocuğu (INHERITS modellenmez)".to_string());
        }
    }
    r
}

fn manual_reasons(def: &ObjectDef) -> Vec<String> {
    let mut r = vec![];
    let us = unsupported_field(def);
    if !us.is_empty() {
        r.push(format!("Modelin dışında özellikler var: {}", us.join(", ")));
    }
    if let ObjectDef::Table(t) = def {
        r.extend(structure_reasons(def));
        if t.partition_text.is_some() {
            r.push("Bölümlenmiş tablo oluşturma modellenmez".to_string());
        }
    }
    r
}

fn kinds_differ(a: &ObjectDef, b: &ObjectDef) -> bool {
    if a.kind() != b.kind() {
        return true;
    }
    match (a, b) {
        (ObjectDef::Type(x), ObjectDef::Type(y)) => discriminant(&x.body) != discriminant(&y.body),
        _ => false,
    }
}

/// Bir nesne çiftinin durumunu karar sırasına göre belirler (spec §2.3).
pub fn prepare(
    key: String,
    source: Option<&Loaded>,
    target: Option<&Loaded>,
    cx: &NormCtx,
) -> Prepared {
    let mut p = Prepared {
        key,
        status: ItemStatus::Same,
        problem: None,
        notes: vec![],
        errors: vec![],
        kind_changed: false,
        source: None,
        target: None,
        source_acl: None,
        target_acl: None,
    };
    // 1) Yükleme hatası.
    for (label, l) in [("Kaynak", source), ("Hedef", target)] {
        let Some(l) = l else { continue };
        if let Err(e) = &l.def {
            p.errors.push(format!("{label}: {e}"));
        }
        if let Some(Err(e)) = &l.acl {
            p.errors.push(format!("{label}: {e}"));
        }
    }
    let existence = match (source.is_some(), target.is_some()) {
        (true, false) => ItemStatus::OnlySource,
        (false, true) => ItemStatus::OnlyTarget,
        _ => ItemStatus::Different,
    };
    if !p.errors.is_empty() {
        p.status = existence;
        p.problem = Some(ItemProblem::LoadFailed);
        return p;
    }
    let s = source.and_then(|l| side_parts(l, SideTag::Source, cx));
    let t = target.and_then(|l| side_parts(l, SideTag::Target, cx));
    match (s, t) {
        // 2) Yalnız kaynak.
        (Some((sd, sa)), None) => {
            p.status = ItemStatus::OnlySource;
            let reasons = manual_reasons(&sd);
            if !reasons.is_empty() {
                p.problem = Some(ItemProblem::Manual);
                p.notes.extend(reasons);
            }
            p.source = Some(sd);
            p.source_acl = sa;
        }
        // 3) Yalnız hedef.
        (None, Some((td, ta))) => {
            p.status = ItemStatus::OnlyTarget;
            p.target = Some(td);
            p.target_acl = ta;
        }
        (Some((sd, sa)), Some((td, ta))) => {
            if kinds_differ(&sd, &td) {
                // 4) Tür ya da tip gövdesi değişti.
                p.status = ItemStatus::Different;
                p.kind_changed = true;
                let reasons = manual_reasons(&sd);
                if !reasons.is_empty() {
                    p.problem = Some(ItemProblem::Manual);
                    p.notes.extend(reasons);
                }
                p.source = Some(sd);
            } else {
                compare_same_kind(&mut p, sd, &td, &sa, &ta);
            }
            p.source_acl = sa;
            p.target = Some(td);
            p.target_acl = ta;
        }
        (None, None) => return p,
    }
    // 6) Gövdede kaynak şema adı geçiyor mu?
    if cx.source_schema != cx.target_schema
        && let Some(sd) = &p.source
        && mentions_schema(sd, cx.source_schema)
    {
        p.notes
            .push("Gövdede kaynak şema adı açıkça geçiyor; gözden geçirin.".to_string());
    }
    p
}

/// 5) İki taraf, aynı tür.
fn compare_same_kind(
    p: &mut Prepared,
    sd: ObjectDef,
    td: &ObjectDef,
    sa: &Option<AclDef>,
    ta: &Option<AclDef>,
) {
    let (aligned, align_notes) = align_to_target(&sd, td);
    p.notes.extend(align_notes);
    let us_s: BTreeSet<&String> = unsupported_field(&aligned).iter().collect();
    let us_t: BTreeSet<&String> = unsupported_field(td).iter().collect();
    let acl_equal_state = match (sa, ta) {
        (Some(a), Some(b)) => {
            if a.foreign != b.foreign || a.columns != b.columns {
                p.notes.push(
                    "Kolon düzeyi ve başka verenlerin yetkileri karşılaştırılmaz.".to_string(),
                );
            }
            acl_equal(a, b)
        }
        _ => true,
    };
    let unsupported_differs = us_s != us_t;
    if unsupported_differs {
        let diff: Vec<&str> = us_s
            .symmetric_difference(&us_t)
            .map(|s| s.as_str())
            .collect();
        p.problem = Some(ItemProblem::Manual);
        p.notes
            .push(format!("Modelin dışında fark var: {}", diff.join(", ")));
    }
    if comparable(&aligned) == comparable(td) {
        p.status = if unsupported_differs || !acl_equal_state {
            ItemStatus::Different
        } else {
            ItemStatus::Same
        };
    } else {
        p.status = ItemStatus::Different;
        if let (ObjectDef::Table(a), ObjectDef::Table(b)) = (&aligned, td) {
            for r in structure_reasons(&aligned)
                .into_iter()
                .chain(structure_reasons(td))
            {
                p.problem = Some(ItemProblem::Manual);
                if !p.notes.contains(&r) {
                    p.notes.push(r);
                }
            }
            if a.partition_text != b.partition_text {
                p.problem = Some(ItemProblem::Manual);
                p.notes.push("Bölümleme anahtarı farklı".to_string());
            }
        }
    }
    p.source = Some(aligned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Listed;
    use pgt_ddl::{
        ArgMode, ColumnDef, ColumnRef, CompositeAttr, ConstraintDef, DomainCheck, DomainDef,
        EnumValue, FkAction, FkMatch, FunctionArg, FunctionDef, IndexColumn, IndexDef, MatViewDef,
        ObjectKind, Parallel, Persistence, Privilege, SeqType, SequenceDef, TableDef, TriggerDef,
        TriggerEnabled, TriggerEvents, TriggerLevel, TriggerTiming, TypeDef, ViewDef, Volatility,
    };

    fn opts(ownership: bool, tablespace: bool, comments: bool) -> DiffOptions {
        DiffOptions {
            ownership,
            tablespace,
            comments,
        }
    }

    fn cx(o: DiffOptions) -> NormCtx<'static> {
        NormCtx {
            source_schema: "src",
            target_schema: "tgt",
            options: o,
        }
    }

    fn col(name: &str, attnum: Option<i16>) -> ColumnDef {
        ColumnDef {
            attnum,
            name: name.into(),
            data_type: "integer".into(),
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            collation: None,
            comment: Some("kc".into()),
            using: Some("x::int".into()),
        }
    }

    fn con(name: &str, oid: Option<u32>, kind: ConstraintKind) -> ConstraintDef {
        ConstraintDef {
            oid,
            name: Some(name.into()),
            kind,
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        }
    }

    fn fk(ref_schema: &str) -> ConstraintKind {
        ConstraintKind::ForeignKey {
            columns: vec!["a".into()],
            ref_schema: ref_schema.into(),
            ref_table: "u".into(),
            ref_columns: vec!["id".into()],
            match_type: FkMatch::Simple,
            on_update: FkAction::NoAction,
            on_delete: FkAction::NoAction,
        }
    }

    fn table(schema: &str, cols: Vec<ColumnDef>) -> TableDef {
        TableDef {
            schema: schema.into(),
            name: "t".into(),
            owner: Some("alice".into()),
            tablespace: Some("ts".into()),
            persistence: Persistence::Permanent,
            columns: cols,
            constraints: vec![],
            partition_by: None,
            fillfactor: None,
            comment: Some("tc".into()),
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        }
    }

    fn view(schema: &str, query: &str) -> ViewDef {
        ViewDef {
            schema: schema.into(),
            name: "v".into(),
            owner: Some("alice".into()),
            query: query.into(),
            check_option: None,
            security_barrier: false,
            security_invoker: false,
            comment: Some("vc".into()),
            recreate: true,
            columns: vec![pgt_ddl::ViewColumn {
                name: "a".into(),
                data_type: "integer".into(),
            }],
            unsupported: vec![],
        }
    }

    fn matview(schema: &str) -> MatViewDef {
        MatViewDef {
            schema: schema.into(),
            name: "mv".into(),
            owner: Some("alice".into()),
            query: "SELECT 1".into(),
            tablespace: Some("ts".into()),
            fillfactor: None,
            with_data: true,
            comment: Some("mc".into()),
            refresh: Some(pgt_ddl::MatViewRefresh::Normal),
            columns: vec![pgt_ddl::ViewColumn {
                name: "a".into(),
                data_type: "integer".into(),
            }],
            unsupported: vec![],
        }
    }

    fn func(schema: &str, body: &str) -> FunctionDef {
        FunctionDef {
            schema: schema.into(),
            name: "f".into(),
            owner: Some("alice".into()),
            comment: Some("fc".into()),
            language: "sql".into(),
            args: vec![],
            returns: None,
            body: body.into(),
            volatility: Volatility::Volatile,
            strict: false,
            security_definer: false,
            leakproof: false,
            parallel: Parallel::Unsafe,
            cost: None,
            rows: None,
            settings: vec![],
            recreate: true,
            editable: true,
            unsupported: vec![],
        }
    }

    fn enum_type(labels: &[&str]) -> TypeDef {
        TypeDef {
            schema: "src".into(),
            name: "mood".into(),
            owner: None,
            comment: None,
            body: TypeBody::Enum {
                values: labels
                    .iter()
                    .map(|l| EnumValue {
                        label: (*l).into(),
                        original: None,
                    })
                    .collect(),
            },
            editable: true,
            unsupported: vec![],
        }
    }

    fn domain(checks: Vec<DomainCheck>) -> DomainDef {
        DomainDef {
            schema: "src".into(),
            name: "d".into(),
            owner: None,
            comment: None,
            base_type: "integer".into(),
            collation: None,
            default: None,
            not_null: false,
            constraints: checks,
            editable: true,
            unsupported: vec![],
        }
    }

    fn check(name: Option<&str>, expr: &str) -> DomainCheck {
        DomainCheck {
            name: name.map(Into::into),
            expression: expr.into(),
            not_valid: false,
            validated: true,
            original: None,
            validate: false,
        }
    }

    fn unwrap_table(d: &ObjectDef) -> &TableDef {
        match d {
            ObjectDef::Table(t) => t,
            _ => panic!("tablo bekleniyordu"),
        }
    }

    // ---- normalize ----

    #[test]
    fn sema_cevirisi_kaynakta_tum_yapisal_alanlari_cevirir() {
        let c = cx(opts(true, true, true));
        let mut t = table("src", vec![col("a", Some(1))]);
        t.constraints = vec![con("fk1", None, fk("src")), con("fk2", None, fk("other"))];
        let n = normalize(&ObjectDef::Table(t.clone()), SideTag::Source, &c);
        let nt = unwrap_table(&n);
        assert_eq!(nt.schema, "tgt");
        let refs: Vec<_> = nt
            .constraints
            .iter()
            .map(|c| match &c.kind {
                ConstraintKind::ForeignKey { ref_schema, .. } => ref_schema.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(refs, ["tgt", "other"]);
        // Hedef tarafta çeviri yok.
        let n = normalize(&ObjectDef::Table(t), SideTag::Target, &c);
        assert_eq!(unwrap_table(&n).schema, "src");

        let idx = ObjectDef::Index(IndexDef {
            schema: "src".into(),
            table: "t".into(),
            name: "i".into(),
            method: "btree".into(),
            unique: false,
            columns: vec![],
            include: vec![],
            where_clause: None,
            tablespace: None,
            fillfactor: None,
            comment: None,
            constraint: None,
            valid: false,
        });
        let ObjectDef::Index(i) = normalize(&idx, SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!((i.schema.as_str(), i.valid), ("tgt", true));

        let trg = ObjectDef::Trigger(TriggerDef {
            schema: "src".into(),
            table: "t".into(),
            name: "g".into(),
            timing: TriggerTiming::Before,
            events: TriggerEvents {
                insert: true,
                update: false,
                update_columns: vec![],
                delete: false,
                truncate: false,
            },
            level: TriggerLevel::Row,
            when: None,
            function_schema: "src".into(),
            function_name: "f".into(),
            args: vec![],
            old_table: None,
            new_table: None,
            constraint: false,
            deferrable: false,
            initially_deferred: false,
            enabled: TriggerEnabled::Origin,
            comment: None,
            editable: true,
            unsupported: vec![],
        });
        let ObjectDef::Trigger(g) = normalize(&trg, SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!(
            (g.schema.as_str(), g.function_schema.as_str()),
            ("tgt", "tgt")
        );
        // Başka şemadaki trigger fonksiyonu değişmez.
        let ObjectDef::Trigger(mut g0) = trg else {
            panic!()
        };
        g0.function_schema = "other".into();
        let ObjectDef::Trigger(g) = normalize(&ObjectDef::Trigger(g0), SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!(g.function_schema, "other");

        let seq = ObjectDef::Sequence(SequenceDef {
            schema: "src".into(),
            name: "s".into(),
            owner: None,
            data_type: SeqType::Bigint,
            increment: 1,
            min_value: None,
            max_value: None,
            start: 1,
            cache: 1,
            cycle: false,
            owned_by: Some(ColumnRef {
                schema: "src".into(),
                table: "t".into(),
                column: "a".into(),
            }),
            comment: None,
        });
        let ObjectDef::Sequence(s) = normalize(&seq, SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!(
            (s.schema.as_str(), s.owned_by.unwrap().schema.as_str()),
            ("tgt", "tgt")
        );

        for d in [
            ObjectDef::View(view("src", "SELECT 1")),
            ObjectDef::MaterializedView(matview("src")),
            ObjectDef::Function(func("src", "SELECT 1")),
            ObjectDef::Procedure(func("src", "SELECT 1")),
            ObjectDef::Type(enum_type(&["a"])),
            ObjectDef::Domain(domain(vec![])),
        ] {
            let kind = d.kind();
            let n = normalize(&d, SideTag::Source, &c);
            let schema = match &n {
                ObjectDef::View(x) => &x.schema,
                ObjectDef::MaterializedView(x) => &x.schema,
                ObjectDef::Function(x) | ObjectDef::Procedure(x) => &x.schema,
                ObjectDef::Type(x) => &x.schema,
                ObjectDef::Domain(x) => &x.schema,
                _ => unreachable!(),
            };
            assert_eq!(schema, "tgt", "{kind:?}");
        }
    }

    #[test]
    fn serbest_sql_cevrilmez() {
        let c = cx(opts(true, true, true));
        let v = ObjectDef::View(view("src", "SELECT * FROM src.t"));
        let ObjectDef::View(n) = normalize(&v, SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!(n.query, "SELECT * FROM src.t");
        let f = ObjectDef::Function(func("src", "SELECT src.g()"));
        let ObjectDef::Function(n) = normalize(&f, SideTag::Source, &c) else {
            panic!()
        };
        assert_eq!(n.body, "SELECT src.g()");
    }

    #[test]
    fn ayni_sema_adinda_ceviri_yapilmaz() {
        let c = NormCtx {
            source_schema: "s",
            target_schema: "s",
            options: opts(true, true, true),
        };
        let n = normalize(&ObjectDef::Table(table("s", vec![])), SideTag::Source, &c);
        assert_eq!(unwrap_table(&n).schema, "s");
    }

    #[test]
    fn yukleme_artiklari_sifirlanir_ve_partition_text_korunur() {
        let c = cx(opts(true, true, true));
        let mut t = table("src", vec![col("a", Some(1))]);
        t.partition_text = Some("RANGE (a)".into());
        let n = normalize(&ObjectDef::Table(t), SideTag::Source, &c);
        let nt = unwrap_table(&n);
        assert!(nt.columns[0].using.is_none());
        assert_eq!(nt.partition_text.as_deref(), Some("RANGE (a)"));

        let ObjectDef::View(v) = normalize(&ObjectDef::View(view("src", "q")), SideTag::Source, &c)
        else {
            panic!()
        };
        assert!(v.columns.is_empty() && !v.recreate);
        let ObjectDef::MaterializedView(m) = normalize(
            &ObjectDef::MaterializedView(matview("src")),
            SideTag::Source,
            &c,
        ) else {
            panic!()
        };
        assert!(m.columns.is_empty() && m.refresh.is_none() && m.with_data);
        let ObjectDef::Function(f) =
            normalize(&ObjectDef::Function(func("src", "b")), SideTag::Source, &c)
        else {
            panic!()
        };
        assert!(!f.recreate);
    }

    #[test]
    fn secenek_matrisi() {
        let all_on = cx(opts(true, true, true));
        let all_off = cx(opts(false, false, false));
        let t = ObjectDef::Table(table("src", vec![col("a", Some(1))]));
        let on = normalize(&t, SideTag::Target, &all_on);
        let off = normalize(&t, SideTag::Target, &all_off);
        let (on, off) = (unwrap_table(&on), unwrap_table(&off));
        assert_eq!(
            (
                on.owner.as_deref(),
                on.tablespace.as_deref(),
                on.comment.as_deref()
            ),
            (Some("alice"), Some("ts"), Some("tc"))
        );
        assert_eq!(on.columns[0].comment.as_deref(), Some("kc"));
        assert_eq!(
            (&off.owner, &off.tablespace, &off.comment),
            (&None, &None, &None)
        );
        assert_eq!(off.columns[0].comment, None);

        // Yalnız tablespace kapalı: MV ve indeks tablespace'i düşer, sahip kalır.
        let c = cx(opts(true, false, true));
        let ObjectDef::MaterializedView(m) = normalize(
            &ObjectDef::MaterializedView(matview("src")),
            SideTag::Target,
            &c,
        ) else {
            panic!()
        };
        assert_eq!(
            (m.tablespace, m.owner.as_deref(), m.comment.as_deref()),
            (None, Some("alice"), Some("mc"))
        );

        // Yorum kapalı: görünüm yorumu ve `COMMENT c` girdisi düşer; `TRIGGER t` kalır.
        let c = cx(opts(true, true, false));
        let mut v = view("src", "q");
        v.unsupported = vec!["TRIGGER t".into(), "COMMENT c".into()];
        let ObjectDef::View(n) = normalize(&ObjectDef::View(v.clone()), SideTag::Target, &c) else {
            panic!()
        };
        assert_eq!(
            (n.comment, n.unsupported),
            (None, vec!["TRIGGER t".to_string()])
        );
        // Yorum açıkken `COMMENT c` kalır.
        let on = cx(opts(true, true, true));
        let ObjectDef::View(n) = normalize(&ObjectDef::View(v), SideTag::Target, &on) else {
            panic!()
        };
        assert_eq!(n.unsupported, ["TRIGGER t", "COMMENT c"]);
    }

    #[test]
    fn unsupported_for_compare_suzer() {
        let mut m = matview("s");
        m.unsupported = vec!["INDEX i".into(), "COMMENT c".into(), "WITH (x=1)".into()];
        let d = ObjectDef::MaterializedView(m);
        assert_eq!(
            unsupported_for_compare(&d, &opts(false, false, true)),
            ["COMMENT c", "WITH (x=1)"]
        );
        assert_eq!(
            unsupported_for_compare(&d, &opts(false, false, false)),
            ["WITH (x=1)"]
        );
        // Tabloda `INDEX ` öneki süzülmez.
        let mut t = table("s", vec![]);
        t.unsupported = vec!["INDEX i".into()];
        assert_eq!(
            unsupported_for_compare(&ObjectDef::Table(t), &opts(false, false, false)),
            ["INDEX i"]
        );
    }

    // ---- mentions_schema ----

    #[test]
    fn mentions_schema_ornekleri() {
        let v = |q: &str| ObjectDef::View(view("src", q));
        assert!(mentions_schema(&v("SELECT * FROM src.t"), "src"));
        assert!(mentions_schema(&v("SELECT * FROM \"src\".t"), "src"));
        assert!(!mentions_schema(&v("SELECT * FROM srcx.t"), "src"));
        assert!(!mentions_schema(&v("SELECT a.src FROM a"), "src"));
        assert!(!mentions_schema(&v("SELECT * FROM xsrc.t"), "src"));
        assert!(mentions_schema(&v("SELECT * FROM \"My Src\".t"), "My Src"));
        assert!(mentions_schema(&v("(src.t)"), "src"));

        let mut f = func("src", "SELECT 1");
        f.args = vec![FunctionArg {
            mode: ArgMode::In,
            name: Some("x".into()),
            type_name: "integer".into(),
            default: Some("src.nextval()".into()),
        }];
        assert!(mentions_schema(&ObjectDef::Function(f), "src"));

        let mut t = table("src", vec![]);
        t.constraints = vec![con(
            "ck",
            None,
            ConstraintKind::Check {
                expr: "src.ok(a)".into(),
                no_inherit: false,
            },
        )];
        assert!(mentions_schema(&ObjectDef::Table(t), "src"));

        let mut c = col("a", None);
        c.default = Some("src.f()".into());
        assert!(mentions_schema(
            &ObjectDef::Table(table("src", vec![c])),
            "src"
        ));

        let d = domain(vec![check(Some("c"), "src.ok(VALUE)")]);
        assert!(mentions_schema(&ObjectDef::Domain(d), "src"));

        let i = ObjectDef::Index(IndexDef {
            schema: "src".into(),
            table: "t".into(),
            name: "i".into(),
            method: "btree".into(),
            unique: false,
            columns: vec![IndexColumn {
                key: IndexKey::Expression {
                    sql: "src.f(a)".into(),
                },
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
        });
        assert!(mentions_schema(&i, "src"));
        assert!(!mentions_schema(
            &ObjectDef::Table(table("src", vec![])),
            "src"
        ));
    }

    // ---- align_to_target ----

    #[test]
    fn align_tablo_attnum_oid_ve_yeni_kolon() {
        let mut s = table(
            "tgt",
            vec![col("b", Some(7)), col("new", Some(8)), col("a", Some(9))],
        );
        s.constraints = vec![
            con(
                "pk",
                Some(1),
                ConstraintKind::PrimaryKey {
                    columns: vec!["a".into()],
                    include: vec![],
                },
            ),
            con(
                "fresh",
                Some(2),
                ConstraintKind::Check {
                    expr: "a > 0".into(),
                    no_inherit: false,
                },
            ),
        ];
        let mut t = table("tgt", vec![col("a", Some(1)), col("b", Some(2))]);
        t.constraints = vec![con(
            "pk",
            Some(500),
            ConstraintKind::PrimaryKey {
                columns: vec!["a".into()],
                include: vec![],
            },
        )];
        let (a, notes) = align_to_target(&ObjectDef::Table(s), &ObjectDef::Table(t));
        let at = unwrap_table(&a);
        let cols: Vec<_> = at
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.attnum))
            .collect();
        assert_eq!(cols, [("a", Some(1)), ("b", Some(2)), ("new", None)]);
        assert_eq!(at.constraints[0].oid, Some(500));
        assert_eq!(at.constraints[1].oid, None);
        assert_eq!(
            notes,
            ["Kolon sırası farklı (ALTER ile değiştirilemez; yeni kolonlar sona eklenir)."]
        );
    }

    #[test]
    fn align_ayni_sirada_not_yok() {
        let s = table("t", vec![col("a", Some(5)), col("n", Some(6))]);
        let t = table("t", vec![col("a", Some(1))]);
        let (_, notes) = align_to_target(&ObjectDef::Table(s), &ObjectDef::Table(t));
        assert!(notes.is_empty());
    }

    #[test]
    fn align_enum_composite_domain_mv() {
        let (a, _) = align_to_target(
            &ObjectDef::Type(enum_type(&["a", "mid", "b"])),
            &ObjectDef::Type(enum_type(&["a", "b"])),
        );
        let ObjectDef::Type(TypeDef {
            body: TypeBody::Enum { values },
            ..
        }) = a
        else {
            panic!()
        };
        let orig: Vec<_> = values.iter().map(|v| v.original.clone()).collect();
        insta::assert_debug_snapshot!(orig);

        let comp = |names: &[&str]| {
            let mut t = enum_type(&[]);
            t.body = TypeBody::Composite {
                attributes: names
                    .iter()
                    .map(|n| CompositeAttr {
                        name: (*n).into(),
                        type_name: "integer".into(),
                        collation: None,
                        original: None,
                    })
                    .collect(),
            };
            ObjectDef::Type(t)
        };
        let (a, _) = align_to_target(&comp(&["x", "y"]), &comp(&["y"]));
        let ObjectDef::Type(TypeDef {
            body: TypeBody::Composite { attributes },
            ..
        }) = a
        else {
            panic!()
        };
        let orig: Vec<_> = attributes.iter().map(|v| v.original.clone()).collect();
        assert_eq!(orig, [None, Some("y".to_string())]);

        let (a, _) = align_to_target(
            &ObjectDef::Domain(domain(vec![
                check(Some("c1"), "x"),
                check(Some("c2"), "y"),
                check(None, "z"),
            ])),
            &ObjectDef::Domain(domain(vec![check(Some("c2"), "y"), check(None, "z")])),
        );
        let ObjectDef::Domain(d) = a else { panic!() };
        let orig: Vec<_> = d.constraints.iter().map(|c| c.original.clone()).collect();
        assert_eq!(orig, [None, Some("c2".to_string()), None]);

        let mut t = matview("s");
        t.with_data = false;
        let (a, _) = align_to_target(
            &ObjectDef::MaterializedView(matview("s")),
            &ObjectDef::MaterializedView(t),
        );
        let ObjectDef::MaterializedView(m) = a else {
            panic!()
        };
        assert!(!m.with_data);
    }

    // ---- acl_equal ----

    fn acl(owner: &str, grants: Vec<AclGrant>) -> AclDef {
        AclDef {
            kind: ObjectKind::Table,
            schema: Some("s".into()),
            name: "t".into(),
            signature: None,
            owner: owner.into(),
            viewer: "v".into(),
            explicit: true,
            grants,
            foreign: vec![],
            columns: vec![],
            editable: true,
            unsupported: vec![],
            available: vec![],
        }
    }

    fn grant(grantee: Option<&str>, privs: &[(Privilege, bool)]) -> AclGrant {
        AclGrant {
            grantee: grantee.map(Into::into),
            privileges: privs
                .iter()
                .map(|(p, g)| PrivilegeGrant {
                    privilege: *p,
                    grant_option: *g,
                })
                .collect(),
        }
    }

    #[test]
    fn acl_equal_sahip_eslemesi_ve_birlesme() {
        use Privilege::*;
        let s = acl(
            "alice",
            vec![
                grant(Some("alice"), &[(Select, false), (Insert, true)]),
                grant(Some("bob"), &[(Select, false)]),
            ],
        );
        // Hedefte sahip farklı; satırı yeniden adlandırılmış, yetkiler iki satıra bölünmüş.
        let t = acl(
            "carol",
            vec![
                grant(Some("bob"), &[(Select, false)]),
                grant(Some("carol"), &[(Insert, true)]),
                grant(Some("carol"), &[(Select, false), (Insert, false)]),
            ],
        );
        assert!(acl_equal(&s, &t));
    }

    #[test]
    fn acl_equal_public_farki_ve_grant_option() {
        use Privilege::*;
        let s = acl("a", vec![grant(None, &[(Select, false)])]);
        let t = acl("a", vec![]);
        assert!(!acl_equal(&s, &t));
        let s = acl("a", vec![grant(Some("b"), &[(Select, true)])]);
        let t = acl("a", vec![grant(Some("b"), &[(Select, false)])]);
        assert!(!acl_equal(&s, &t));
    }

    // ---- prepare / durum tablosu ----

    fn loaded(def: ObjectDef, a: Option<AclDef>) -> Loaded {
        Loaded {
            listed: Listed {
                kind: def.kind(),
                oid: 1,
                name: def.name().into(),
                table: None,
                signature: None,
                is_partition: false,
                deps: vec![],
            },
            def: Ok(def),
            acl: a.map(Ok),
        }
    }

    fn lt(t: TableDef) -> Loaded {
        loaded(ObjectDef::Table(t), None)
    }

    fn prep(s: Option<&Loaded>, t: Option<&Loaded>, o: DiffOptions) -> Prepared {
        prepare("k".into(), s, t, &cx(o))
    }

    fn st(p: &Prepared) -> (ItemStatus, Option<ItemProblem>) {
        (p.status, p.problem)
    }

    const DEF: DiffOptions = DiffOptions {
        ownership: false,
        tablespace: false,
        comments: true,
    };

    #[test]
    fn durum_tablosu_temel() {
        use ItemProblem::*;
        use ItemStatus::*;
        let a = lt(table("src", vec![col("a", Some(1))]));
        let b = lt(table("tgt", vec![col("a", Some(1))]));
        let mut diff_t = table("tgt", vec![col("a", Some(1)), col("z", Some(2))]);
        diff_t.name = "t".into();
        let diff = lt(diff_t);

        assert_eq!(st(&prep(Some(&a), None, DEF)), (OnlySource, None));
        assert_eq!(st(&prep(None, Some(&b), DEF)), (OnlyTarget, None));
        assert_eq!(st(&prep(Some(&a), Some(&b), DEF)), (Same, None));
        assert_eq!(st(&prep(Some(&a), Some(&diff), DEF)), (Different, None));

        // Yükleme hatası: varlığa göre durum.
        let bad = Loaded {
            def: Err("hata".into()),
            ..lt(table("src", vec![]))
        };
        let p = prep(Some(&bad), None, DEF);
        assert_eq!(
            (st(&p), p.errors.len()),
            ((OnlySource, Some(LoadFailed)), 1)
        );
        assert!(p.errors[0].contains("hata"));
        assert_eq!(
            st(&prep(None, Some(&bad), DEF)),
            (OnlyTarget, Some(LoadFailed))
        );
        assert_eq!(
            st(&prep(Some(&a), Some(&bad), DEF)),
            (Different, Some(LoadFailed))
        );
        // ACL hatası da yükleme hatası.
        let bad_acl = Loaded {
            acl: Some(Err("yetki".into())),
            ..lt(table("src", vec![]))
        };
        assert_eq!(
            st(&prep(Some(&bad_acl), Some(&b), DEF)),
            (Different, Some(LoadFailed))
        );
    }

    #[test]
    fn durum_tablosu_tur_degisimi() {
        use ItemProblem::*;
        use ItemStatus::*;
        let t = lt(table("src", vec![]));
        let v = loaded(ObjectDef::View(view("tgt", "SELECT 1")), None);
        let p = prep(Some(&t), Some(&v), DEF);
        assert_eq!((st(&p), p.kind_changed), ((Different, None), true));
        assert!(p.source.is_some() && p.target.is_some());

        let e = loaded(ObjectDef::Type(enum_type(&["a"])), None);
        let mut ct = enum_type(&[]);
        ct.body = TypeBody::Composite { attributes: vec![] };
        let c = loaded(ObjectDef::Type(ct), None);
        let p = prep(Some(&e), Some(&c), DEF);
        assert_eq!((st(&p), p.kind_changed), ((Different, None), true));

        // Kaynak tarafın "elle" koşulu tür değişiminde de geçerli.
        let mut part = table("src", vec![]);
        part.is_partition = true;
        let p = prep(Some(&lt(part)), Some(&v), DEF);
        assert_eq!((st(&p), p.kind_changed), ((Different, Some(Manual)), true));
    }

    #[test]
    fn durum_tablosu_elle_kosullari_yalniz_kaynak() {
        use ItemProblem::*;
        use ItemStatus::*;
        let mut part = table("src", vec![]);
        part.is_partition = true;
        let p = prep(Some(&lt(part)), None, DEF);
        assert_eq!(st(&p), (OnlySource, Some(Manual)));
        assert_eq!(p.notes, ["Bölüm alt tablosu (ATTACH modellenmez)"]);

        let mut inh = table("src", vec![]);
        inh.inherits = true;
        let p = prep(Some(&lt(inh)), None, DEF);
        assert_eq!(st(&p), (OnlySource, Some(Manual)));
        assert_eq!(p.notes, ["Kalıtım çocuğu (INHERITS modellenmez)"]);

        let mut pt = table("src", vec![]);
        pt.partition_text = Some("RANGE (a)".into());
        let p = prep(Some(&lt(pt)), None, DEF);
        assert_eq!(st(&p), (OnlySource, Some(Manual)));
        assert_eq!(p.notes, ["Bölümlenmiş tablo oluşturma modellenmez"]);

        let mut un = table("src", vec![]);
        un.unsupported = vec!["EXCLUDE ex".into(), "OF t".into()];
        let p = prep(Some(&lt(un.clone())), None, DEF);
        assert_eq!(st(&p), (OnlySource, Some(Manual)));
        assert_eq!(
            p.notes,
            ["Modelin dışında özellikler var: EXCLUDE ex, OF t"]
        );

        // Yalnız hedefte aynı özellikler sorun değildir.
        assert_eq!(st(&prep(None, Some(&lt(un)), DEF)), (OnlyTarget, None));
    }

    #[test]
    fn durum_tablosu_bolumleme_farki() {
        use ItemProblem::*;
        use ItemStatus::*;
        let mut a = table("src", vec![col("a", Some(1))]);
        a.partition_text = Some("RANGE (a)".into());
        let mut b = table("tgt", vec![col("a", Some(1))]);
        b.partition_text = Some("LIST (a)".into());
        let p = prep(Some(&lt(a.clone())), Some(&lt(b)), DEF);
        assert_eq!(st(&p), (Different, Some(Manual)));
        assert!(p.notes.contains(&"Bölümleme anahtarı farklı".to_string()));
        // Aynı bölümleme metni -> Same.
        let mut b2 = table("tgt", vec![col("a", Some(1))]);
        b2.partition_text = Some("RANGE (a)".into());
        assert_eq!(st(&prep(Some(&lt(a)), Some(&lt(b2)), DEF)), (Same, None));
        // Hedefte bölüm alt tablosu + fark -> Manual.
        let s = table("src", vec![col("a", Some(1)), col("x", Some(2))]);
        let mut t = table("tgt", vec![col("a", Some(1))]);
        t.is_partition = true;
        assert_eq!(
            st(&prep(Some(&lt(s)), Some(&lt(t)), DEF)),
            (Different, Some(Manual))
        );
    }

    #[test]
    fn unsupported_farki_modeller_farkliyken_de_elle() {
        use ItemProblem::*;
        use ItemStatus::*;
        let mut s = table("src", vec![col("a", Some(1)), col("x", Some(2))]);
        s.unsupported = vec!["EXCLUDE ex".into()];
        let t = table("tgt", vec![col("a", Some(1))]);
        let p = prep(Some(&lt(s)), Some(&lt(t)), DEF);
        assert_eq!(st(&p), (Different, Some(Manual)));
        assert!(
            p.notes
                .contains(&"Modelin dışında fark var: EXCLUDE ex".to_string())
        );

        let mut v = view("src", "SELECT 1");
        v.unsupported = vec!["TRIGGER t".into()];
        let sv = loaded(ObjectDef::View(v), None);
        let tv = loaded(ObjectDef::View(view("tgt", "SELECT 2")), None);
        let p = prep(Some(&sv), Some(&tv), DEF);
        assert_eq!(st(&p), (Different, Some(Manual)));
        assert!(
            p.notes
                .contains(&"Modelin dışında fark var: TRIGGER t".to_string())
        );
    }

    #[test]
    fn iki_tarafli_bolum_kalitim_gerekcesi_notu() {
        use ItemProblem::*;
        use ItemStatus::*;
        let s = table("src", vec![col("a", Some(1)), col("x", Some(2))]);
        let mut t = table("tgt", vec![col("a", Some(1))]);
        t.is_partition = true;
        t.inherits = true;
        let p = prep(Some(&lt(s)), Some(&lt(t)), DEF);
        assert_eq!(st(&p), (Different, Some(Manual)));
        assert!(
            p.notes
                .contains(&"Bölüm alt tablosu (ATTACH modellenmez)".to_string())
        );
        assert!(
            p.notes
                .contains(&"Kalıtım çocuğu (INHERITS modellenmez)".to_string())
        );
    }

    #[test]
    fn durum_tablosu_mv_ve_view() {
        use ItemProblem::*;
        use ItemStatus::*;
        // MV yalnız with_data farkı -> Same.
        let mut t = matview("tgt");
        t.with_data = false;
        let s = loaded(ObjectDef::MaterializedView(matview("src")), None);
        let t = loaded(ObjectDef::MaterializedView(t), None);
        assert_eq!(st(&prep(Some(&s), Some(&t), DEF)), (Same, None));
        // MV yalnız INDEX i farkı -> Same.
        let mut m = matview("src");
        m.unsupported = vec!["INDEX i".into()];
        let s = loaded(ObjectDef::MaterializedView(m), None);
        let t = loaded(ObjectDef::MaterializedView(matview("tgt")), None);
        assert_eq!(st(&prep(Some(&s), Some(&t), DEF)), (Same, None));
        // Görünüm TRIGGER t farkı -> Different + Manual.
        let mut v = view("src", "SELECT 1");
        v.unsupported = vec!["TRIGGER t".into()];
        let s = loaded(ObjectDef::View(v), None);
        let t = loaded(ObjectDef::View(view("tgt", "SELECT 1")), None);
        let p = prep(Some(&s), Some(&t), DEF);
        assert_eq!(st(&p), (Different, Some(Manual)));
        assert_eq!(p.notes, ["Modelin dışında fark var: TRIGGER t"]);
    }

    #[test]
    fn durum_tablosu_kolon_sirasi_ve_acl() {
        use ItemStatus::*;
        let s = lt(table("src", vec![col("b", Some(1)), col("a", Some(2))]));
        let t = lt(table("tgt", vec![col("a", Some(1)), col("b", Some(2))]));
        let p = prep(Some(&s), Some(&t), DEF);
        assert_eq!(st(&p), (Same, None));
        assert_eq!(p.notes.len(), 1);
        assert!(p.notes[0].starts_with("Kolon sırası farklı"));

        // Sahiplik açıkken yalnız GRANT farkı -> Different.
        use Privilege::*;
        let on = opts(true, false, true);
        let sa = acl("alice", vec![grant(Some("bob"), &[(Select, false)])]);
        let ta = acl("alice", vec![]);
        let s = loaded(ObjectDef::Table(table("src", vec![])), Some(sa.clone()));
        let t = loaded(ObjectDef::Table(table("tgt", vec![])), Some(ta));
        assert_eq!(st(&prep(Some(&s), Some(&t), on)), (Different, None));
        // Aynı grant'ler -> Same; kolon düzeyi fark yalnız not ekler.
        let mut tb = sa.clone();
        tb.columns = vec![pgt_ddl::ColumnGrant {
            column: "a".into(),
            grantor: "alice".into(),
            grantee: Some("bob".into()),
            privilege: Select,
            grant_option: false,
        }];
        let t = loaded(ObjectDef::Table(table("tgt", vec![])), Some(tb));
        let p = prep(Some(&s), Some(&t), on);
        assert_eq!(st(&p), (Same, None));
        assert_eq!(
            p.notes,
            ["Kolon düzeyi ve başka verenlerin yetkileri karşılaştırılmaz."]
        );
    }

    #[test]
    fn kaynak_sema_adi_notu() {
        use ItemStatus::*;
        let s = loaded(ObjectDef::View(view("src", "SELECT * FROM src.t")), None);
        let t = loaded(ObjectDef::View(view("tgt", "SELECT * FROM tgt.t")), None);
        let p = prep(Some(&s), Some(&t), DEF);
        assert_eq!(p.status, Different);
        assert!(
            p.notes
                .contains(&"Gövdede kaynak şema adı açıkça geçiyor; gözden geçirin.".to_string())
        );
        let p = prep(Some(&s), None, DEF);
        assert!(p.notes.iter().any(|n| n.starts_with("Gövdede kaynak şema")));
        // Şema adları aynıysa not yok.
        let c = NormCtx {
            source_schema: "src",
            target_schema: "src",
            options: DEF,
        };
        let p = prepare("k".into(), Some(&s), None, &c);
        assert!(p.notes.is_empty());
    }

    #[test]
    fn prepare_hizalanmis_kaynak_dondurur() {
        let s = lt(table("src", vec![col("b", Some(9))]));
        let t = lt(table("tgt", vec![col("b", Some(3))]));
        let p = prep(Some(&s), Some(&t), DEF);
        assert_eq!(
            unwrap_table(p.source.as_ref().unwrap()).columns[0].attnum,
            Some(3)
        );
        assert_eq!(unwrap_table(p.source.as_ref().unwrap()).schema, "tgt");
    }
}
