//! Öğe başına aşamalı DDL üretimi: `Prepared` durumundan `pgt-ddl` üreticileriyle ifadeleri kurar ve
//! betik sıralaması için aşamalara (spec §2.6) ayırır. Veritabanına dokunmaz.

use pgt_ddl::build::routine::needs_recreate;
use pgt_ddl::build::{self, BuildCtx, DropOptions, table_alter};
use pgt_ddl::{
    AclDef, AclGrant, ConstraintDef, ConstraintKind, FieldError, ObjectDef, ObjectKind, Plan,
    Privilege, PrivilegeGrant, Risk, Statement, TableDef, acl_supported, privileges_for,
    sort_grants,
};

use crate::model::{ItemProblem, ItemStatus};
use crate::normalize::{Prepared, canonical_grants};

/// Betik aşaması; sıralama da bu sırayla (spec §2.6 adım 3 ve 4.1-4.10).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Phase {
    DropTrigger,
    DropIndex,
    DropView,
    DropRoutine,
    DropTable,
    DropSequence,
    DropDomain,
    DropType,
    Types,
    Sequences,
    Routines,
    Tables,
    OwnedBy,
    ForeignKeys,
    Views,
    Indexes,
    Triggers,
    Privileges,
}

/// Bir öğenin üretilen ifadeleri (yalnız dolu aşamalar, aşama sırasıyla).
#[derive(Debug, Clone)]
pub struct Generated {
    pub parts: Vec<(Phase, Vec<Statement>)>,
    pub warnings: Vec<String>,
    pub risk: Risk,
    pub notes: Vec<String>,
    /// Öğe hedefteki tabloyu siler.
    pub drops_table: bool,
    /// Tablo değişikliğinin düşürdüğü hedef kolonları.
    pub dropped_columns: Vec<String>,
    /// Yalnız hedefte dizi: `OWNED BY` (tablo, kolon).
    pub sequence_owner: Option<(String, String)>,
}

impl Generated {
    fn empty() -> Self {
        Generated {
            parts: vec![],
            warnings: vec![],
            risk: Risk::Safe,
            notes: vec![],
            drops_table: false,
            dropped_columns: vec![],
            sequence_owner: None,
        }
    }
}

/// Üretilemeyen öğe: `BuildFailed` (üretici iletileri) ya da `Manual`.
#[derive(Debug, Clone)]
pub struct GenFailure {
    pub problem: ItemProblem,
    pub messages: Vec<String>,
}

const MANUAL_MSG: &str = "Fark üreticiyle ifade edilemiyor.";
const REDACTED_MSG: &str = "Gizli değer içeren ifade betiğe alınamaz.";
/// Üreticilerin "yetkiler yeniden verilmeli" uyarıları (görünüm, rutin, MV yeniden kurma).
const REGRANT_WARNINGS: [&str; 3] = [
    build::view::RECREATE_GRANT_WARNING,
    build::routine::RECREATE_GRANT_WARNING,
    build::matview::GRANT_WARNING,
];
const KIND_CHANGED_NOTE: &str = "Tür değişti: hedefteki nesne silinip yeniden oluşturulur";
const RECREATE_NOTE: &str = "Yeniden kurulacak (DROP + CREATE)";

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

fn manual(msg: &str) -> GenFailure {
    GenFailure {
        problem: ItemProblem::Manual,
        messages: vec![msg.to_string()],
    }
}

fn create_phase(kind: ObjectKind) -> Option<Phase> {
    Some(match kind {
        ObjectKind::Type | ObjectKind::Domain => Phase::Types,
        ObjectKind::Sequence => Phase::Sequences,
        ObjectKind::Function | ObjectKind::Procedure => Phase::Routines,
        ObjectKind::Table => Phase::Tables,
        ObjectKind::View | ObjectKind::MaterializedView => Phase::Views,
        ObjectKind::Index => Phase::Indexes,
        ObjectKind::Trigger => Phase::Triggers,
        _ => return None,
    })
}

fn drop_phase(kind: ObjectKind) -> Option<Phase> {
    Some(match kind {
        ObjectKind::Trigger => Phase::DropTrigger,
        ObjectKind::Index => Phase::DropIndex,
        ObjectKind::View | ObjectKind::MaterializedView => Phase::DropView,
        ObjectKind::Function | ObjectKind::Procedure => Phase::DropRoutine,
        ObjectKind::Table => Phase::DropTable,
        ObjectKind::Sequence => Phase::DropSequence,
        ObjectKind::Domain => Phase::DropDomain,
        ObjectKind::Type => Phase::DropType,
        _ => return None,
    })
}

/// Yeni oluşturulan nesnenin varsayılan (`acldefault`) yetkileri: sahip (grant option yok); işlev,
/// prosedür, tip ve domain için ayrıca `PUBLIC`.
pub fn default_grants(kind: ObjectKind, owner: &str, version_num: u32) -> Vec<AclGrant> {
    let row = |grantee: Option<&str>, ps: &[Privilege]| AclGrant {
        grantee: grantee.map(str::to_string),
        privileges: ps
            .iter()
            .map(|&privilege| PrivilegeGrant {
                privilege,
                grant_option: false,
            })
            .collect(),
    };
    let mut g = vec![row(Some(owner), privileges_for(kind, version_num))];
    match kind {
        ObjectKind::Function | ObjectKind::Procedure => g.push(row(None, &[Privilege::Execute])),
        ObjectKind::Type | ObjectKind::Domain => g.push(row(None, &[Privilege::Usage])),
        ObjectKind::Database => g.push(row(None, &[Privilege::Connect, Privilege::Temporary])),
        _ => {}
    }
    sort_grants(&mut g, privileges_for(kind, 170000));
    g
}

