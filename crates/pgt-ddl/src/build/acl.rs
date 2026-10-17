//! Nesne yetkileri farkı (Plan 20 Karar 2, 5–10): yüklenen `AclDef` ile düzenlenmiş kopyası arasındaki fark →
//! `REVOKE [GRANT OPTION FOR] … [CASCADE]` ve `GRANT … [WITH GRANT OPTION]` ifadeleri.

//!
//! Eski taraf yüklü tanımdır (yönetici yeniden yükleme eşitliğiyle doğrular): kimlik alanları yalnızca gevşek
//! doğrulanır ve eskiyle yeni birebir eşit olmalıdır; yeni alıcılar katı (`check_ident`), eskiden gelenler gevşek.

use std::collections::BTreeSet;

use crate::acl::{AclDef, AclGrant, Privilege, acl_supported, privileges_for};
use crate::model::{FieldError, ObjectKind};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident};

use super::freesql::composed_is_clean;
use super::{BuildCtx, check_ident, check_old_ident, check_old_ref, check_old_schema_ident, err, scan_semicolons, stmt};

/// `REVOKE … CASCADE` ifade notu (Karar 7).
pub const CASCADE_NOTE: &str = "Alıcının bu yetkiyle başkalarına verdiği yetkiler de geri alınır.";
/// Karar 8.
pub const OWNER_REVOKE_WARNING: &str = "Sahip kendi yetkisini kaldırıyor; sahip bu yetkiyi istediği an geri verebilir.";
/// Karar 9.
pub const VIEWER_REVOKE_WARNING: &str = "Bağlı rol bu nesne üzerindeki yetkisini kaybedebilir.";
pub const CONNECT_WARNING: &str =
    "Bu veritabanına bağlanma yetkisi kaldırılıyor; bağlı rol (ve PUBLIC üzerinden bağlananlar) yeniden bağlanamayabilir.";
pub const USAGE_WARNING: &str = "Şemadaki nesnelere erişim kesilebilir.";

const UNSUPPORTED_KIND_MSG: &str = "Bu nesne türünün yetkileri yoktur.";
const IDENTITY_MSG: &str = "Nesne kimliği (tür, şema, ad, imza, sahip, bağlı rol) yetki düzenlemede değiştirilemez.";
const READ_ONLY_MSG: &str = "Bu nesnenin yetkileri salt okunurdur; değiştirilemez.";
const FOREIGN_MSG: &str = "Başka verenlerin verdiği yetkiler buradan değiştirilemez.";
const COLUMNS_MSG: &str = "Kolon yetkileri buradan değiştirilemez.";
const INVALID_PRIVILEGE_MSG: &str = "Bu nesne türünde geçersiz yetki.";
const MAINTAIN_MSG: &str = "MAINTAIN yetkisi PostgreSQL 17 ve sonrasında kullanılabilir.";
const DUP_GRANTEE_MSG: &str = "Bu alıcı zaten listede.";
const DUP_PRIVILEGE_MSG: &str = "Bu yetki bu alıcı için zaten listede.";
const PUBLIC_MSG: &str = "PUBLIC için alıcıyı boş bırakın.";
const OWNER_GRANT_OPTION_MSG: &str = "Sahip, yetkileri zaten devredebilir; GRANT OPTION ayarlanamaz.";
const SIGNATURE_MSG: &str = "İmza metni güvenli biçimde yazılamıyor.";
const SIGNATURE_MISSING_MSG: &str = "Fonksiyon/procedure için imza gerekli.";
const SIGNATURE_EXTRA_MSG: &str = "Bu nesne türünde imza olmaz.";
const SCHEMA_MISSING_MSG: &str = "Bu nesne türü için şema gerekli.";
const SCHEMA_EXTRA_MSG: &str = "Bu nesne türünde şema olmaz.";
const COMPOSED_BROKEN: &str = "İfade güvenli biçimde oluşturulamadı.";

fn is_routine(kind: ObjectKind) -> bool {
    matches!(kind, ObjectKind::Function | ObjectKind::Procedure)
}

/// Nihai ifade taraması başarısızsa hatanın yazılacağı alan: dış kaynaklı tek serbest parça imzadır.
fn composed_path(kind: ObjectKind) -> &'static str {
    if is_routine(kind) { "signature" } else { "object" }
}

/// İmza tek başına temiz olmalı: dış `;` yok, kapanmamış tırnak/yorum yok, parantezler dengeli (hedefin `(…)`
/// sarmalından taşamaz), sonda açık `--` yorumu yok, NUL/denetim karakteri yok.
fn signature_is_clean(sig: &str) -> bool {
    !sig.chars().any(char::is_control) && composed_is_clean(sig) && !scan_semicolons(sig).ends_in_line_comment
}

