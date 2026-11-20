//! Varsayılan yetkiler farkı (Plan 20 Karar 12): yüklenen `DefaultAclDef` ile düzenlenmiş kopyası arasındaki fark →
//! `ALTER DEFAULT PRIVILEGES FOR ROLE … [IN SCHEMA …] GRANT/REVOKE …`. Alıcı/yetki kuralları ve ifade sırası nesne
//! yetkileriyle ortaktır (`build::acl`); sahip burada `role`'dür, `CASCADE` yoktur, tüm ifadeler Safe'tir.

use crate::acl::{DefaultAclDef, DefaultAclObjectType, default_privileges_for, default_sections};
use crate::model::FieldError;
use crate::plan::{Plan, Risk};
use crate::quote::quote_ident;

use super::acl::{check_old_grantees, diff_rows, grantee_statements, validate_rows};
use super::freesql::composed_is_clean;
use super::{BuildCtx, check_old_ident, check_old_schema_ident, err};

/// Plan notu (Ruling R0a): plan boş değilse tek uyarı olarak eklenir.
pub const NOTE: &str = "Yalnızca bundan sonra oluşturulan nesneleri etkiler; mevcut nesnelerin yetkileri değişmez.";

const IDENTITY_MSG: &str = "Varsayılan yetki kimliği (rol, şema, bağlı rol) değiştirilemez.";
const READ_ONLY_MSG: &str = "Bu varsayılan yetkiler salt okunurdur; değiştirilemez.";
const SECTIONS_MSG: &str = "Bölümler sabit sırada ve uygun türlerin tamamıyla olmalıdır.";
const BASELINE_MSG: &str = "Genel kapsamdan gelen değer buradan değiştirilemez.";
const COMPOSED_BROKEN: &str = "İfade güvenli biçimde oluşturulamadı.";

/// `ON <TÜR>` parçası.
fn on_clause(t: DefaultAclObjectType) -> &'static str {
    match t {
        DefaultAclObjectType::Tables => "ON TABLES",
        DefaultAclObjectType::Sequences => "ON SEQUENCES",
        DefaultAclObjectType::Functions => "ON FUNCTIONS",
        DefaultAclObjectType::Types => "ON TYPES",
        DefaultAclObjectType::Schemas => "ON SCHEMAS",
    }
}

/// Bölüm türleri beklenen sırayla (kümesi + sırası) birebir eşleşiyor mu.
fn sections_match(d: &DefaultAclDef) -> bool {
    let want = default_sections(d.schema.is_some());
    d.sections.len() == want.len() && d.sections.iter().zip(want).all(|(s, t)| s.object_type == *t)
}

/// Yüklü kimliğin (rol, şema) gevşek doğrulaması.
fn check_identity(d: &DefaultAclDef, e: &mut Vec<FieldError>) {
    check_old_ident("object", &d.role, e);
    if let Some(s) = &d.schema {
        check_old_schema_ident("object", s, e);
    }
}

/// Yüklü tanımın üreticide reddedilecek kimlik/alıcı hataları (rol, şema, bölümlerin eski alıcıları); yükleyici bunlar
/// varsa tanımı salt okunur yapar (`load_acl::builder_gate_default`).
pub(crate) fn loaded_errors(d: &DefaultAclDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_identity(d, &mut e);
    for (i, s) in d.sections.iter().enumerate() {
        check_old_grantees(&format!("sections[{i}].grants"), &s.grants, &mut e);
    }
    e
}