/// FK kısıtlarını `base` tabloya ekleyen plan: eski = `base` (kolon `attnum = i+1`, kısıt
/// `oid = i+1`), yeni = eski + `fks` (`oid = None`). Çıktıda yalnız `ADD CONSTRAINT` ifadeleri olur.
pub fn fk_add_plan(
    base: &TableDef,
    fks: &[ConstraintDef],
    ctx: &BuildCtx,
) -> Result<Plan, Vec<FieldError>> {
    let mut old = base.clone();
    for (i, c) in old.columns.iter_mut().enumerate() {
        c.attnum = Some(i as i16 + 1);
    }
    for (i, c) in old.constraints.iter_mut().enumerate() {
        c.oid = Some(i as u32 + 1);
    }
    let mut new = old.clone();
    new.constraints.extend(fks.iter().cloned().map(|mut c| {
        c.oid = None;
        c
    }));
    table_alter::alter(&old, &new, ctx)
}

fn is_fk(c: &ConstraintDef) -> bool {
    matches!(c.kind, ConstraintKind::ForeignKey { .. })
}

/// Tablo -> (FK'sız tablo, FK kısıtları).
fn split_fks(t: &TableDef) -> (TableDef, Vec<ConstraintDef>) {
    let mut no_fk = t.clone();
    let (fks, rest): (Vec<_>, Vec<_>) = t.constraints.iter().cloned().partition(is_fk);
    no_fk.constraints = rest;
    (no_fk, fks)
}

/// Üretim biriktiricisi: planları aşamalara koyar, hataları toplar.
struct Acc {
    parts: Vec<(Phase, Vec<Statement>)>,
    warnings: Vec<String>,
    notes: Vec<String>,
    floor: Risk,
    errors: Vec<FieldError>,
    drops_table: bool,
    dropped_columns: Vec<String>,
    sequence_owner: Option<(String, String)>,
}

impl Acc {
    fn new() -> Self {
        Acc {
            parts: vec![],
            warnings: vec![],
            notes: vec![],
            floor: Risk::Safe,
            errors: vec![],
            drops_table: false,
            dropped_columns: vec![],
            sequence_owner: None,
        }
    }

    fn plan(&mut self, phase: Phase, r: Result<Plan, Vec<FieldError>>) {
        match r {
            Ok(p) => {
                for w in &p.warnings {
                    push_unique(&mut self.warnings, w);
                }
                if !p.statements.is_empty() {
                    self.parts.push((phase, p.statements));
                }
            }
            Err(e) => self.errors.extend(e),
        }
    }

    /// Kaynak nesneyi oluşturma (yalnız kaynakta ya da tür değişimi).
    fn create_source(&mut self, def: &ObjectDef, ctx: &BuildCtx) -> Result<(), GenFailure> {
        let phase = create_phase(def.kind()).ok_or_else(|| manual(MANUAL_MSG))?;
        match def {
            ObjectDef::Table(t) => {
                let (no_fk, fks) = split_fks(t);
                self.plan(phase, build::create(&ObjectDef::Table(no_fk.clone()), ctx));
                if !fks.is_empty() {
                    self.plan(Phase::ForeignKeys, fk_add_plan(&no_fk, &fks, ctx));
                }
            }
            ObjectDef::Sequence(s) if s.owned_by.is_some() => {
                let mut bare = s.clone();
                bare.owned_by = None;
                let bare = ObjectDef::Sequence(bare);
                self.plan(phase, build::create(&bare, ctx));
                self.plan(Phase::OwnedBy, build::alter(&bare, def, ctx));
            }
            _ => self.plan(phase, build::create(def, ctx)),
        }
        Ok(())
    }

    /// Hedefteki nesneyi silme (`CASCADE` yok).
    fn drop_target(&mut self, def: &ObjectDef, ctx: &BuildCtx) -> Result<(), GenFailure> {
        let phase = drop_phase(def.kind()).ok_or_else(|| manual(MANUAL_MSG))?;
        let o = DropOptions {
            cascade: false,
            force: false,
        };
        self.plan(phase, build::drop_with(def, o, ctx));
        match def {
            ObjectDef::Table(_) => self.drops_table = true,
            ObjectDef::Sequence(s) => {
                self.sequence_owner = s
                    .owned_by
                    .as_ref()
                    .map(|o| (o.table.clone(), o.column.clone()));
            }
            _ => {}
        }
        Ok(())
    }