/// GRANT/REVOKE hedefi (`TABLE s.t`, `FUNCTION s.f(integer)`, `DATABASE d` …).
pub fn object_target(d: &AclDef) -> Result<String, Vec<FieldError>> {
    if !acl_supported(d.kind) {
        return Err(vec![err("object", UNSUPPORTED_KIND_MSG)]);
    }
    let mut e = vec![];
    let schemaless = matches!(d.kind, ObjectKind::Schema | ObjectKind::Database);
    match (&d.schema, schemaless) {
        (Some(_), true) => e.push(err("schema", SCHEMA_EXTRA_MSG)),
        (None, false) => e.push(err("schema", SCHEMA_MISSING_MSG)),
        (Some(s), false) => check_old_schema_ident("schema", s, &mut e),
        (None, true) => {}
    }
    match d.kind {
        ObjectKind::Schema => check_old_schema_ident("name", &d.name, &mut e),
        _ => check_old_ident("name", &d.name, &mut e),
    }
    match (&d.signature, is_routine(d.kind)) {
        (None, true) => e.push(err("signature", SIGNATURE_MISSING_MSG)),
        (Some(_), false) => e.push(err("signature", SIGNATURE_EXTRA_MSG)),
        (Some(s), true) if !signature_is_clean(s) => e.push(err("signature", SIGNATURE_MSG)),
        _ => {}
    }
    if !e.is_empty() {
        return Err(e);
    }
    let rel = || qualified(d.schema.as_deref().unwrap_or_default(), &d.name);
    let target = match d.kind {
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView => format!("TABLE {}", rel()),
        ObjectKind::Sequence => format!("SEQUENCE {}", rel()),
        ObjectKind::Function => format!("FUNCTION {}({})", rel(), d.signature.as_deref().unwrap_or_default()),
        ObjectKind::Procedure => format!("PROCEDURE {}({})", rel(), d.signature.as_deref().unwrap_or_default()),
        ObjectKind::Schema => format!("SCHEMA {}", quote_ident(&d.name)),
        ObjectKind::Database => format!("DATABASE {}", quote_ident(&d.name)),
        ObjectKind::Type => format!("TYPE {}", rel()),
        ObjectKind::Domain => format!("DOMAIN {}", rel()),
        _ => unreachable!("acl_supported"),
    };
    if !composed_is_clean(&target) {
        return Err(vec![err(composed_path(d.kind), COMPOSED_BROKEN)]);
    }
    Ok(target)
}

fn same_identity(a: &AclDef, b: &AclDef) -> bool {
    a.kind == b.kind
        && a.schema == b.schema
        && a.name == b.name
        && a.signature == b.signature
        && a.owner == b.owner
        && a.viewer == b.viewer
        && a.explicit == b.explicit
}

fn find<'a>(grants: &'a [AclGrant], who: Option<&str>) -> Option<&'a AclGrant> {
    grants.iter().find(|g| g.grantee.as_deref() == who)
}

/// Alıcının yetkisi ve GRANT OPTION durumu (`None` = yok).
fn grant_option(g: Option<&AclGrant>, p: Privilege) -> Option<bool> {
    g?.privileges.iter().find(|x| x.privilege == p).map(|x| x.grant_option)
}

/// Yeni tarafın alıcı/yetki kuralları (Karar 2, 8, 10) ve eski alıcı adlarının gevşek doğrulaması.
fn validate_grants(ctx: &BuildCtx, old: &AclDef, new: &AclDef, e: &mut Vec<FieldError>) {
    let allowed = privileges_for(new.kind, ctx.version_num);
    let allowed_17 = privileges_for(new.kind, 170000);
    for (i, g) in old.grants.iter().enumerate() {
        if let Some(n) = &g.grantee {
            check_old_ref(&format!("old.grants[{i}].grantee"), n, e);
        }
    }
    let mut seen = BTreeSet::new();
    for (i, g) in new.grants.iter().enumerate() {
        let path = format!("grants[{i}].grantee");
        if let Some(n) = &g.grantee {
            if n.eq_ignore_ascii_case("public") {
                // PG `public` adlı rolü yasaklar; adla yazmak `PUBLIC` anlamına gelmez/hata verir.
                e.push(err(&path, PUBLIC_MSG));
            } else if find(&old.grants, Some(n)).is_some() {
                check_old_ref(&path, n, e);
            } else {
                check_ident(&path, n, e);
            }
        }
        if !seen.insert(g.grantee.as_deref()) {
            e.push(err(&path, DUP_GRANTEE_MSG));
        }
        let is_owner = g.grantee.as_deref() == Some(new.owner.as_str());
        let old_row = find(&old.grants, g.grantee.as_deref());
        let mut privs = BTreeSet::new();
        for (j, p) in g.privileges.iter().enumerate() {
            let pp = format!("grants[{i}].privileges[{j}]");
            if !allowed.contains(&p.privilege) {
                let m = if p.privilege == Privilege::Maintain && allowed_17.contains(&Privilege::Maintain) {
                    MAINTAIN_MSG
                } else {
                    INVALID_PRIVILEGE_MSG
                };
                e.push(err(&format!("{pp}.privilege"), m));
            }
            if !privs.insert(p.privilege.sql()) {
                e.push(err(&pp, DUP_PRIVILEGE_MSG));
            }
            // Sahip satırında GRANT OPTION eskisinden farklı olamaz (PG 0LP01'i önden yakalar).
            if is_owner && grant_option(old_row, p.privilege).unwrap_or(false) != p.grant_option {
                e.push(err(&format!("{pp}.grantOption"), OWNER_GRANT_OPTION_MSG));
            }
        }
    }
}