/// Bölüm bölüm fark (Karar 6 sırası, bölüm sırasıyla). Fark yoksa boş plan; boş değilse tek plan notu eklenir.
pub fn default_privileges(ctx: &BuildCtx, old: &DefaultAclDef, new: &DefaultAclDef) -> Result<Plan, Vec<FieldError>> {
    if old.role != new.role || old.schema != new.schema || old.viewer != new.viewer {
        return Err(vec![err("object", IDENTITY_MSG)]);
    }
    // Bölüm kümesi, sırası ve `available` listeleri kimliktir (eski ≠ yeni → `sections`).
    if !sections_match(old) || !sections_match(new) || old.sections.iter().zip(&new.sections).any(|(o, n)| o.available != n.available) {
        return Err(vec![err("sections", SECTIONS_MSG)]);
    }
    // Kanonik sıra: tüm sürümlerin kümesi (MAINTAIN dahil); geçersiz yetki doğrulamada reddedilir.
    let diffs: Vec<_> = old
        .sections
        .iter()
        .zip(&new.sections)
        .map(|(o, n)| (o.object_type, diff_rows(default_privileges_for(o.object_type, 170000), &o.grants, &n.grants)))
        .collect();
    let changed = diffs.iter().any(|(_, ds)| ds.iter().any(|(_, d)| !d.is_empty()));
    if !old.editable {
        // Salt okunur: değişiklik yoksa boş plan; kapı yalnızca yüklenen (`old`) tanıma bakar.
        let baseline_changed = old.sections.iter().zip(&new.sections).any(|(o, n)| o.baseline != n.baseline);
        return if changed || baseline_changed { Err(vec![err("object", READ_ONLY_MSG)]) } else { Ok(Plan::default()) };
    }
    let mut e = vec![];
    check_identity(old, &mut e);
    for (i, (o, n)) in old.sections.iter().zip(&new.sections).enumerate() {
        if o.baseline != n.baseline {
            e.push(err(&format!("sections[{i}].baseline"), BASELINE_MSG));
        }
        // İkinci küme (`170000`) yalnız MAINTAIN iletisini "sürüm" ile "tür" arasında ayırmak için.
        validate_rows(
            &format!("sections[{i}].grants"),
            (default_privileges_for(n.object_type, ctx.version_num), default_privileges_for(n.object_type, 170000)),
            &old.role,
            (&o.grants, &n.grants),
            &mut e,
        );
    }
    if !e.is_empty() {
        return Err(e);
    }

    let head = match &old.schema {
        Some(s) => format!("ALTER DEFAULT PRIVILEGES FOR ROLE {} IN SCHEMA {} ", quote_ident(&old.role), quote_ident(s)),
        None => format!("ALTER DEFAULT PRIVILEGES FOR ROLE {} ", quote_ident(&old.role)),
    };
    let mut statements = vec![];
    for (t, ds) in &diffs {
        let (mut revokes, mut grants) = (vec![], vec![]);
        for (who, d) in ds {
            let (r, g) = grantee_statements(d, *who, (&head, on_clause(*t)), (Risk::Safe, None, ""));
            revokes.extend(r);
            grants.extend(g);
        }
        statements.extend(revokes.into_iter().chain(grants));
    }
    // Nihai ifade taraması (ucuz savunma; adlar tırnaklı).
    if statements.iter().any(|s| !composed_is_clean(&s.sql)) {
        return Err(vec![err("object", COMPOSED_BROKEN)]);
    }
    let warnings = if statements.is_empty() { vec![] } else { vec![NOTE.to_string()] };
    Ok(Plan { statements, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::{AclGrant, DefaultAclSection, Privilege, Privilege::*, PrivilegeGrant};
    use crate::build::{assert_single_statements, render};
    use DefaultAclObjectType::*;

    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
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

    /// Yüklenmiş varsayılan: rol `own`; genel kapsamda her bölümde sahibin tüm yetkileri (Functions'ta PUBLIC EXECUTE de).
    fn def(schema: Option<&str>) -> DefaultAclDef {
        let sections = default_sections(schema.is_some())
            .iter()
            .map(|&t| {
                let all = default_privileges_for(t, 160000);
                let mut grants = vec![];
                if schema.is_none() {
                    if t == Functions {
                        grants.push(row(None, &plain(all)));
                    }
                    grants.push(row(Some("own"), &plain(all)));
                }
                DefaultAclSection { object_type: t, grants, baseline: vec![], available: default_privileges_for(t, 160000).to_vec() }
            })
            .collect();
        DefaultAclDef { role: "own".into(), schema: schema.map(Into::into), viewer: "app".into(), sections, editable: true, unsupported: vec![] }
    }
    fn section(d: &mut DefaultAclDef, t: DefaultAclObjectType) -> &mut DefaultAclSection {
        d.sections.iter_mut().find(|s| s.object_type == t).unwrap()
    }
    fn ok(old: &DefaultAclDef, new: &DefaultAclDef) -> Plan {
        let p = default_privileges(&ctx(), old, new).unwrap_or_else(|e| panic!("{e:?}"));
        assert_single_statements(&p);
        for s in &p.statements {
            assert!(s.transactional && !s.redacted && s.risk == Risk::Safe && s.note.is_none(), "{s:?}");
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
    fn general_scope_tables_grant() {
        let old = def(None);
        let mut new = old.clone();
        section(&mut new, Tables).grants.push(row(Some("r"), &plain(&[Select])));
        let p = ok(&old, &new);
        assert_eq!(sqls(&p), ["ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT SELECT ON TABLES TO r"]);
        assert_eq!(p.warnings, [NOTE]);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn functions_revoke_public_execute() {
        let old = def(None);
        let mut new = old.clone();
        section(&mut new, Functions).grants.retain(|g| g.grantee.is_some());
        let p = ok(&old, &new);
        assert_eq!(sqls(&p), ["ALTER DEFAULT PRIVILEGES FOR ROLE own REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC"]);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn schemas_section_grant() {
        let old = def(None);
        let mut new = old.clone();
        section(&mut new, Schemas).grants.push(row(Some("r"), &plain(&[Usage])));
        let p = ok(&old, &new);
        assert_eq!(sqls(&p), ["ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT USAGE ON SCHEMAS TO r"]);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn schema_scope_grant_and_revoke() {
        let mut old = def(Some("Şema \"x\""));
        section(&mut old, Sequences).grants.push(row(Some("old_r"), &plain(&[Usage, Select])));
        let mut new = old.clone();
        section(&mut new, Tables).grants.push(row(None, &plain(&[Select])));
        section(&mut new, Sequences).grants[0].privileges.retain(|p| p.privilege == Usage);
        let p = ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER DEFAULT PRIVILEGES FOR ROLE own IN SCHEMA \"Şema \"\"x\"\"\" GRANT SELECT ON TABLES TO PUBLIC",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own IN SCHEMA \"Şema \"\"x\"\"\" REVOKE SELECT ON SEQUENCES FROM old_r",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn grant_option_add_and_remove() {
        let mut old = def(None);
        section(&mut old, Tables).grants.push(row(Some("r"), &[(Select, true), (Insert, false)]));
        let mut new = old.clone();
        {
            let g = &mut section(&mut new, Tables).grants[1];
            g.privileges[0].grant_option = false;
            g.privileges[1].grant_option = true;
        }
        let p = ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER DEFAULT PRIVILEGES FOR ROLE own REVOKE GRANT OPTION FOR SELECT ON TABLES FROM r",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT INSERT ON TABLES TO r WITH GRANT OPTION",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn multi_section_order() {
        let old = def(None);
        let mut new = old.clone();
        section(&mut new, Schemas).grants.push(row(Some("a"), &plain(&[Usage])));
        section(&mut new, Types).grants.push(row(Some("a"), &plain(&[Usage])));
        section(&mut new, Tables).grants.push(row(Some("a"), &plain(&[Select])));
        section(&mut new, Tables).grants.push(row(Some("b"), &[(Select, true)]));
        section(&mut new, Functions).grants.retain(|g| g.grantee.is_some());
        section(&mut new, Sequences).grants[0].privileges.retain(|p| p.privilege != Update);
        let p = ok(&old, &new);
        // Bölümler Tables, Sequences, Functions, Types, Schemas sırasında; her bölümde önce REVOKE'lar, sonra GRANT'lar.
        assert_eq!(
            sqls(&p),
            [
                "ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT SELECT ON TABLES TO a",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT SELECT ON TABLES TO b WITH GRANT OPTION",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own REVOKE UPDATE ON SEQUENCES FROM own",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT USAGE ON TYPES TO a",
                "ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT USAGE ON SCHEMAS TO a",
            ]
        );
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn empty_diff_has_no_note() {
        for d in [def(None), def(Some("s"))] {
            let p = ok(&d, &d);
            assert!(p.statements.is_empty() && p.warnings.is_empty());
        }
        // Salt okunur tanım: değişiklik yoksa boş plan.
        let ro = DefaultAclDef { editable: false, unsupported: vec!["yetki SET".into()], ..def(None) };
        assert!(ok(&ro, &ro).statements.is_empty());
        // Sıra farkı fark değildir.
        let old = def(None);
        let mut new = old.clone();
        for s in &mut new.sections {
            s.grants.reverse();
            for g in &mut s.grants {
                g.privileges.reverse();
            }
        }
        assert!(ok(&old, &new).statements.is_empty());
    }

    #[test]
    fn errors_table() {
        let base = def(None);
        let edit = |b: &DefaultAclDef, f: &dyn Fn(&mut DefaultAclDef)| {
            let mut n = b.clone();
            f(&mut n);
            n
        };
        let scoped = def(Some("s"));
        let cases: Vec<(&str, DefaultAclDef, DefaultAclDef, Vec<&str>)> = vec![
            (
                "şema kapsamında Schemas bölümü",
                scoped.clone(),
                edit(&scoped, &|n| n.sections.push(DefaultAclSection { object_type: Schemas, grants: vec![], baseline: vec![], available: default_privileges_for(Schemas, 160000).to_vec() })),
                vec!["sections"],
            ),
            (
                "eksik bölüm",
                base.clone(),
                edit(&base, &|n| {
                    n.sections.pop();
                }),
                vec!["sections"],
            ),
            ("yinelenen bölüm", base.clone(), edit(&base, &|n| n.sections[1] = n.sections[0].clone()), vec!["sections"]),
            ("bölüm sırası", base.clone(), edit(&base, &|n| n.sections.swap(0, 1)), vec!["sections"]),
            (
                "baseline değişimi",
                scoped.clone(),
                edit(&scoped, &|n| n.sections[2].baseline.push(row(None, &plain(&[Execute])))),
                vec!["sections[2].baseline"],
            ),
            (
                "PG 16 Maintain",
                base.clone(),
                edit(&base, &|n| n.sections[0].grants[0].privileges.push(PrivilegeGrant { privilege: Maintain, grant_option: false })),
                vec!["sections[0].grants[0].privileges[7].privilege"],
            ),
            (
                "tür dışı yetki",
                base.clone(),
                edit(&base, &|n| n.sections[3].grants[0].privileges.push(PrivilegeGrant { privilege: Select, grant_option: false })),
                vec!["sections[3].grants[0].privileges[1].privilege"],
            ),
            ("kimlik: rol", base.clone(), edit(&base, &|n| n.role = "x".into()), vec!["object"]),
            ("kimlik: şema", base.clone(), edit(&base, &|n| n.schema = Some("s".into())), vec!["object"]),
            ("kimlik: viewer", base.clone(), edit(&base, &|n| n.viewer = "x".into()), vec!["object"]),
            ("kimlik: available", base.clone(), edit(&base, &|n| n.sections[0].available.pop().map(drop).unwrap_or(())), vec!["sections"]),
            (
                "editable = false",
                DefaultAclDef { editable: false, ..base.clone() },
                DefaultAclDef { editable: false, ..edit(&base, &|n| n.sections[0].grants.clear()) },
                vec!["object"],
            ),
            (
                "yinelenen alıcı",
                base.clone(),
                edit(&base, &|n| n.sections[1].grants.push(row(Some("own"), &plain(&[Usage])))),
                vec!["sections[1].grants[1].grantee"],
            ),
            (
                "Some(public)",
                base.clone(),
                edit(&base, &|n| n.sections[0].grants.push(row(Some("public"), &plain(&[Select])))),
                vec!["sections[0].grants[1].grantee"],
            ),
            (
                "PUBLIC grantOption",
                base.clone(),
                edit(&base, &|n| n.sections[0].grants.push(row(None, &[(Select, true)]))),
                vec!["sections[0].grants[1].privileges[0].grantOption"],
            ),
            (
                "sahip grantOption",
                base.clone(),
                edit(&base, &|n| n.sections[2].grants[1].privileges[0].grant_option = true),
                vec!["sections[2].grants[1].privileges[0].grantOption"],
            ),
            (
                "denetim karakterli alıcı",
                base.clone(),
                edit(&base, &|n| n.sections[0].grants.push(row(Some("x\n--"), &plain(&[Select])))),
                vec!["sections[0].grants[1].grantee"],
            ),
        ];
        for (label, old, new, want) in cases {
            let e = default_privileges(&ctx(), &old, &new).expect_err(label);
            assert_eq!(paths(e), want, "{label}");
        }
        // PG 17'de Maintain Tables bölümünde geçerli.
        let c17 = BuildCtx { version_num: 170000, ..Default::default() };
        let m = edit(&base, &|n| n.sections[0].grants.push(row(Some("r"), &plain(&[Maintain]))));
        assert_eq!(
            sqls(&default_privileges(&c17, &base, &m).unwrap()),
            ["ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT MAINTAIN ON TABLES TO r"]
        );
        // Sahibin geri verilen yetkisi grantOption = false ile geçerli.
        let mut less = base.clone();
        section(&mut less, Tables).grants[0].privileges.retain(|p| p.privilege != Delete);
        assert_eq!(sqls(&ok(&less, &base)), ["ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT DELETE ON TABLES TO own"]);
        // Mesaj.
        let b = edit(&scoped, &|n| n.sections[0].baseline.push(row(None, &plain(&[Select]))));
        assert_eq!(default_privileges(&ctx(), &scoped, &b).unwrap_err()[0].message, BASELINE_MSG);
    }

    #[test]
    fn public_named_roles_are_real_roles() {
        // PG yalnız tam küçük harf `public`'i ayırır; `"PUBLIC"`/`"Public"` gerçek roldür.
        for name in ["PUBLIC", "Public"] {
            let mut loaded = def(None);
            section(&mut loaded, Tables).grants.push(row(Some(name), &plain(&[Select])));
            crate::acl::sort_grants(&mut section(&mut loaded, Tables).grants, default_privileges_for(Tables, 170000));
            assert!(ok(&loaded, &loaded).statements.is_empty(), "{name}");
            assert_eq!(
                sqls(&ok(&loaded, &def(None))),
                [format!("ALTER DEFAULT PRIVILEGES FOR ROLE own REVOKE SELECT ON TABLES FROM \"{name}\"")],
                "{name}"
            );
            assert_eq!(
                sqls(&ok(&def(None), &loaded)),
                [format!("ALTER DEFAULT PRIVILEGES FOR ROLE own GRANT SELECT ON TABLES TO \"{name}\"")],
                "{name}"
            );
        }
        let mut new = def(None);
        section(&mut new, Tables).grants.push(row(Some("public"), &plain(&[Select])));
        let e = default_privileges(&ctx(), &def(None), &new).unwrap_err();
        assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("sections[0].grants[1].grantee", "PUBLIC için alıcıyı boş bırakın."));
    }

    #[test]
    fn hostile_names_single_statement() {
        let mut old = def(Some("s'; DROP --"));
        old.role = "Rol \"ğ\"".into();
        old.sections[0].grants.push(row(Some("a b"), &plain(&[Select])));
        let mut new = old.clone();
        new.sections[0].grants[0] = row(Some("c\"d"), &[(Select, true)]);
        let p = ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER DEFAULT PRIVILEGES FOR ROLE \"Rol \"\"ğ\"\"\" IN SCHEMA \"s'; DROP --\" REVOKE SELECT ON TABLES FROM \"a b\"",
                "ALTER DEFAULT PRIVILEGES FOR ROLE \"Rol \"\"ğ\"\"\" IN SCHEMA \"s'; DROP --\" GRANT SELECT ON TABLES TO \"c\"\"d\" WITH GRANT OPTION",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }
}