    /// Aynı türde fark; nesne DROP + CREATE ile yeniden kuruluyorsa `true` döner.
    fn alter_same_kind(
        &mut self,
        s: &ObjectDef,
        t: &ObjectDef,
        ctx: &BuildCtx,
    ) -> Result<bool, GenFailure> {
        match (s, t) {
            (ObjectDef::Table(d), ObjectDef::Table(old)) => {
                self.dropped_columns = old
                    .columns
                    .iter()
                    .filter(|c| !d.columns.iter().any(|n| n.name == c.name))
                    .map(|c| c.name.clone())
                    .collect();
                // Yeni ya da değişen FK'lar sona bırakılır; değişenler `alter` içinde önce düşer.
                let fk_changed = |c: &ConstraintDef| {
                    let prev = c
                        .oid
                        .and_then(|o| old.constraints.iter().find(|x| x.oid == Some(o)));
                    match prev {
                        None => true,
                        Some(x) => {
                            x.kind != c.kind
                                || x.deferrable != c.deferrable
                                || x.initially_deferred != c.initially_deferred
                                || x.not_valid != c.not_valid
                        }
                    }
                };
                let mut no_fk = d.clone();
                no_fk.constraints.clear();
                let mut added = vec![];
                for c in &d.constraints {
                    if is_fk(c) && fk_changed(c) {
                        added.push(c.clone());
                    } else {
                        no_fk.constraints.push(c.clone());
                    }
                }
                self.plan(
                    Phase::Tables,
                    build::alter(t, &ObjectDef::Table(no_fk.clone()), ctx),
                );
                if !added.is_empty() {
                    self.plan(Phase::ForeignKeys, fk_add_plan(&no_fk, &added, ctx));
                }
                Ok(false)
            }
            (ObjectDef::Sequence(d), ObjectDef::Sequence(old)) => {
                let mut mid = d.clone();
                if mid.owned_by != old.owned_by {
                    mid.owned_by = None;
                }
                let differs = mid.owned_by != d.owned_by;
                let mid = ObjectDef::Sequence(mid);
                self.plan(Phase::Sequences, build::alter(t, &mid, ctx));
                if differs {
                    self.plan(Phase::OwnedBy, build::alter(&mid, s, ctx));
                }
                Ok(false)
            }
            (ObjectDef::View(d), ObjectDef::View(old)) => {
                let mut d = d.clone();
                d.recreate = d.query != old.query;
                let rebuilt = d.recreate;
                if rebuilt {
                    push_unique(&mut self.notes, RECREATE_NOTE);
                }
                self.plan(Phase::Views, build::alter(t, &ObjectDef::View(d), ctx));
                Ok(rebuilt)
            }
            (ObjectDef::MaterializedView(d), ObjectDef::MaterializedView(old)) => {
                self.plan(Phase::Views, build::alter(t, s, ctx));
                Ok(d.query != old.query)
            }
            (ObjectDef::Function(d), ObjectDef::Function(old))
            | (ObjectDef::Procedure(d), ObjectDef::Procedure(old)) => {
                let mut d = d.clone();
                d.recreate = !needs_recreate(s.kind(), old, &d).is_empty();
                let rebuilt = d.recreate;
                if rebuilt {
                    push_unique(&mut self.notes, RECREATE_NOTE);
                    self.floor = self.floor.max(Risk::Destructive);
                }
                let new = if matches!(s, ObjectDef::Function(_)) {
                    ObjectDef::Function(d)
                } else {
                    ObjectDef::Procedure(d)
                };
                self.plan(Phase::Routines, build::alter(t, &new, ctx));
                Ok(rebuilt)
            }
            _ => {
                let phase = create_phase(s.kind()).ok_or_else(|| manual(MANUAL_MSG))?;
                self.plan(phase, build::alter(t, s, ctx));
                Ok(false)
            }
        }
    }

    fn finish(mut self) -> Result<Generated, GenFailure> {
        if !self.errors.is_empty() {
            let mut messages: Vec<String> = vec![];
            for e in &self.errors {
                push_unique(&mut messages, &e.message);
            }
            return Err(GenFailure {
                problem: ItemProblem::BuildFailed,
                messages,
            });
        }
        if self.parts.iter().flat_map(|(_, v)| v).any(|s| s.redacted) {
            return Err(GenFailure {
                problem: ItemProblem::BuildFailed,
                messages: vec![REDACTED_MSG.to_string()],
            });
        }
        if self.parts.is_empty() {
            return Err(manual(MANUAL_MSG));
        }
        self.parts.sort_by_key(|(ph, _)| *ph);
        let risk = self
            .parts
            .iter()
            .flat_map(|(_, v)| v)
            .map(|s| s.risk)
            .max()
            .unwrap_or(Risk::Safe)
            .max(self.floor);
        Ok(Generated {
            parts: self.parts,
            warnings: self.warnings,
            risk,
            notes: self.notes,
            drops_table: self.drops_table,
            dropped_columns: self.dropped_columns,
            sequence_owner: self.sequence_owner,
        })
    }
}

/// Kaynak ACL'nin "yeni oluşturulmuş nesne" başlangıcı: varsayılan yetkiler.
fn base_acl(a: &AclDef, ctx: &BuildCtx, viewer: &str) -> AclDef {
    AclDef {
        kind: a.kind,
        schema: a.schema.clone(),
        name: a.name.clone(),
        signature: a.signature.clone(),
        owner: a.owner.clone(),
        viewer: viewer.to_string(),
        explicit: false,
        grants: default_grants(a.kind, &a.owner, ctx.version_num),
        foreign: vec![],
        columns: vec![],
        editable: true,
        unsupported: vec![],
        available: privileges_for(a.kind, ctx.version_num).to_vec(),
    }
}