/// Bir alıcının farkı (kanonik sırada): kaldırılan, GRANT OPTION'ı kaldırılan, eklenen, GRANT OPTION'lı verilecek.
#[derive(Default)]
struct GranteeDiff {
    revoke: Vec<Privilege>,
    revoke_option: Vec<Privilege>,
    grant: Vec<Privilege>,
    grant_option: Vec<Privilege>,
}

fn diff_grantee(order: &[Privilege], old: Option<&AclGrant>, new: Option<&AclGrant>) -> GranteeDiff {
    let mut d = GranteeDiff::default();
    for &p in order {
        match (grant_option(old, p), grant_option(new, p)) {
            (Some(_), None) => d.revoke.push(p),
            (Some(true), Some(false)) => d.revoke_option.push(p),
            (None, Some(false)) => d.grant.push(p),
            (None, Some(true)) | (Some(false), Some(true)) => d.grant_option.push(p),
            _ => {}
        }
    }
    d
}

fn list(ps: &[Privilege]) -> String {
    ps.iter().map(|p| p.sql()).collect::<Vec<_>>().join(", ")
}

fn push_warning(w: &mut Vec<String>, msg: &str) {
    if !w.iter().any(|x| x == msg) {
        w.push(msg.to_string());
    }
}

/// Karar 6 sırası: önce tüm alıcıların `REVOKE` ve `REVOKE GRANT OPTION FOR`'ları, sonra `GRANT` ve
/// `GRANT … WITH GRANT OPTION`'ları; alıcılar `PUBLIC` önce, sonra ad (bayt sırası). `cascade` yalnız REVOKE'lara
/// eklenir (DataLoss + not); REVOKE yoksa etkisizdir. Fark yoksa boş plan.
pub fn privileges(ctx: &BuildCtx, old: &AclDef, new: &AclDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    if !acl_supported(old.kind) {
        return Err(vec![err("object", UNSUPPORTED_KIND_MSG)]);
    }
    if !same_identity(old, new) {
        return Err(vec![err("object", IDENTITY_MSG)]);
    }
    // Kanonik sıra: tüm sürümlerin kümesi (MAINTAIN dahil); geçersiz yetki zaten doğrulamada reddedilir.
    let order = privileges_for(old.kind, 170000);
    let grantees: BTreeSet<Option<&str>> =
        old.grants.iter().chain(&new.grants).map(|g| g.grantee.as_deref()).collect();
    let diffs: Vec<(Option<&str>, GranteeDiff)> = grantees
        .into_iter()
        .map(|who| (who, diff_grantee(order, find(&old.grants, who), find(&new.grants, who))))
        .collect();
    let grants_changed = diffs
        .iter()
        .any(|(_, d)| !(d.revoke.is_empty() && d.revoke_option.is_empty() && d.grant.is_empty() && d.grant_option.is_empty()));
    if !old.editable {
        // Salt okunur: değişiklik yoksa boş plan (yüklü tanımın kendisiyle karşılaştırılması).
        return if grants_changed || old.foreign != new.foreign || old.columns != new.columns {
            Err(vec![err("object", READ_ONLY_MSG)])
        } else {
            Ok(Plan::default())
        };
    }
    let mut e = vec![];
    if old.foreign != new.foreign {
        e.push(err("foreign", FOREIGN_MSG));
    }
    if old.columns != new.columns {
        e.push(err("columns", COLUMNS_MSG));
    }
    validate_grants(ctx, old, new, &mut e);
    let target = match object_target(old) {
        Ok(t) => Some(t),
        Err(te) => {
            e.extend(te);
            None
        }
    };
    if !e.is_empty() {
        return Err(e);
    }
    let target = target.unwrap_or_default();

    let suffix = if cascade { " CASCADE" } else { "" };
    let (revoke_risk, revoke_note) = if cascade { (Risk::DataLoss, Some(CASCADE_NOTE)) } else { (Risk::Destructive, None) };
    let mut revokes: Vec<Statement> = vec![];
    let mut grants: Vec<Statement> = vec![];
    let mut warnings = vec![];
    for (who, d) in &diffs {
        let g = who.map_or_else(|| "PUBLIC".to_string(), quote_ident);
        if !d.revoke.is_empty() {
            revokes.push(stmt(format!("REVOKE {} ON {target} FROM {g}{suffix}", list(&d.revoke)), revoke_risk, revoke_note));
            let is_viewer = *who == Some(old.viewer.as_str());
            if *who == Some(old.owner.as_str()) {
                push_warning(&mut warnings, OWNER_REVOKE_WARNING);
            }
            if is_viewer {
                push_warning(&mut warnings, VIEWER_REVOKE_WARNING);
            }
            // Kendini kilitleme: PUBLIC ya da bağlı rol (Karar 9).
            if who.is_none() || is_viewer {
                if old.kind == ObjectKind::Database && d.revoke.contains(&Privilege::Connect) {
                    push_warning(&mut warnings, CONNECT_WARNING);
                }
                if old.kind == ObjectKind::Schema && d.revoke.contains(&Privilege::Usage) {
                    push_warning(&mut warnings, USAGE_WARNING);
                }
            }
        }
        if !d.revoke_option.is_empty() {
            revokes.push(stmt(
                format!("REVOKE GRANT OPTION FOR {} ON {target} FROM {g}{suffix}", list(&d.revoke_option)),
                revoke_risk,
                revoke_note,
            ));
        }
        if !d.grant.is_empty() {
            grants.push(stmt(format!("GRANT {} ON {target} TO {g}", list(&d.grant)), Risk::Safe, None));
        }
        if !d.grant_option.is_empty() {
            grants.push(stmt(format!("GRANT {} ON {target} TO {g} WITH GRANT OPTION", list(&d.grant_option)), Risk::Safe, None));
        }
    }
    let statements: Vec<Statement> = revokes.into_iter().chain(grants).collect();
    // Nihai ifade taraması (ucuz savunma; adlar tırnaklı, imza yukarıda tek başına tarandı).
    if statements.iter().any(|s| !composed_is_clean(&s.sql)) {
        return Err(vec![err(composed_path(old.kind), COMPOSED_BROKEN)]);
    }
    Ok(Plan { statements, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::{AclGrant, ColumnGrant, ForeignGrant, Privilege, Privilege::*, PrivilegeGrant};
    use crate::build::{assert_single_statements, render};
    use crate::model::ObjectKind;
    use crate::plan::Risk;

    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }
    fn ctx17() -> BuildCtx {
        BuildCtx { version_num: 170000, ..Default::default() }
    }

    fn row(who: Option<&str>, ps: &[(Privilege, bool)]) -> AclGrant {
        AclGrant {
            grantee: who.map(Into::into),
            privileges: ps.iter().map(|&(privilege, grant_option)| PrivilegeGrant { privilege, grant_option }).collect(),
        }
    }
    fn plain(ps: &[Privilege]) -> Vec<(Privilege, bool)> {
        ps.iter().map(|p| (*p, false)).collect()
    }

    /// Tür için yüklenmiş tanım: sahip `own` (tüm yetkiler), bağlı rol `app`.
    fn def(kind: ObjectKind) -> AclDef {
        let (schema, name, signature) = match kind {
            ObjectKind::Schema => (None, "s", None),
            ObjectKind::Database => (None, "d", None),
            ObjectKind::Function | ObjectKind::Procedure => (Some("s".to_string()), "f", Some("integer".to_string())),
            _ => (Some("s".to_string()), "t", None),
        };
        let all = crate::acl::privileges_for(kind, 160000);
        AclDef {
            kind,
            schema,
            name: name.into(),
            signature,
            owner: "own".into(),
            viewer: "app".into(),
            explicit: true,
            grants: if all.is_empty() { vec![] } else { vec![row(Some("own"), &plain(all))] },
            foreign: vec![],
            columns: vec![],
            editable: true,
            unsupported: vec![],
        }
    }
    fn table() -> AclDef {
        def(ObjectKind::Table)
    }
    fn with(d: &AclDef, extra: Vec<AclGrant>) -> AclDef {
        let mut x = d.clone();
        x.grants.extend(extra);
        crate::acl::sort_grants(&mut x.grants, crate::acl::privileges_for(x.kind, 170000));
        x
    }
    fn ok(old: &AclDef, new: &AclDef, cascade: bool) -> Plan {
        let p = privileges(&ctx(), old, new, cascade).unwrap_or_else(|e| panic!("{e:?}"));
        assert_single_statements(&p);
        for s in &p.statements {
            assert!(s.transactional && !s.redacted, "{s:?}");
        }
        p
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(e: Vec<FieldError>) -> Vec<String> {
        e.into_iter().map(|e| e.path).collect()
    }

    #[test]
    fn targets_per_kind() {
        let cases = [
            (ObjectKind::Table, "TABLE s.t"),
            (ObjectKind::View, "TABLE s.t"),
            (ObjectKind::MaterializedView, "TABLE s.t"),
            (ObjectKind::Sequence, "SEQUENCE s.t"),
            (ObjectKind::Function, "FUNCTION s.f(integer)"),
            (ObjectKind::Procedure, "PROCEDURE s.f(integer)"),
            (ObjectKind::Schema, "SCHEMA s"),
            (ObjectKind::Database, "DATABASE d"),
            (ObjectKind::Type, "TYPE s.t"),
            (ObjectKind::Domain, "DOMAIN s.t"),
        ];
        for (k, want) in cases {
            assert_eq!(object_target(&def(k)).unwrap(), want, "{k:?}");
        }
        let mut s = def(ObjectKind::Schema);
        s.name = "My \"S\"".into();
        assert_eq!(object_target(&s).unwrap(), "SCHEMA \"My \"\"S\"\"\"");
        let mut f = def(ObjectKind::Function);
        f.signature = Some(String::new());
        assert_eq!(object_target(&f).unwrap(), "FUNCTION s.f()");
        // Desteklenmeyen tür ve eksik/fazla kimlik parçaları.
        let mut ix = table();
        ix.kind = ObjectKind::Index;
        assert_eq!(paths(object_target(&ix).unwrap_err()), ["object"]);
        let mut nosig = def(ObjectKind::Procedure);
        nosig.signature = None;
        assert_eq!(paths(object_target(&nosig).unwrap_err()), ["signature"]);
        let mut sig = table();
        sig.signature = Some("integer".into());
        assert_eq!(paths(object_target(&sig).unwrap_err()), ["signature"]);
        let mut noschema = table();
        noschema.schema = None;
        assert_eq!(paths(object_target(&noschema).unwrap_err()), ["schema"]);
        let mut dbschema = def(ObjectKind::Database);
        dbschema.schema = Some("s".into());
        assert_eq!(paths(object_target(&dbschema).unwrap_err()), ["schema"]);
    }

    #[test]
    fn grant_only() {
        let old = table();
        let new = with(&old, vec![row(None, &plain(&[Select])), row(Some("Reader"), &plain(&[Insert, Select]))]);
        let p = ok(&old, &new, false);
        assert_eq!(sqls(&p), ["GRANT SELECT ON TABLE s.t TO PUBLIC", "GRANT SELECT, INSERT ON TABLE s.t TO \"Reader\""]);
        assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.note.is_none()));
        assert!(p.warnings.is_empty());
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn revoke_only_is_destructive() {
        let old = with(&table(), vec![row(Some("reader"), &plain(&[Select, Update, Delete]))]);
        let new = with(&table(), vec![row(Some("reader"), &plain(&[Select]))]);
        let p = ok(&old, &new, false);
        assert_eq!(sqls(&p), ["REVOKE UPDATE, DELETE ON TABLE s.t FROM reader"]);
        assert_eq!((p.statements[0].risk, p.statements[0].note.as_deref()), (Risk::Destructive, None));
        assert!(p.warnings.is_empty());
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn grant_option_add_on_existing_privilege() {
        let old = with(&table(), vec![row(Some("reader"), &[(Select, false), (Insert, false)])]);
        let new = with(&table(), vec![row(Some("reader"), &[(Select, true), (Insert, false), (Update, true)])]);
        let p = ok(&old, &new, false);
        // Var olan yetkiye GRANT OPTION eklemek yalnızca `WITH GRANT OPTION`'lı GRANT'tır (REVOKE yok).
        assert_eq!(sqls(&p), ["GRANT SELECT, UPDATE ON TABLE s.t TO reader WITH GRANT OPTION"]);
        assert_eq!(p.risk(), Risk::Safe);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn grant_option_remove() {
        let old = with(&table(), vec![row(Some("reader"), &[(Select, true), (Insert, true)])]);
        let new = with(&table(), vec![row(Some("reader"), &[(Select, false), (Insert, true)])]);
        let p = ok(&old, &new, false);
        assert_eq!(sqls(&p), ["REVOKE GRANT OPTION FOR SELECT ON TABLE s.t FROM reader"]);
        assert_eq!(p.statements[0].risk, Risk::Destructive);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn mixed_many_grantees_revokes_first_public_first() {
        let old = with(
            &table(),
            vec![
                row(None, &plain(&[Select, Insert])),
                row(Some("b"), &[(Select, true), (Update, false)]),
                row(Some("a"), &plain(&[Delete])),
                row(Some("gone"), &[(Select, true), (Trigger, false)]),
            ],
        );
        let new = with(
            &table(),
            vec![
                row(None, &plain(&[Select, Truncate])),
                row(Some("b"), &[(Select, false), (References, true)]),
                row(Some("a"), &[(Delete, true), (Insert, false)]),
                row(Some("Yeni"), &plain(&[Select])),
            ],
        );
        let p = ok(&old, &new, false);
        assert_eq!(
            sqls(&p),
            [
                "REVOKE INSERT ON TABLE s.t FROM PUBLIC",
                "REVOKE UPDATE ON TABLE s.t FROM b",
                "REVOKE GRANT OPTION FOR SELECT ON TABLE s.t FROM b",
                "REVOKE SELECT, TRIGGER ON TABLE s.t FROM gone",
                "GRANT TRUNCATE ON TABLE s.t TO PUBLIC",
                "GRANT SELECT ON TABLE s.t TO \"Yeni\"",
                "GRANT INSERT ON TABLE s.t TO a",
                "GRANT DELETE ON TABLE s.t TO a WITH GRANT OPTION",
                "GRANT REFERENCES ON TABLE s.t TO b WITH GRANT OPTION",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn grantee_removed_entirely_or_emptied_and_new_grantee() {
        let old = with(&table(), vec![row(Some("x"), &[(Select, true), (Insert, false)]), row(Some("y"), &plain(&[Select]))]);
        // `x` satırı hiç yok, `y` boş yetki listesiyle: ikisi de tüm yetkilerini kaybeder; `w` yeni, `z` yeni ama boş (ifade yok).
        let new = with(&table(), vec![row(Some("y"), &[]), row(Some("z"), &[]), row(Some("w"), &plain(&[Select]))]);
        let p = ok(&old, &new, false);
        assert_eq!(
            sqls(&p),
            ["REVOKE SELECT, INSERT ON TABLE s.t FROM x", "REVOKE SELECT ON TABLE s.t FROM y", "GRANT SELECT ON TABLE s.t TO w"]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn cascade_marks_revokes_data_loss_with_note() {
        let old = with(&table(), vec![row(Some("r"), &[(Select, true), (Insert, true)])]);
        let new = with(&table(), vec![row(Some("r"), &[(Insert, false)]), row(Some("s"), &plain(&[Select]))]);
        let p = ok(&old, &new, true);
        assert_eq!(
            sqls(&p),
            [
                "REVOKE SELECT ON TABLE s.t FROM r CASCADE",
                "REVOKE GRANT OPTION FOR INSERT ON TABLE s.t FROM r CASCADE",
                "GRANT SELECT ON TABLE s.t TO s",
            ]
        );
        assert_eq!(p.statements[0].risk, Risk::DataLoss);
        assert_eq!(p.statements[1].risk, Risk::DataLoss);
        assert_eq!(p.statements[0].note.as_deref(), Some(CASCADE_NOTE));
        assert_eq!(p.statements[1].note.as_deref(), Some(CASCADE_NOTE));
        assert_eq!((p.statements[2].risk, p.statements[2].note.as_deref()), (Risk::Safe, None));
        assert!(p.warnings.is_empty());
        insta::assert_snapshot!(render(&p));
        // REVOKE'suz planda `cascade` yok sayılır.
        let only_grant = with(&table(), vec![row(Some("s"), &plain(&[Select]))]);
        let p = ok(&table(), &only_grant, true);
        assert_eq!(sqls(&p), ["GRANT SELECT ON TABLE s.t TO s"]);
        assert!(p.statements[0].note.is_none() && p.warnings.is_empty() && p.risk() == Risk::Safe);
    }

    #[test]
    fn empty_diff_is_empty_plan_for_every_kind() {
        for k in [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Sequence,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Schema,
            ObjectKind::Database,
            ObjectKind::Type,
            ObjectKind::Domain,
        ] {
            let d = with(&def(k), vec![row(None, &[(crate::acl::privileges_for(k, 160000)[0], false)])]);
            let p = ok(&d, &d, true);
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{k:?}");
        }
        // Yetki sırası/satır sırası farkı fark değildir.
        let old = with(&table(), vec![row(Some("r"), &plain(&[Select, Insert]))]);
        let mut new = old.clone();
        new.grants.reverse();
        for g in &mut new.grants {
            g.privileges.reverse();
        }
        assert!(ok(&old, &new, false).statements.is_empty());
        // Salt okunur tanım: değişiklik yoksa boş plan.
        let ro = AclDef { editable: false, unsupported: vec!["yetki SET".into()], ..old.clone() };
        assert!(ok(&ro, &ro, false).statements.is_empty());
    }

    #[test]
    fn errors_table() {
        let base = with(&table(), vec![row(Some("r"), &plain(&[Select]))]);
        let edit = |f: &dyn Fn(&mut AclDef)| {
            let mut n = base.clone();
            f(&mut n);
            n
        };
        let cases: Vec<(&str, AclDef, AclDef, BuildCtx, Vec<&str>)> = vec![
            (
                "geçersiz yetki",
                base.clone(),
                edit(&|n| n.grants[1].privileges.push(PrivilegeGrant { privilege: Execute, grant_option: false })),
                ctx(),
                vec!["grants[1].privileges[1].privilege"],
            ),
            (
                "PG 16 MAINTAIN",
                base.clone(),
                edit(&|n| n.grants[1].privileges.push(PrivilegeGrant { privilege: Maintain, grant_option: false })),
                ctx(),
                vec!["grants[1].privileges[1].privilege"],
            ),
            ("yinelenen alıcı", base.clone(), edit(&|n| n.grants.push(row(Some("r"), &plain(&[Insert])))), ctx(), vec!["grants[2].grantee"]),
            (
                "yinelenen yetki",
                base.clone(),
                edit(&|n| n.grants[1].privileges.push(PrivilegeGrant { privilege: Select, grant_option: true })),
                ctx(),
                vec!["grants[1].privileges[1]"],
            ),
            ("Some(PUBLIC)", base.clone(), edit(&|n| n.grants.push(row(Some("PUBLIC"), &plain(&[Select])))), ctx(), vec!["grants[2].grantee"]),
            ("Some(public)", base.clone(), edit(&|n| n.grants.push(row(Some("public"), &plain(&[Select])))), ctx(), vec!["grants[2].grantee"]),
            ("boş alıcı", base.clone(), edit(&|n| n.grants.push(row(Some(""), &plain(&[Select])))), ctx(), vec!["grants[2].grantee"]),
            (
                "denetim karakterli alıcı",
                base.clone(),
                edit(&|n| n.grants.push(row(Some("x\n--"), &plain(&[Select])))),
                ctx(),
                vec!["grants[2].grantee"],
            ),
            (
                "sahip grantOption",
                base.clone(),
                edit(&|n| n.grants[0].privileges[2].grant_option = true),
                ctx(),
                vec!["grants[0].privileges[2].grantOption"],
            ),
            (
                "foreign değişimi",
                base.clone(),
                edit(&|n| {
                    n.foreign.push(ForeignGrant { grantor: "g".into(), grantee: None, privilege: Select, grant_option: false })
                }),
                ctx(),
                vec!["foreign"],
            ),
            (
                "columns değişimi",
                base.clone(),
                edit(&|n| {
                    n.columns.push(ColumnGrant {
                        column: "c".into(),
                        grantor: "own".into(),
                        grantee: None,
                        privilege: Select,
                        grant_option: false,
                    })
                }),
                ctx(),
                vec!["columns"],
            ),
            ("kimlik: ad", base.clone(), edit(&|n| n.name = "u".into()), ctx(), vec!["object"]),
            ("kimlik: şema", base.clone(), edit(&|n| n.schema = Some("x".into())), ctx(), vec!["object"]),
            ("kimlik: sahip", base.clone(), edit(&|n| n.owner = "x".into()), ctx(), vec!["object"]),
            ("kimlik: viewer", base.clone(), edit(&|n| n.viewer = "x".into()), ctx(), vec!["object"]),
            ("kimlik: explicit", base.clone(), edit(&|n| n.explicit = false), ctx(), vec!["object"]),
            ("kimlik: tür", base.clone(), edit(&|n| n.kind = ObjectKind::View), ctx(), vec!["object"]),
            (
                "editable = false",
                AclDef { editable: false, ..base.clone() },
                AclDef { editable: false, ..edit(&|n| n.grants[1].privileges.clear()) },
                ctx(),
                vec!["object"],
            ),
            (
                "desteklenmeyen tür",
                AclDef { kind: ObjectKind::Index, grants: vec![], ..base.clone() },
                AclDef { kind: ObjectKind::Index, grants: vec![row(None, &plain(&[Select]))], ..base.clone() },
                ctx(),
                vec!["object"],
            ),
        ];
        for (label, old, new, c, want) in cases {
            let e = privileges(&c, &old, &new, false).expect_err(label);
            assert_eq!(paths(e), want, "{label}");
        }
        // Mesajlar (Karar 2, 8, 10).
        let msg = |old: &AclDef, new: &AclDef, c: &BuildCtx| privileges(c, old, new, false).unwrap_err()[0].message.clone();
        let invalid = edit(&|n| n.grants[1].privileges.push(PrivilegeGrant { privilege: Execute, grant_option: false }));
        assert_eq!(msg(&base, &invalid, &ctx()), "Bu nesne türünde geçersiz yetki.");
        let dup = edit(&|n| n.grants.push(row(Some("r"), &plain(&[Insert]))));
        assert_eq!(msg(&base, &dup, &ctx()), "Bu alıcı zaten listede.");
        let public = edit(&|n| n.grants.push(row(Some("Public"), &plain(&[Select]))));
        assert_eq!(msg(&base, &public, &ctx()), "PUBLIC için alıcıyı boş bırakın.");
        let owner_go = edit(&|n| n.grants[0].privileges[0].grant_option = true);
        assert_eq!(msg(&base, &owner_go, &ctx()), "Sahip, yetkileri zaten devredebilir; GRANT OPTION ayarlanamaz.");
        let mut ix = base.clone();
        ix.kind = ObjectKind::Trigger;
        assert_eq!(msg(&ix, &ix, &ctx()), "Bu nesne türünün yetkileri yoktur.");
        // MAINTAIN PG 17+'da geçerli (tablo türlerinde); başka türde her sürümde geçersiz.
        let maintain = edit(&|n| n.grants[1].privileges.push(PrivilegeGrant { privilege: Maintain, grant_option: false }));
        assert_eq!(sqls(&privileges(&ctx17(), &base, &maintain, false).unwrap()), ["GRANT MAINTAIN ON TABLE s.t TO r"]);
        assert!(msg(&base, &maintain, &ctx()).contains("17"));
        let seq = def(ObjectKind::Sequence);
        let seq_m = with(&seq, vec![row(Some("r"), &plain(&[Maintain]))]);
        assert_eq!(paths(privileges(&ctx17(), &seq, &seq_m, false).unwrap_err()), ["grants[1].privileges[0].privilege"]);
        // Sahibin yeni eklenen yetkisi grantOption = false ile geri verilebilir; true ile verilemez.
        let owner_less = AclDef { grants: vec![row(Some("own"), &plain(&[Select]))], ..table() };
        let back = AclDef { grants: vec![row(Some("own"), &plain(&[Select, Insert]))], ..table() };
        assert_eq!(sqls(&privileges(&ctx(), &owner_less, &back, false).unwrap()), ["GRANT INSERT ON TABLE s.t TO own"]);
        let back_go = AclDef { grants: vec![row(Some("own"), &[(Select, false), (Insert, true)])], ..table() };
        assert_eq!(paths(privileges(&ctx(), &owner_less, &back_go, false).unwrap_err()), ["grants[0].privileges[1].grantOption"]);
    }

    #[test]
    fn warnings_self_lockout() {
        let warn = |old: &AclDef, new: &AclDef| ok(old, new, false).warnings;
        // Sahip kendi yetkisini kaldırıyor.
        let t = table();
        let mut own_less = t.clone();
        own_less.grants[0].privileges.retain(|p| p.privilege != Delete);
        assert_eq!(warn(&t, &own_less), [OWNER_REVOKE_WARNING]);
        // Bağlı rol (viewer): yetki kaybı uyarısı; yalnız GRANT OPTION kaldırmak uyarı değildir.
        let with_app = with(&t, vec![row(Some("app"), &[(Select, true), (Insert, false)])]);
        let app_less = with(&t, vec![row(Some("app"), &[(Select, true)])]);
        assert_eq!(warn(&with_app, &app_less), [VIEWER_REVOKE_WARNING]);
        let app_no_go = with(&t, vec![row(Some("app"), &[(Select, false), (Insert, false)])]);
        assert!(warn(&with_app, &app_no_go).is_empty());
        // Viewer = sahip: iki uyarı, tekrarsız.
        let mut same = t.clone();
        same.viewer = "own".into();
        let mut same_less = same.clone();
        same_less.grants[0].privileges.retain(|p| p.privilege != Delete && p.privilege != Insert);
        assert_eq!(warn(&same, &same_less), [OWNER_REVOKE_WARNING, VIEWER_REVOKE_WARNING]);
        // Database: PUBLIC ya da viewer'dan CONNECT.
        let db = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Temporary, Connect]))]);
        let db_less = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Temporary]))]);
        assert_eq!(warn(&db, &db_less), [CONNECT_WARNING]);
        let db_tmp = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Connect]))]);
        assert!(warn(&db, &db_tmp).is_empty(), "TEMPORARY kaldırmak bağlantıyı kesmez");
        let dbv = with(&def(ObjectKind::Database), vec![row(Some("app"), &plain(&[Connect]))]);
        assert_eq!(warn(&dbv, &def(ObjectKind::Database)), [VIEWER_REVOKE_WARNING, CONNECT_WARNING]);
        let dbo = with(&def(ObjectKind::Database), vec![row(Some("other"), &plain(&[Connect]))]);
        assert!(warn(&dbo, &def(ObjectKind::Database)).is_empty());
        // Schema: PUBLIC/viewer'dan USAGE; CREATE kaldırmak uyarı değildir.
        let sc = with(&def(ObjectKind::Schema), vec![row(None, &plain(&[Usage, Create]))]);
        let sc_no_usage = with(&def(ObjectKind::Schema), vec![row(None, &plain(&[Create]))]);
        assert_eq!(warn(&sc, &sc_no_usage), [USAGE_WARNING]);
        let sc_no_create = with(&def(ObjectKind::Schema), vec![row(None, &plain(&[Usage]))]);
        assert!(warn(&sc, &sc_no_create).is_empty());
        let scv = with(&def(ObjectKind::Schema), vec![row(Some("app"), &plain(&[Usage]))]);
        assert_eq!(warn(&scv, &def(ObjectKind::Schema)), [VIEWER_REVOKE_WARNING, USAGE_WARNING]);
        // Uyarılar ad/değer içermez.
        for w in [OWNER_REVOKE_WARNING, VIEWER_REVOKE_WARNING, CONNECT_WARNING, USAGE_WARNING] {
            assert!(!w.contains("app") && !w.contains("own"), "{w}");
        }
        // Birden çok alıcı aynı uyarıyı üretse de bir kez.
        let two = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Connect])), row(Some("app"), &plain(&[Connect]))]);
        assert_eq!(warn(&two, &def(ObjectKind::Database)), [CONNECT_WARNING, VIEWER_REVOKE_WARNING]);
    }

    #[test]
    fn warnings_snapshot() {
        let db = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Temporary, Connect]))]);
        let mut db_less = with(&def(ObjectKind::Database), vec![row(None, &plain(&[Temporary]))]);
        db_less.grants.retain(|g| g.grantee.as_deref() != Some("own"));
        let p = ok(&db, &db_less, false);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn hostile_names_single_statement() {
        let mut old = def(ObjectKind::Function);
        old.schema = Some("Şema \"x\"".into());
        old.name = "f; DROP TABLE t --".into();
        old.signature = Some("\"My Type\"[], VARIADIC integer[]".into());
        old.owner = "Sahip ğ".into();
        old.grants = vec![row(Some("Sahip ğ"), &plain(&[Execute])), row(None, &plain(&[Execute]))];
        let mut new = old.clone();
        new.grants = vec![row(Some("Sahip ğ"), &plain(&[Execute])), row(Some("a \"b\" ç"), &[(Execute, true)])];
        let p = ok(&old, &new, false);
        assert_eq!(
            sqls(&p),
            [
                "REVOKE EXECUTE ON FUNCTION \"Şema \"\"x\"\"\".\"f; DROP TABLE t --\"(\"My Type\"[], VARIADIC integer[]) FROM PUBLIC",
                "GRANT EXECUTE ON FUNCTION \"Şema \"\"x\"\"\".\"f; DROP TABLE t --\"(\"My Type\"[], VARIADIC integer[]) TO \"a \"\"b\"\" ç\" WITH GRANT OPTION",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn signature_injection_is_invalid() {
        for sig in ["integer); DROP TABLE x; --", "integer\0", "integer) TO x --", "'", "integer /* x"] {
            let mut old = def(ObjectKind::Function);
            old.signature = Some(sig.into());
            let new = with(&old, vec![row(Some("r"), &plain(&[Execute]))]);
            let e = privileges(&ctx(), &old, &new, false).expect_err(sig);
            assert_eq!(paths(e), ["signature"], "{sig:?}");
            assert!(object_target(&old).is_err(), "{sig:?}");
        }
    }
}