/// Bir öğenin ifadelerini üretir. `Same` boş sonuç verir; `problem` doluysa üretilmez.
/// `target_viewer`: hedefe bağlı rol (yeni nesnenin başlangıç ACL'sinde `viewer`).
pub fn generate(
    p: &Prepared,
    ctx: &BuildCtx,
    target_viewer: &str,
) -> Result<Generated, GenFailure> {
    if let Some(problem) = p.problem {
        return Err(GenFailure {
            problem,
            messages: p.errors.iter().chain(&p.notes).cloned().collect(),
        });
    }
    if p.status == ItemStatus::Same {
        return Ok(Generated::empty());
    }
    let mut a = Acc::new();
    // Nesne baştan kuruluyorsa (oluşturma, tür değişimi, DROP + CREATE) yetkiler varsayılandan başlar.
    let recreated = match (p.status, p.source.as_ref(), p.target.as_ref()) {
        (ItemStatus::OnlySource, Some(s), _) => {
            a.create_source(s, ctx)?;
            true
        }
        (ItemStatus::OnlyTarget, _, Some(t)) => {
            a.drop_target(t, ctx)?;
            false
        }
        (ItemStatus::Different, Some(s), Some(t)) if p.kind_changed => {
            a.drop_target(t, ctx)?;
            a.create_source(s, ctx)?;
            push_unique(&mut a.notes, KIND_CHANGED_NOTE);
            true
        }
        (ItemStatus::Different, Some(s), Some(t)) => a.alter_same_kind(s, t, ctx)?,
        _ => return Err(manual(MANUAL_MSG)),
    };
    // Yetkiler: sahiplik seçeneği açıksa kaynak ACL yüklüdür.
    if p.status != ItemStatus::OnlyTarget
        && let Some(sa) = p.source_acl.as_ref().filter(|x| acl_supported(x.kind))
    {
        let old = match (&p.target_acl, recreated) {
            (_, true) => Some(base_acl(sa, ctx, target_viewer)),
            (Some(ta), false) => {
                // Hedef ACL'nin sahibi kaynak sahibine çevrilir (OWNER TO yetkileri taşır).
                let mut o = ta.clone();
                o.grants = canonical_grants(ta, Some((ta.owner.as_str(), sa.owner.as_str())));
                o.owner = sa.owner.clone();
                Some(o)
            }
            (None, false) => None,
        };
        if let Some(old) = old {
            let mut new = old.clone();
            new.grants = canonical_grants(sa, None);
            a.plan(
                Phase::Privileges,
                build::acl::privileges(ctx, &old, &new, false),
            );
            // Yeniden kurulan nesnenin yetkileri Privileges aşamasında zaten veriliyor.
            if recreated && a.parts.iter().any(|(ph, _)| *ph == Phase::Privileges) {
                a.warnings
                    .retain(|w| !REGRANT_WARNINGS.contains(&w.as_str()));
            }
        }
    }
    a.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgt_ddl::{
        ArgMode, ColumnDef, ColumnRef, EnumValue, FkAction, FkMatch, FunctionArg, FunctionDef,
        FunctionReturn, MatViewDef, Parallel, Persistence, SeqType, SequenceDef, TypeBody, TypeDef,
        ViewColumn, ViewDef, Volatility,
    };

    fn ctx(v: u32) -> BuildCtx {
        BuildCtx {
            version_num: v,
            concurrently: false,
        }
    }

    fn prep(status: ItemStatus, s: Option<ObjectDef>, t: Option<ObjectDef>) -> Prepared {
        Prepared {
            key: "k".into(),
            status,
            problem: None,
            notes: vec![],
            errors: vec![],
            kind_changed: false,
            source: s,
            target: t,
            source_acl: None,
            target_acl: None,
        }
    }

    fn render(g: &Generated) -> String {
        let mut out = String::new();
        for (ph, sts) in &g.parts {
            for s in sts {
                out.push_str(&format!(
                    "{ph:?} [{:?}{}] {}\n",
                    s.risk,
                    if s.transactional { "" } else { " non-tx" },
                    s.sql
                ));
            }
        }
        for w in &g.warnings {
            out.push_str(&format!("uyarı: {w}\n"));
        }
        for n in &g.notes {
            out.push_str(&format!("not: {n}\n"));
        }
        out.push_str(&format!("risk: {:?}\n", g.risk));
        out
    }

    fn sql_of(g: &Generated, ph: Phase) -> Vec<String> {
        g.parts
            .iter()
            .filter(|(p, _)| *p == ph)
            .flat_map(|(_, v)| v.iter().map(|s| s.sql.clone()))
            .collect()
    }

    fn gen_ok(p: &Prepared, v: u32) -> Generated {
        generate(p, &ctx(v), "app").expect("üretilmeli")
    }

    fn col(name: &str, ty: &str, attnum: Option<i16>) -> ColumnDef {
        ColumnDef {
            attnum,
            name: name.into(),
            data_type: ty.into(),
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            collation: None,
            comment: None,
            using: None,
        }
    }

    fn fk(name: &str, oid: Option<u32>, ref_table: &str) -> ConstraintDef {
        ConstraintDef {
            oid,
            name: Some(name.into()),
            kind: ConstraintKind::ForeignKey {
                columns: vec!["b_id".into()],
                ref_schema: "s".into(),
                ref_table: ref_table.into(),
                ref_columns: vec!["id".into()],
                match_type: FkMatch::Simple,
                on_update: FkAction::NoAction,
                on_delete: FkAction::NoAction,
            },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        }
    }

    fn pk(oid: Option<u32>) -> ConstraintDef {
        ConstraintDef {
            oid,
            name: Some("t_pkey".into()),
            kind: ConstraintKind::PrimaryKey {
                columns: vec!["id".into()],
                include: vec![],
            },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        }
    }

    fn table(name: &str, cols: Vec<ColumnDef>, cons: Vec<ConstraintDef>) -> TableDef {
        TableDef {
            schema: "s".into(),
            name: name.into(),
            owner: None,
            tablespace: None,
            persistence: Persistence::Permanent,
            columns: cols,
            constraints: cons,
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        }
    }

    fn t(name: &str, cols: Vec<ColumnDef>, cons: Vec<ConstraintDef>) -> ObjectDef {
        ObjectDef::Table(table(name, cols, cons))
    }

    fn seq(owned: Option<&str>) -> SequenceDef {
        SequenceDef {
            schema: "s".into(),
            name: "q".into(),
            owner: None,
            data_type: SeqType::Bigint,
            increment: 1,
            min_value: None,
            max_value: None,
            start: 1,
            cache: 1,
            cycle: false,
            owned_by: owned.map(|c| ColumnRef {
                schema: "s".into(),
                table: "t".into(),
                column: c.into(),
            }),
            comment: None,
        }
    }

    fn view(q: &str) -> ViewDef {
        ViewDef {
            schema: "s".into(),
            name: "v".into(),
            owner: None,
            query: q.into(),
            check_option: None,
            security_barrier: false,
            security_invoker: false,
            comment: None,
            recreate: false,
            columns: vec![],
            unsupported: vec![],
        }
    }

    fn func(ret: &str, body: &str) -> FunctionDef {
        FunctionDef {
            schema: "s".into(),
            name: "f".into(),
            owner: None,
            comment: None,
            language: "sql".into(),
            args: vec![FunctionArg {
                mode: ArgMode::In,
                name: Some("a".into()),
                type_name: "integer".into(),
                default: None,
            }],
            returns: Some(FunctionReturn::Type {
                type_name: ret.into(),
                setof: false,
            }),
            body: body.into(),
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
        }
    }

    fn enum_type(vals: &[(&str, bool)]) -> ObjectDef {
        ObjectDef::Type(TypeDef {
            schema: "s".into(),
            name: "mood".into(),
            owner: None,
            comment: None,
            body: TypeBody::Enum {
                values: vals
                    .iter()
                    .map(|(l, existing)| EnumValue {
                        label: (*l).into(),
                        original: existing.then(|| (*l).into()),
                    })
                    .collect(),
            },
            editable: true,
            unsupported: vec![],
        })
    }

    fn id_cols() -> Vec<ColumnDef> {
        let mut c = vec![
            col("id", "integer", Some(1)),
            col("b_id", "integer", Some(2)),
        ];
        c[0].not_null = true;
        c
    }

    fn new_cols() -> Vec<ColumnDef> {
        let mut c = vec![col("id", "integer", None), col("b_id", "integer", None)];
        c[0].not_null = true;
        c
    }

    #[test]
    fn yeni_iki_tablo_fk_sona_kalir() {
        let b = prep(
            ItemStatus::OnlySource,
            Some(t("b", new_cols(), vec![pk(None)])),
            None,
        );
        let a = prep(
            ItemStatus::OnlySource,
            Some(t(
                "a",
                new_cols(),
                vec![pk(None), fk("a_b_fkey", None, "b")],
            )),
            None,
        );
        let (ga, gb) = (gen_ok(&a, 160000), gen_ok(&b, 160000));
        assert_eq!(sql_of(&ga, Phase::Tables).len(), 1);
        assert_eq!(sql_of(&gb, Phase::Tables).len(), 1);
        assert!(!sql_of(&ga, Phase::Tables)[0].contains("FOREIGN KEY"));
        let fks = sql_of(&ga, Phase::ForeignKeys);
        assert_eq!(fks.len(), 1);
        assert!(fks[0].starts_with("ALTER TABLE") && fks[0].contains("ADD CONSTRAINT"));
        insta::assert_snapshot!("yeni_tablo_fk", render(&ga));
    }

    #[test]
    fn degisen_tabloya_fk_eklenir() {
        let target = t("a", id_cols(), vec![pk(Some(10))]);
        let source = t(
            "a",
            id_cols(),
            vec![pk(Some(10)), fk("a_b_fkey", None, "b")],
        );
        let g = gen_ok(
            &prep(ItemStatus::Different, Some(source), Some(target)),
            160000,
        );
        assert!(g.parts.iter().all(|(p, _)| *p == Phase::ForeignKeys));
        insta::assert_snapshot!("tabloya_fk_eklenir", render(&g));
    }

    #[test]
    fn fk_hedef_tablosu_degisir() {
        let target = t(
            "a",
            id_cols(),
            vec![pk(Some(10)), fk("a_b_fkey", Some(11), "b")],
        );
        let source = t(
            "a",
            id_cols(),
            vec![pk(Some(10)), fk("a_b_fkey", Some(11), "c")],
        );
        let g = gen_ok(
            &prep(ItemStatus::Different, Some(source), Some(target)),
            160000,
        );
        assert!(sql_of(&g, Phase::Tables)[0].contains("DROP CONSTRAINT"));
        assert!(sql_of(&g, Phase::ForeignKeys)[0].contains("ADD CONSTRAINT"));
        insta::assert_snapshot!("fk_hedefi_degisir", render(&g));
    }

    #[test]
    fn kolon_yeniden_adlandirma_drop_add() {
        let target = t(
            "a",
            vec![col("id", "integer", Some(1)), col("eski", "text", Some(2))],
            vec![],
        );
        let source = t(
            "a",
            vec![col("id", "integer", Some(1)), col("yeni", "text", None)],
            vec![],
        );
        let g = gen_ok(
            &prep(ItemStatus::Different, Some(source), Some(target)),
            160000,
        );
        assert_eq!(g.risk, Risk::DataLoss);
        assert_eq!(g.dropped_columns, ["eski"]);
        insta::assert_snapshot!("kolon_adi_degisir", render(&g));
    }

    #[test]
    fn kolon_tipi_degisimi_uretici_riskiyle() {
        let target = t("a", vec![col("id", "integer", Some(1))], vec![]);
        let source = t("a", vec![col("id", "bigint", Some(1))], vec![]);
        let g = gen_ok(
            &prep(ItemStatus::Different, Some(source), Some(target)),
            160000,
        );
        // Risk üreticinin kararıdır (`ALTER COLUMN … TYPE` = DataLoss); öğe riski ifadelerin en yükseği.
        assert_eq!(g.risk, Risk::DataLoss);
        assert!(sql_of(&g, Phase::Tables)[0].contains("ALTER COLUMN"));
        assert!(sql_of(&g, Phase::Tables)[0].contains(" TYPE "));
        assert!(g.dropped_columns.is_empty());
    }

    fn seq_gen(s: Option<&str>, t_: Option<&str>) -> Generated {
        gen_ok(
            &prep(
                ItemStatus::Different,
                Some(ObjectDef::Sequence(seq(s))),
                Some(ObjectDef::Sequence(seq(t_))),
            ),
            160000,
        )
    }

    #[test]
    fn sequence_owned_by_yok_var() {
        let g = seq_gen(Some("id"), None);
        assert!(sql_of(&g, Phase::Sequences).is_empty());
        insta::assert_snapshot!("seq_yok_var", render(&g));
    }

    #[test]
    fn sequence_owned_by_baska_kolon() {
        let g = seq_gen(Some("b_id"), Some("id"));
        assert!(sql_of(&g, Phase::Sequences)[0].contains("OWNED BY NONE"));
        assert_eq!(sql_of(&g, Phase::OwnedBy).len(), 1);
        insta::assert_snapshot!("seq_baska_kolon", render(&g));
    }

    #[test]
    fn sequence_owned_by_var_yok() {
        let g = seq_gen(None, Some("id"));
        assert!(sql_of(&g, Phase::Sequences)[0].contains("OWNED BY NONE"));
        assert!(sql_of(&g, Phase::OwnedBy).is_empty());
    }

    #[test]
    fn yeni_sequence_bag_sonra() {
        let g = gen_ok(
            &prep(
                ItemStatus::OnlySource,
                Some(ObjectDef::Sequence(seq(Some("id")))),
                None,
            ),
            160000,
        );
        assert!(sql_of(&g, Phase::Sequences)[0].ends_with("OWNED BY NONE"));
        assert!(sql_of(&g, Phase::OwnedBy)[0].contains("OWNED BY s.t.id"));
    }

    #[test]
    fn gorunum_sorgu_degisimi_yeniden_kurulur() {
        let g = gen_ok(
            &prep(
                ItemStatus::Different,
                Some(ObjectDef::View(view("SELECT 2 AS a"))),
                Some(ObjectDef::View(view("SELECT 1 AS a"))),
            ),
            160000,
        );
        assert!(g.notes.iter().any(|n| n.contains("Yeniden kurulacak")));
        insta::assert_snapshot!("view_sorgu", render(&g));
    }

    #[test]
    fn gorunum_sorgu_ayniysa_yeniden_kurulmaz() {
        let mut s = view("SELECT 1 AS a");
        s.comment = Some("c".into());
        let g = gen_ok(
            &prep(
                ItemStatus::Different,
                Some(ObjectDef::View(s)),
                Some(ObjectDef::View(view("SELECT 1 AS a"))),
            ),
            160000,
        );
        assert!(g.notes.is_empty());
        assert!(!sql_of(&g, Phase::Views)[0].contains("DROP"));
    }

    #[test]
    fn fonksiyon_donus_tipi_yeniden_kurulur() {
        let g = gen_ok(
            &prep(
                ItemStatus::Different,
                Some(ObjectDef::Function(func("text", "SELECT 1"))),
                Some(ObjectDef::Function(func("integer", "SELECT 1"))),
            ),
            160000,
        );
        assert!(g.risk >= Risk::Destructive);
        assert!(g.notes.iter().any(|n| n.contains("Yeniden kurulacak")));
        let all = sql_of(&g, Phase::Routines);
        assert!(all[0].starts_with("DROP FUNCTION"));
        insta::assert_snapshot!("fonksiyon_donus", render(&g));
    }

    #[test]
    fn fonksiyon_govde_degisimi_or_replace() {
        let g = gen_ok(
            &prep(
                ItemStatus::Different,
                Some(ObjectDef::Function(func("integer", "SELECT 2"))),
                Some(ObjectDef::Function(func("integer", "SELECT 1"))),
            ),
            160000,
        );
        assert!(g.notes.is_empty());
        let all = sql_of(&g, Phase::Routines);
        assert_eq!(all.len(), 1);
        assert!(all[0].starts_with("CREATE OR REPLACE FUNCTION"));
    }

    fn matview(q: &str) -> ObjectDef {
        ObjectDef::MaterializedView(MatViewDef {
            schema: "s".into(),
            name: "mv".into(),
            owner: None,
            query: q.into(),
            tablespace: None,
            fillfactor: None,
            with_data: true,
            comment: None,
            refresh: None,
            columns: vec![ViewColumn {
                name: "a".into(),
                data_type: "integer".into(),
            }],
            unsupported: vec!["INDEX mv_i".into()],
        })
    }

    #[test]
    fn mv_sorgu_degisimi_dataloss_ve_indeks_uyarisi() {
        let g = gen_ok(
            &prep(
                ItemStatus::Different,
                Some(matview("SELECT 2")),
                Some(matview("SELECT 1")),
            ),
            160000,
        );
        assert_eq!(g.risk, Risk::DataLoss);
        assert!(g.warnings.iter().any(|w| w.contains("mv_i")));
    }

    #[test]
    fn tur_degisimi_tablo_gorunum() {
        let mut p = prep(
            ItemStatus::Different,
            Some(ObjectDef::View(view("SELECT 1 AS a"))),
            Some(t("v", vec![col("a", "integer", Some(1))], vec![])),
        );
        p.kind_changed = true;
        let g = gen_ok(&p, 160000);
        let phases: Vec<Phase> = g.parts.iter().map(|(p, _)| *p).collect();
        assert_eq!(phases, [Phase::DropTable, Phase::Views]);
        assert!(g.drops_table);
        assert!(g.notes.iter().any(|n| n.starts_with("Tür değişti")));
        insta::assert_snapshot!("tur_degisimi", render(&g));
    }

    #[test]
    fn enum_deger_ekleme_pg11_transaction_disi() {
        let p = prep(
            ItemStatus::Different,
            Some(enum_type(&[("a", true), ("b", false)])),
            Some(enum_type(&[("a", true)])),
        );
        let g11 = gen_ok(&p, 110000);
        let st = &g11.parts[0].1[0];
        assert!(st.sql.contains("ADD VALUE"));
        assert!(!st.transactional);
        assert_eq!(g11.parts[0].0, Phase::Types);
        let g16 = gen_ok(&p, 160000);
        assert!(g16.parts[0].1[0].transactional);
    }

    #[test]
    fn enum_deger_silme_build_failed() {
        let p = prep(
            ItemStatus::Different,
            Some(enum_type(&[("a", true)])),
            Some(enum_type(&[("a", true), ("b", true)])),
        );
        let e = generate(&p, &ctx(160000), "app").unwrap_err();
        assert_eq!(e.problem, ItemProblem::BuildFailed);
        assert!(!e.messages.is_empty());
        let direct = build::alter(
            p.target.as_ref().unwrap(),
            p.source.as_ref().unwrap(),
            &ctx(160000),
        )
        .unwrap_err();
        assert_eq!(e.messages[0], direct[0].message);
    }

    #[test]
    fn nulls_not_distinct_pg13_build_failed() {
        let uq = ConstraintDef {
            oid: None,
            name: Some("t_uq".into()),
            kind: ConstraintKind::Unique {
                columns: vec!["id".into()],
                include: vec![],
                nulls_not_distinct: true,
            },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        };
        let p = prep(
            ItemStatus::OnlySource,
            Some(t("t", new_cols(), vec![uq])),
            None,
        );
        let e = generate(&p, &ctx(130000), "app").unwrap_err();
        assert_eq!(e.problem, ItemProblem::BuildFailed);
        assert!(e.messages.iter().any(|m| m.contains("15")));
        assert!(generate(&p, &ctx(160000), "app").is_ok());
    }

    #[test]
    fn yalniz_hedefte_tablo_drop() {
        let g = gen_ok(
            &prep(
                ItemStatus::OnlyTarget,
                None,
                Some(t("a", id_cols(), vec![])),
            ),
            160000,
        );
        assert!(g.drops_table);
        assert_eq!(g.parts[0].0, Phase::DropTable);
        assert!(!g.parts[0].1[0].sql.contains("CASCADE"));
    }

    #[test]
    fn yalniz_hedefte_bagli_dizi_sahibi() {
        let g = gen_ok(
            &prep(
                ItemStatus::OnlyTarget,
                None,
                Some(ObjectDef::Sequence(seq(Some("id")))),
            ),
            160000,
        );
        assert_eq!(g.sequence_owner, Some(("t".to_string(), "id".to_string())));
        assert_eq!(g.parts[0].0, Phase::DropSequence);
    }

    #[test]
    fn varsayilan_yetkiler() {
        let names = |g: &[AclGrant]| -> Vec<(Option<String>, Vec<Privilege>)> {
            g.iter()
                .map(|r| {
                    (
                        r.grantee.clone(),
                        r.privileges.iter().map(|p| p.privilege).collect(),
                    )
                })
                .collect()
        };
        let tbl = default_grants(ObjectKind::Table, "o", 160000);
        assert_eq!(
            names(&tbl),
            [(
                Some("o".into()),
                privileges_for(ObjectKind::Table, 160000).to_vec()
            )]
        );
        let f = default_grants(ObjectKind::Function, "o", 160000);
        assert_eq!(
            names(&f),
            [
                (None, vec![Privilege::Execute]),
                (Some("o".into()), vec![Privilege::Execute])
            ]
        );
        for k in [ObjectKind::Type, ObjectKind::Domain] {
            let g = default_grants(k, "o", 160000);
            assert_eq!(
                names(&g),
                [
                    (None, vec![Privilege::Usage]),
                    (Some("o".into()), vec![Privilege::Usage])
                ]
            );
        }
        let t17 = default_grants(ObjectKind::Table, "o", 170000);
        assert!(
            t17[0]
                .privileges
                .iter()
                .any(|p| p.privilege == Privilege::Maintain)
        );
        let t16 = default_grants(ObjectKind::Table, "o", 160000);
        assert!(
            !t16[0]
                .privileges
                .iter()
                .any(|p| p.privilege == Privilege::Maintain)
        );
    }

    fn acl(owner: &str, grants: Vec<AclGrant>) -> AclDef {
        AclDef {
            kind: ObjectKind::Table,
            schema: Some("s".into()),
            name: "a".into(),
            signature: None,
            owner: owner.into(),
            viewer: "app".into(),
            explicit: true,
            grants,
            foreign: vec![],
            columns: vec![],
            editable: true,
            unsupported: vec![],
            available: privileges_for(ObjectKind::Table, 160000).to_vec(),
        }
    }

    fn grant(who: Option<&str>, ps: &[Privilege]) -> AclGrant {
        AclGrant {
            grantee: who.map(Into::into),
            privileges: ps
                .iter()
                .map(|&privilege| PrivilegeGrant {
                    privilege,
                    grant_option: false,
                })
                .collect(),
        }
    }

    fn owner_all(owner: &str) -> AclGrant {
        default_grants(ObjectKind::Table, owner, 160000).remove(0)
    }

    #[test]
    fn yeni_tabloda_public_select_tek_grant() {
        let mut p = prep(
            ItemStatus::OnlySource,
            Some(t("a", new_cols(), vec![])),
            None,
        );
        p.source_acl = Some(acl(
            "alice",
            vec![grant(None, &[Privilege::Select]), owner_all("alice")],
        ));
        let g = gen_ok(&p, 160000);
        let privs = sql_of(&g, Phase::Privileges);
        assert_eq!(privs.len(), 1);
        assert!(privs[0].starts_with("GRANT SELECT ON TABLE") && privs[0].ends_with("TO PUBLIC"));
        insta::assert_snapshot!("yeni_tablo_yetki", render(&g));
    }

    #[test]
    fn farkli_tabloda_revoke_ve_grant() {
        let mut p = prep(
            ItemStatus::Different,
            Some(t("a", id_cols(), vec![])),
            Some(t("a", id_cols(), vec![])),
        );
        p.source_acl = Some(acl(
            "alice",
            vec![owner_all("alice"), grant(Some("rw"), &[Privilege::Insert])],
        ));
        p.target_acl = Some(acl(
            "alice",
            vec![owner_all("alice"), grant(None, &[Privilege::Select])],
        ));
        let g = gen_ok(&p, 160000);
        assert!(g.parts.iter().all(|(ph, _)| *ph == Phase::Privileges));
        insta::assert_snapshot!("farkli_tablo_yetki", render(&g));
    }

    #[test]
    fn sahip_farki_object_planinda_owner_to() {
        let mut s = table("a", id_cols(), vec![]);
        s.owner = Some("bob".into());
        let mut tg = table("a", id_cols(), vec![]);
        tg.owner = Some("alice".into());
        let mut p = prep(
            ItemStatus::Different,
            Some(ObjectDef::Table(s)),
            Some(ObjectDef::Table(tg)),
        );
        p.source_acl = Some(acl("bob", vec![owner_all("bob")]));
        p.target_acl = Some(acl("alice", vec![owner_all("alice")]));
        let g = gen_ok(&p, 160000);
        assert!(sql_of(&g, Phase::Tables)[0].contains("OWNER TO"));
        assert!(sql_of(&g, Phase::Privileges).is_empty());
    }

    #[test]
    fn yeniden_kurulan_gorunumde_yetkiler_varsayilandan_baslar() {
        let mut p = prep(
            ItemStatus::Different,
            Some(ObjectDef::View(view("SELECT 2 AS a"))),
            Some(ObjectDef::View(view("SELECT 1 AS a"))),
        );
        let mut a = acl(
            "alice",
            vec![owner_all("alice"), grant(None, &[Privilege::Select])],
        );
        a.name = "v".into();
        p.source_acl = Some(a.clone());
        p.target_acl = Some(a);
        let g = gen_ok(&p, 160000);
        assert_eq!(sql_of(&g, Phase::Privileges).len(), 1);
        // Yetkiler zaten Privileges aşamasında veriliyor: üreticinin uyarısı düşer.
        assert!(
            g.warnings.iter().all(|w| !w.contains("yetkiler (GRANT)")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn yeniden_kurulan_gorunumde_yetki_secenegi_kapaliysa_uyari_kalir() {
        let p = prep(
            ItemStatus::Different,
            Some(ObjectDef::View(view("SELECT 2 AS a"))),
            Some(ObjectDef::View(view("SELECT 1 AS a"))),
        );
        let g = gen_ok(&p, 160000);
        assert!(g.warnings.iter().any(|w| w.contains("yetkiler (GRANT)")));
    }

    #[test]
    fn bilinmeyen_tur_manual() {
        let sch = || {
            ObjectDef::Schema(pgt_ddl::SchemaDef {
                name: "x".into(),
                owner: None,
                comment: None,
            })
        };
        let p = prep(ItemStatus::Different, Some(sch()), Some(sch()));
        let e = generate(&p, &ctx(160000), "app").unwrap_err();
        assert_eq!(e.problem, ItemProblem::Manual);
    }

    #[test]
    fn bos_birlesik_plan_manual() {
        let p = prep(
            ItemStatus::Different,
            Some(t("a", id_cols(), vec![])),
            Some(t("a", id_cols(), vec![])),
        );
        let e = generate(&p, &ctx(160000), "app").unwrap_err();
        assert_eq!(e.problem, ItemProblem::Manual);
        assert_eq!(e.messages, ["Fark üreticiyle ifade edilemiyor."]);
    }

    #[test]
    fn same_bos_ve_problemli_uretilmez() {
        let g = gen_ok(&prep(ItemStatus::Same, None, None), 160000);
        assert!(g.parts.is_empty() && g.risk == Risk::Safe);
        let mut p = prep(
            ItemStatus::OnlySource,
            Some(t("a", id_cols(), vec![])),
            None,
        );
        p.problem = Some(ItemProblem::Manual);
        assert_eq!(
            generate(&p, &ctx(160000), "app").unwrap_err().problem,
            ItemProblem::Manual
        );
    }

    #[test]
    fn fk_add_plan_yalniz_add_constraint() {
        let base = table("a", id_cols(), vec![pk(Some(1))]);
        let plan = fk_add_plan(
            &base,
            &[fk("f1", Some(99), "b"), fk("f2", None, "c")],
            &ctx(160000),
        )
        .unwrap();
        assert_eq!(plan.statements.len(), 2);
        assert!(
            plan.statements
                .iter()
                .all(|s| s.sql.starts_with("ALTER TABLE") && s.sql.contains("ADD CONSTRAINT"))
        );
        assert!(plan.statements.iter().all(|s| !s.sql.contains("DROP")));
    }
}
