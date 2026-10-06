//! Tablo alter'ının kısıt farkı: `oid` kimlik eşlemesi, kolon yeniden adlandırma eşlemesiyle karşılaştırma,
//! yeniden adlandırma / DROP+ADD / VALIDATE kararları ve ifade metinleri. İfade sırası `table_alter.rs`'te.
//!
//! Kararlar:
//! - Eşleme `oid` ile (yalnızca yüklemede dolar): `new`'de `oid: Some(n)` eski tanımda yoksa tanım bayattır (hata),
//!   `oid: None` yeni kısıttır, eski tanımda olup yenide olmayan silinir. Liste sırası anlamsızdır.
//! - "Değişmemiş" sayma: tür etiketli tüm alanlar eşit; eski kısıtın kolon listeleri (PK/UNIQUE kolonları ve
//!   INCLUDE, FK yerel kolonları) önce kolon yeniden adlandırma eşlemesinden (eski ad → yeni ad) geçirilir — PG kısıtı
//!   kolonla birlikte izler, gereksiz DROP+ADD üretilmez. Silinen kolona başvuran eski kısıt eşlenemez → yapısal.
//!   `Check.expr` metin eşitliğiyle (boşluk farkı da yapısaldır); FK `ref_*` doğrudan karşılaştırılır. TEK istisna:
//!   kendine başvuran FK (hedef = eski tablo adı) — PG hedefi oid ile izlediğinden, hedef şema/tablo/kolonları
//!   eşlemeden geçmiş hâli de (yayılmış) ham hâli de (yayılmamış) değişmemiş sayılır.
//! - `not_valid` karşılaştırmada yoksayılır: eski doğrulanmamış + yeni doğrulanmış ve yapısal değişim yok →
//!   `VALIDATE CONSTRAINT` (ad değişimiyle birlikte olabilir; yeni adla, ad değişiminden sonra). Doğrulanmış kısıt
//!   yapısal değişim olmadan tekrar `NOT VALID` yapılamaz (hata); yapısal değişimde yeniden ekleme `NOT VALID` olabilir.
//! - Yalnızca ad değişimi → `RENAME CONSTRAINT`; değişmeyen mevcut kısıtın adı boşaltılamaz (hata). Yapısal değişim
//!   → eski adla `DROP CONSTRAINT` (adım 3) + yeni tanımla (yeni ad, yeni kolon adları) `ADD` (adım 10).
//! - DROP sırası: önce yabancı anahtarlar, sonra diğerleri (her grup eski sırayla) — kendine başvuran FK'nin bağlı
//!   olduğu PK/UNIQUE ondan önce düşmesin. ADD sırası: önce FK dışındakiler, sonra FK'ler (her grup yeni sırayla) —
//!   kendine başvuran FK'nin hedef PK/UNIQUE'i önce kurulsun.
//! - Başka tablolardaki bağımlı FK'ler yerelde bilinmez: DROP notu 2BP01'i anlatır, CASCADE sunulmaz. FK hedefinin
//!   varlığı sunucuda denetlenir (uygulama transaction içinde, hata geri alınır).

use std::collections::{HashMap, HashSet};

use super::table::constraint_sql;
use super::{check_ident, err};
use crate::model::{ConstraintDef, ConstraintKind, FieldError, TableDef};
use crate::quote::quote_ident;

pub(crate) const DROP_CONSTRAINT_NOTE: &str = "Kısıt kaldırılır (PRIMARY KEY/UNIQUE ise indeksi de); başka yabancı anahtarlar bu kısıta bağlıysa uygulama 2BP01 hatasıyla durur (CASCADE sunulmaz)";
pub(crate) const RENAME_CONSTRAINT_NOTE: &str = "Kısıta adla başvuran betikler etkilenir";
pub(crate) const RENAME_INDEX_CONSTRAINT_NOTE: &str =
    "Kısıtın indeksi de aynı adla yeniden adlandırılır; kısıta/indekse adla başvuran betikler etkilenir";
pub(crate) const ADD_CONSTRAINT_NOTE: &str =
    "Mevcut satırları tarar (PRIMARY KEY/UNIQUE indeks oluşturur); işlem boyunca tablo kilitlenir";
pub(crate) const ADD_FK_NOTE: &str =
    "Mevcut satırları tarar; işlem boyunca tablo ve hedef tablo kilitlenir; hedef tablo/kolonlar sunucuda denetlenir";
pub(crate) const ADD_NOT_VALID_NOTE: &str =
    "NOT VALID: mevcut satırlar denetlenmez, yalnızca yeni satırlar; sonra VALIDATE CONSTRAINT ile doğrulanabilir";
pub(crate) const VALIDATE_NOTE: &str =
    "Tabloyu tarar (yazmaları engellemeyen SHARE UPDATE EXCLUSIVE kilidiyle); kurala uymayan satır varsa başarısız olur";
pub(crate) const PK_DROP_WARNING: &str = "Birincil anahtar düşürülüyor: başka tablolardaki yabancı anahtarlar ona bağlıysa uygulama 2BP01 hatasıyla durur; önce bağımlı yabancı anahtarları kaldırın";
pub(crate) const ADD_CONSTRAINT_WARNING: &str = "Kısıt ekleme mevcut satırları tarar (PRIMARY KEY/UNIQUE indeks oluşturur); işlem boyunca tablo yazmalara kapalı kalır ve büyük tablolarda uzun sürer. FOREIGN KEY ve CHECK için NOT VALID ile ekleyip sonra doğrulamak kilidi kısaltır";
const REVALIDATE_MSG: &str =
    "Doğrulanmış kısıt tekrar NOT VALID yapılamaz; kısıtı değiştirmeden NOT VALID işaretini kaldırın.";
const KEPT_NAME_MSG: &str = "Mevcut kısıtın adı boş bırakılamaz.";

/// Eski tanımın kısıtları: yalnızca SQL'e giren ad (DROP/RENAME) ve eşleme anahtarı `oid` doğrulanır; kısıt gövdesi
/// SQL'e girmez (yalnızca karşılaştırılır).
pub(crate) fn validate_old_constraints(old: &TableDef, e: &mut Vec<FieldError>) {
    let mut seen = HashSet::new();
    for (i, c) in old.constraints.iter().enumerate() {
        let np = format!("old.constraints[{i}].name");
        match &c.name {
            Some(n) => check_ident(&np, n, e),
            None => e.push(err(&np, "Yüklenmiş kısıtın adı yok; tanımı yeniden yükleyin.")),
        }
        let p = format!("old.constraints[{i}].oid");
        match c.oid {
            None => e.push(err(&p, "Yüklenmiş kısıtın oid değeri yok; tanımı yeniden yükleyin.")),
            Some(n) if !seen.insert(n) => e.push(err(
                &p,
                format!("oid {n} birden fazla kısıtta; tanımı yeniden yükleyin."),
            )),
            Some(_) => {}
        }
    }
}

/// Yeni tanımın kısıt oid'leri: benzersiz ve eski tanımda var (yoksa tanım bayat ya da oynanmış).
pub(crate) fn check_new_oids(old: &TableDef, new: &TableDef, e: &mut Vec<FieldError>) {
    let known: HashSet<u32> = old.constraints.iter().filter_map(|c| c.oid).collect();
    let mut seen = HashSet::new();
    for (i, c) in new.constraints.iter().enumerate() {
        let Some(n) = c.oid else { continue };
        let p = format!("constraints[{i}].oid");
        if !seen.insert(n) {
            e.push(err(&p, format!("oid {n} birden fazla kısıtta.")));
        } else if !known.contains(&n) {
            e.push(err(
                &p,
                format!("oid {n} mevcut tabloda yok; tanım bayat, yeniden yükleyin."),
            ));
        }
    }
}

/// Kısıt farkı. Konumlar (`usize`) yeni tanımdaki kısıt konumudur (hata yolları için).
#[derive(Default)]
pub(crate) struct ConstraintDiff<'a> {
    /// Adım 3: silinen ve yapısal değişen kısıtlar (ESKİ tanım; FK'ler önce).
    pub drops: Vec<&'a ConstraintDef>,
    /// Adım 9: yalnızca adı değişen kısıtlar (konum, eski ad, yeni ad).
    pub renames: Vec<(usize, &'a str, &'a str)>,
    /// Adım 10: yeni ve yapısal değişen kısıtlar (YENİ tanım; FK'ler sonda).
    pub adds: Vec<(usize, &'a ConstraintDef)>,
    /// Adım 10'dan sonra: doğrulanacak kısıtların (yeni) adları.
    pub validates: Vec<&'a str>,
    /// Yerinde kalan (yeniden kurulmayan) mevcut kısıtlar (yeni tanım): tip değişimi uyarısı için.
    pub kept: Vec<&'a ConstraintDef>,
    /// Alter kuralı hataları (NOT VALID geri dönüşü, değişmeyen kısıtın boş adı).
    pub errs: Vec<FieldError>,
}

impl ConstraintDiff<'_> {
    /// İfade üreten bir fark var mı.
    pub fn changed(&self) -> bool {
        !(self.drops.is_empty() && self.renames.is_empty() && self.adds.is_empty() && self.validates.is_empty())
    }
}

fn is_fk(c: &ConstraintDef) -> bool {
    matches!(c.kind, ConstraintKind::ForeignKey { .. })
}

/// Kolon listesini yeniden adlandırma eşlemesinden geçirir; eşlenemeyen (silinen) kolon → `None`.
fn map_cols(cols: &[String], m: &HashMap<&str, &str>) -> Option<Vec<String>> {
    cols.iter().map(|c| m.get(c.as_str()).map(|n| n.to_string())).collect()
}

/// Eski kısıt türünü yeni tanımın ad uzayına taşır. `self_ref = Some((şema, tablo))` ise FK hedefi de (yeni tablo
/// adı ve kolon eşlemesiyle) taşınır.
fn map_kind(k: &ConstraintKind, m: &HashMap<&str, &str>, self_ref: Option<(&str, &str)>) -> Option<ConstraintKind> {
    Some(match k {
        ConstraintKind::PrimaryKey { columns, include } => ConstraintKind::PrimaryKey {
            columns: map_cols(columns, m)?,
            include: map_cols(include, m)?,
        },
        ConstraintKind::Unique {
            columns,
            include,
            nulls_not_distinct,
        } => ConstraintKind::Unique {
            columns: map_cols(columns, m)?,
            include: map_cols(include, m)?,
            nulls_not_distinct: *nulls_not_distinct,
        },
        ConstraintKind::Check { .. } => k.clone(),
        ConstraintKind::ForeignKey {
            columns,
            ref_schema,
            ref_table,
            ref_columns,
            match_type,
            on_update,
            on_delete,
        } => {
            let (ref_schema, ref_table, ref_columns) = match self_ref {
                Some((s, t)) => (s.to_string(), t.to_string(), map_cols(ref_columns, m)?),
                None => (ref_schema.clone(), ref_table.clone(), ref_columns.clone()),
            };
            ConstraintKind::ForeignKey {
                columns: map_cols(columns, m)?,
                ref_schema,
                ref_table,
                ref_columns,
                match_type: *match_type,
                on_update: *on_update,
                on_delete: *on_delete,
            }
        }
    })
}

/// Yapısal olarak aynı mı (ad ve `not_valid` hariç tüm alanlar, eski taraf eşlemeden geçmiş).
fn same_structure(old: &TableDef, new: &TableDef, oc: &ConstraintDef, c: &ConstraintDef, m: &HashMap<&str, &str>) -> bool {
    if oc.deferrable != c.deferrable || oc.initially_deferred != c.initially_deferred {
        return false;
    }
    if map_kind(&oc.kind, m, None).as_ref() == Some(&c.kind) {
        return true;
    }
    let self_ref = matches!(&oc.kind, ConstraintKind::ForeignKey { ref_schema, ref_table, .. }
        if *ref_schema == old.schema && *ref_table == old.name);
    self_ref && map_kind(&oc.kind, m, Some((&new.schema, &new.name))).as_ref() == Some(&c.kind)
}

/// Kısıt farkı. `col_map`: korunan kolonların eski adı → yeni adı (silinen kolonlar yok). Ön koşul: eski taraf
/// `validate_old_constraints`, yeni oid'ler `check_new_oids` geçti (arama her zaman başarılı).
pub(crate) fn diff<'a>(old: &'a TableDef, new: &'a TableDef, col_map: &HashMap<&str, &str>) -> ConstraintDiff<'a> {
    let mut d = ConstraintDiff::default();
    let by_oid: HashMap<u32, &ConstraintDef> = old.constraints.iter().filter_map(|c| c.oid.map(|n| (n, c))).collect();
    let mut keep: HashSet<u32> = HashSet::new();
    let mut adds = vec![];
    for (i, c) in new.constraints.iter().enumerate() {
        let Some(oc) = c.oid.and_then(|n| by_oid.get(&n).copied()) else {
            adds.push((i, c));
            continue;
        };
        if !same_structure(old, new, oc, c, col_map) {
            // Yapısal değişim: eski adla DROP + yeni tanımla ADD (NOT VALID yeniden eklemede serbest).
            adds.push((i, c));
            continue;
        }
        keep.extend(c.oid);
        d.kept.push(c);
        let Some(to) = c.name.as_deref() else {
            d.errs.push(err(&format!("constraints[{i}].name"), KEPT_NAME_MSG));
            continue;
        };
        let from = oc.name.as_deref().unwrap_or(to);
        if from != to {
            d.renames.push((i, from, to));
        }
        match (oc.not_valid, c.not_valid) {
            (true, false) => d.validates.push(to),
            (false, true) => d.errs.push(err(&format!("constraints[{i}].notValid"), REVALIDATE_MSG)),
            _ => {}
        }
    }
    let dropped = |c: &&ConstraintDef| c.oid.is_some_and(|n| !keep.contains(&n));
    d.drops = old.constraints.iter().filter(|c| is_fk(c)).filter(dropped).collect();
    d.drops.extend(old.constraints.iter().filter(|c| !is_fk(c)).filter(dropped));
    d.adds = adds.iter().filter(|(_, c)| !is_fk(c)).copied().collect();
    d.adds.extend(adds.iter().filter(|(_, c)| is_fk(c)).copied());
    d
}

/// Kısıtın kolon listesi + INCLUDE (CHECK için boş: ifadedeki kolonlar bilinmez).
pub(crate) fn constraint_columns(c: &ConstraintDef) -> Vec<&str> {
    match &c.kind {
        ConstraintKind::PrimaryKey { columns, include } | ConstraintKind::Unique { columns, include, .. } => {
            columns.iter().chain(include).map(String::as_str).collect()
        }
        ConstraintKind::ForeignKey { columns, .. } => columns.iter().map(String::as_str).collect(),
        ConstraintKind::Check { .. } => vec![],
    }
}

pub(crate) fn is_pk(c: &ConstraintDef) -> bool {
    matches!(c.kind, ConstraintKind::PrimaryKey { .. })
}

pub(crate) fn drop_sql(q: &str, name: &str) -> String {
    format!("ALTER TABLE {q} DROP CONSTRAINT {}", quote_ident(name))
}

pub(crate) fn rename_sql(q: &str, from: &str, to: &str) -> String {
    format!("ALTER TABLE {q} RENAME CONSTRAINT {} TO {}", quote_ident(from), quote_ident(to))
}

/// Yeniden adlandırma notu (konum yeni tanımdaki kısıt).
pub(crate) fn rename_note(c: &ConstraintDef) -> &'static str {
    match c.kind {
        ConstraintKind::PrimaryKey { .. } | ConstraintKind::Unique { .. } => RENAME_INDEX_CONSTRAINT_NOTE,
        _ => RENAME_CONSTRAINT_NOTE,
    }
}

/// `ADD [CONSTRAINT <q>] <tanım> [NOT VALID]` (NOT VALID yalnızca FK/CHECK; PK/UNIQUE'te doğrulama hatası).
pub(crate) fn add_sql(q: &str, c: &ConstraintDef) -> String {
    format!(
        "ALTER TABLE {q} ADD {}{}",
        constraint_sql(c),
        if c.not_valid { " NOT VALID" } else { "" }
    )
}

/// ADD notu ve mevcut satırları tarayıp taramadığı.
pub(crate) fn add_note(c: &ConstraintDef) -> (&'static str, bool) {
    if c.not_valid {
        (ADD_NOT_VALID_NOTE, false)
    } else if is_fk(c) {
        (ADD_FK_NOTE, true)
    } else {
        (ADD_CONSTRAINT_NOTE, true)
    }
}

/// Serbest SQL taraması başarısızsa hata yolu: CHECK ifadesi ya da kısıtın kendisi.
pub(crate) fn add_path(i: usize, c: &ConstraintDef) -> String {
    match c.kind {
        ConstraintKind::Check { .. } => format!("constraints[{i}].expr"),
        _ => format!("constraints[{i}]"),
    }
}

pub(crate) fn validate_sql(q: &str, name: &str) -> String {
    format!("ALTER TABLE {q} VALIDATE CONSTRAINT {}", quote_ident(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::table_alter::{COLUMN_ORDER_WARNING, LOCK_TIMEOUT_WARNING, REWRITE_WARNING};
    use crate::build::type_syntax::is_spoofing_format;
    use crate::build::{BuildCtx, alter as balter, assert_single_statements, render, scan_semicolons};
    use crate::model::*;
    use crate::plan::{Plan, Risk};

    const MANAGED: &str = "Bölüm/kalıtımlı tablonun kolon ve kısıtları ana tablodan yönetilir.";

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 180000,
            ..Default::default()
        }
    }
    fn o(d: TableDef) -> ObjectDef {
        ObjectDef::Table(d)
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn lc(n: i16, name: &str, ty: &str) -> ColumnDef {
        ColumnDef {
            attnum: Some(n),
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
    fn col(name: &str, ty: &str) -> ColumnDef {
        ColumnDef { attnum: None, ..lc(0, name, ty) }
    }
    fn con(oid: Option<u32>, name: Option<&str>, kind: ConstraintKind) -> ConstraintDef {
        ConstraintDef {
            oid,
            name: name.map(str::to_string),
            kind,
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        }
    }
    fn pk(cols: &[&str]) -> ConstraintKind {
        ConstraintKind::PrimaryKey { columns: s(cols), include: vec![] }
    }
    fn uq(cols: &[&str], include: &[&str]) -> ConstraintKind {
        ConstraintKind::Unique { columns: s(cols), include: s(include), nulls_not_distinct: false }
    }
    fn ck(expr: &str) -> ConstraintKind {
        ConstraintKind::Check { expr: expr.into(), no_inherit: false }
    }
    fn fk(cols: &[&str], ref_table: &str, ref_cols: &[&str]) -> ConstraintKind {
        ConstraintKind::ForeignKey {
            columns: s(cols),
            ref_schema: "public".into(),
            ref_table: ref_table.into(),
            ref_columns: s(ref_cols),
            match_type: FkMatch::Simple,
            on_update: FkAction::NoAction,
            on_delete: FkAction::NoAction,
        }
    }
    /// Yüklenmiş tablo: PK, INCLUDE'lu UNIQUE, CHECK, dış FK, kendine başvuran FK.
    /// Kısıt konumları: 0 t_pkey, 1 t_code_key, 2 t_qty_check, 3 t_org_fkey, 4 t_parent_fkey.
    fn tbl() -> TableDef {
        TableDef {
            schema: "public".into(),
            name: "t".into(),
            owner: None,
            tablespace: None,
            persistence: Persistence::Permanent,
            columns: vec![
                ColumnDef { not_null: true, ..lc(1, "id", "bigint") },
                ColumnDef { not_null: true, ..lc(2, "code", "text") },
                lc(3, "parent_id", "bigint"),
                lc(4, "qty", "integer"),
                lc(5, "org_id", "integer"),
                lc(6, "note", "text"),
            ],
            constraints: vec![
                con(Some(100), Some("t_pkey"), pk(&["id"])),
                con(Some(101), Some("t_code_key"), uq(&["code"], &["note"])),
                con(Some(102), Some("t_qty_check"), ck("qty >= 0")),
                con(Some(103), Some("t_org_fkey"), fk(&["org_id"], "org", &["id"])),
                con(Some(104), Some("t_parent_fkey"), fk(&["parent_id"], "t", &["id"])),
            ],
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        }
    }
    /// Kısıtsız aynı tablo.
    fn bare() -> TableDef {
        TableDef { constraints: vec![], ..tbl() }
    }
    fn with(d: &TableDef, f: impl FnOnce(&mut TableDef)) -> TableDef {
        let mut d = d.clone();
        f(&mut d);
        d
    }
    fn cm<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ConstraintDef {
        d.constraints.iter_mut().find(|c| c.name.as_deref() == Some(name)).unwrap()
    }
    fn colm<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ColumnDef {
        d.columns.iter_mut().find(|c| c.name == name).unwrap()
    }
    type FkParts<'a> = (
        &'a mut Vec<String>,
        &'a mut String,
        &'a mut String,
        &'a mut Vec<String>,
        &'a mut FkMatch,
        &'a mut FkAction,
        &'a mut FkAction,
    );
    /// FK alanları: (kolonlar, hedef şema, hedef tablo, hedef kolonlar, MATCH, ON UPDATE, ON DELETE).
    fn fk_mut(c: &mut ConstraintDef) -> FkParts<'_> {
        match &mut c.kind {
            ConstraintKind::ForeignKey { columns, ref_schema, ref_table, ref_columns, match_type, on_update, on_delete } => {
                (columns, ref_schema, ref_table, ref_columns, match_type, on_update, on_delete)
            }
            _ => panic!("FK değil"),
        }
    }
    fn cols_mut(c: &mut ConstraintDef) -> &mut Vec<String> {
        match &mut c.kind {
            ConstraintKind::PrimaryKey { columns, .. }
            | ConstraintKind::Unique { columns, .. }
            | ConstraintKind::ForeignKey { columns, .. } => columns,
            _ => panic!("kolon listesi yok"),
        }
    }
    fn include_mut(c: &mut ConstraintDef) -> &mut Vec<String> {
        match &mut c.kind {
            ConstraintKind::PrimaryKey { include, .. } | ConstraintKind::Unique { include, .. } => include,
            _ => panic!("INCLUDE yok"),
        }
    }
    fn expr_mut(c: &mut ConstraintDef) -> &mut String {
        match &mut c.kind {
            ConstraintKind::Check { expr, .. } => expr,
            _ => panic!("CHECK değil"),
        }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn clean(p: &Plan) {
        assert_single_statements(p);
        for s in &p.statements {
            let r = scan_semicolons(&s.sql);
            assert!(r.is_clean() && r.parens_balanced() && !r.ends_in_line_comment, "{}", s.sql);
            assert!(s.transactional, "{}", s.sql);
        }
    }
    fn ok(old: &TableDef, new: &TableDef) -> Plan {
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx())
            .unwrap_or_else(|e| panic!("reddedildi: {e:?}\n{new:?}"));
        clean(&p);
        p
    }
    fn errs(old: &TableDef, new: &TableDef) -> Vec<FieldError> {
        match balter(&o(old.clone()), &o(new.clone()), &ctx()) {
            Ok(p) => panic!("hata bekleniyordu:\n{}", render(&p)),
            Err(e) => e,
        }
    }

    #[test]
    fn noop_and_reordering_are_empty_plans() {
        let t = tbl();
        assert!(ok(&t, &t).statements.is_empty());
        // Kısıt listesinin sırası anlamsızdır (eşleme oid ile).
        let p = ok(&t, &with(&t, |d| d.constraints.reverse()));
        assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}", render(&p));
        // Yüklenmiş NOT VALID kısıt değişmeden kalır.
        let nv = with(&t, |d| {
            cm(d, "t_org_fkey").not_valid = true;
            cm(d, "t_qty_check").not_valid = true;
        });
        assert!(ok(&nv, &nv).statements.is_empty());
        // Kısıtsız tablo.
        assert!(ok(&bare(), &bare()).statements.is_empty());
    }

    /// Her kısıt türü için ekleme/silme/ad değişimi/yapısal değişim/VALIDATE: (etiket, eski, yeni, beklenen ifadeler).
    #[test]
    fn single_constraint_changes_exact_and_snapshot() {
        let t = tbl();
        let b = bare();
        let add = |c: ConstraintDef| with(&b, |d| d.constraints.push(c));
        let nv = |name: &str| with(&t, |d| cm(d, name).not_valid = true);
        let cases: Vec<(&str, TableDef, TableDef, Vec<&str>)> = vec![
            // --- ekleme
            ("PK ekle", b.clone(), add(con(None, Some("t_pkey"), pk(&["id"]))), vec!["ALTER TABLE public.t ADD CONSTRAINT t_pkey PRIMARY KEY (id)"]),
            (
                "PK adsız + INCLUDE",
                b.clone(),
                add(con(None, None, ConstraintKind::PrimaryKey { columns: s(&["id"]), include: s(&["code"]) })),
                vec!["ALTER TABLE public.t ADD PRIMARY KEY (id) INCLUDE (code)"],
            ),
            ("UNIQUE ekle", b.clone(), add(con(None, Some("u"), uq(&["code", "qty"], &[]))), vec!["ALTER TABLE public.t ADD CONSTRAINT u UNIQUE (code, qty)"]),
            (
                "UNIQUE NULLS NOT DISTINCT + INCLUDE",
                b.clone(),
                add(con(
                    None,
                    Some("u"),
                    ConstraintKind::Unique { columns: s(&["code"]), include: s(&["note"]), nulls_not_distinct: true },
                )),
                vec!["ALTER TABLE public.t ADD CONSTRAINT u UNIQUE NULLS NOT DISTINCT (code) INCLUDE (note)"],
            ),
            (
                "UNIQUE DEFERRABLE INITIALLY DEFERRED",
                b.clone(),
                add(ConstraintDef { deferrable: true, initially_deferred: true, ..con(None, Some("u"), uq(&["code"], &[])) }),
                vec!["ALTER TABLE public.t ADD CONSTRAINT u UNIQUE (code) DEFERRABLE INITIALLY DEFERRED"],
            ),
            ("CHECK ekle", b.clone(), add(con(None, Some("c"), ck("qty > 0"))), vec!["ALTER TABLE public.t ADD CONSTRAINT c CHECK (qty > 0\n)"]),
            ("CHECK adsız", b.clone(), add(con(None, None, ck("qty > 0"))), vec!["ALTER TABLE public.t ADD CHECK (qty > 0\n)"]),
            (
                "CHECK NO INHERIT NOT VALID",
                b.clone(),
                add(ConstraintDef {
                    not_valid: true,
                    ..con(None, Some("c"), ConstraintKind::Check { expr: "qty > 0".into(), no_inherit: true })
                }),
                vec!["ALTER TABLE public.t ADD CONSTRAINT c CHECK (qty > 0\n) NO INHERIT NOT VALID"],
            ),
            (
                "FK ekle",
                b.clone(),
                add(con(None, Some("f"), fk(&["org_id"], "org", &["id"]))),
                vec!["ALTER TABLE public.t ADD CONSTRAINT f FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION"],
            ),
            (
                "FK MATCH FULL + eylemler + DEFERRABLE + NOT VALID",
                b.clone(),
                add({
                    let mut c = ConstraintDef { deferrable: true, not_valid: true, ..con(None, Some("f"), fk(&["org_id"], "org", &["id"])) };
                    let (_, rs, _, _, m, u, dl) = fk_mut(&mut c);
                    *rs = "S 2".into();
                    *m = FkMatch::Full;
                    *u = FkAction::Cascade;
                    *dl = FkAction::SetNull;
                    c
                }),
                vec!["ALTER TABLE public.t ADD CONSTRAINT f FOREIGN KEY (org_id) REFERENCES \"S 2\".org (id) MATCH FULL ON UPDATE CASCADE ON DELETE SET NULL DEFERRABLE NOT VALID"],
            ),
            // --- silme
            ("UNIQUE sil", t.clone(), with(&t, |d| d.constraints.retain(|c| c.oid != Some(101))), vec!["ALTER TABLE public.t DROP CONSTRAINT t_code_key"]),
            ("CHECK sil", t.clone(), with(&t, |d| d.constraints.retain(|c| c.oid != Some(102))), vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check"]),
            ("FK sil", t.clone(), with(&t, |d| d.constraints.retain(|c| c.oid != Some(103))), vec!["ALTER TABLE public.t DROP CONSTRAINT t_org_fkey"]),
            (
                "PK + ona bağlı kendi FK'si sil (FK önce)",
                t.clone(),
                with(&t, |d| d.constraints.retain(|c| c.oid != Some(100) && c.oid != Some(104))),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_parent_fkey", "ALTER TABLE public.t DROP CONSTRAINT t_pkey"],
            ),
            // --- yalnızca ad
            ("PK ad", t.clone(), with(&t, |d| cm(d, "t_pkey").name = Some("t_pk".into())), vec!["ALTER TABLE public.t RENAME CONSTRAINT t_pkey TO t_pk"]),
            ("UNIQUE ad", t.clone(), with(&t, |d| cm(d, "t_code_key").name = Some("Kod".into())), vec!["ALTER TABLE public.t RENAME CONSTRAINT t_code_key TO \"Kod\""]),
            ("CHECK ad", t.clone(), with(&t, |d| cm(d, "t_qty_check").name = Some("qty_pos".into())), vec!["ALTER TABLE public.t RENAME CONSTRAINT t_qty_check TO qty_pos"]),
            ("FK ad", t.clone(), with(&t, |d| cm(d, "t_org_fkey").name = Some("org_ref".into())), vec!["ALTER TABLE public.t RENAME CONSTRAINT t_org_fkey TO org_ref"]),
            // --- yapısal: DROP (eski ad) + ADD (yeni tanım)
            (
                "PK kolonları",
                t.clone(),
                with(&t, |d| cols_mut(cm(d, "t_pkey")).push("code".into())),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_pkey", "ALTER TABLE public.t ADD CONSTRAINT t_pkey PRIMARY KEY (id, code)"],
            ),
            (
                "UNIQUE kolonları",
                t.clone(),
                with(&t, |d| cols_mut(cm(d, "t_code_key")).push("qty".into())),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_code_key", "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE (code, qty) INCLUDE (note)"],
            ),
            (
                "UNIQUE INCLUDE",
                t.clone(),
                with(&t, |d| include_mut(cm(d, "t_code_key")).clear()),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_code_key", "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE (code)"],
            ),
            (
                "UNIQUE NULLS NOT DISTINCT",
                t.clone(),
                with(&t, |d| {
                    if let ConstraintKind::Unique { nulls_not_distinct, .. } = &mut cm(d, "t_code_key").kind {
                        *nulls_not_distinct = true;
                    }
                }),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_code_key",
                    "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE NULLS NOT DISTINCT (code) INCLUDE (note)",
                ],
            ),
            (
                "UNIQUE DEFERRABLE",
                t.clone(),
                with(&t, |d| cm(d, "t_code_key").deferrable = true),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_code_key",
                    "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE (code) INCLUDE (note) DEFERRABLE",
                ],
            ),
            (
                "FK INITIALLY DEFERRED",
                with(&t, |d| cm(d, "t_org_fkey").deferrable = true),
                with(&t, |d| {
                    let c = cm(d, "t_org_fkey");
                    c.deferrable = true;
                    c.initially_deferred = true;
                }),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED",
                ],
            ),
            (
                "CHECK ifadesi",
                t.clone(),
                with(&t, |d| *expr_mut(cm(d, "t_qty_check")) = "qty > 0".into()),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty > 0\n)"],
            ),
            (
                "CHECK ifadesi yalnız boşluk farkı (metin eşitliği)",
                t.clone(),
                with(&t, |d| *expr_mut(cm(d, "t_qty_check")) = "qty>=0".into()),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty>=0\n)"],
            ),
            (
                "CHECK NO INHERIT",
                t.clone(),
                with(&t, |d| cm(d, "t_qty_check").kind = ConstraintKind::Check { expr: "qty >= 0".into(), no_inherit: true }),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty >= 0\n) NO INHERIT"],
            ),
            (
                "FK ON DELETE",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).6 = FkAction::Cascade),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE CASCADE",
                ],
            ),
            (
                "FK ON UPDATE",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).5 = FkAction::Restrict),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE RESTRICT ON DELETE NO ACTION",
                ],
            ),
            (
                "FK MATCH",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).4 = FkMatch::Full),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) MATCH FULL ON UPDATE NO ACTION ON DELETE NO ACTION",
                ],
            ),
            (
                "FK hedef şema",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).1 = "crm".into()),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES crm.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION",
                ],
            ),
            (
                "FK hedef tablo",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).2 = "Org".into()),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.\"Org\" (id) ON UPDATE NO ACTION ON DELETE NO ACTION",
                ],
            ),
            (
                "FK hedef kolonlar",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).3 = s(&["org_no"])),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (org_no) ON UPDATE NO ACTION ON DELETE NO ACTION",
                ],
            ),
            (
                "FK yerel kolonlar",
                t.clone(),
                with(&t, |d| *fk_mut(cm(d, "t_org_fkey")).0 = s(&["qty"])),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (qty) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION",
                ],
            ),
            (
                "tür değişimi (aynı oid: CHECK → UNIQUE)",
                t.clone(),
                with(&t, |d| cm(d, "t_qty_check").kind = uq(&["qty"], &[])),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check UNIQUE (qty)"],
            ),
            (
                "ad + yapısal: eski adla DROP, yeni adla ADD",
                t.clone(),
                with(&t, |d| {
                    let c = cm(d, "t_qty_check");
                    c.name = Some("qty_pos".into());
                    *expr_mut(c) = "qty > 0".into();
                }),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT qty_pos CHECK (qty > 0\n)"],
            ),
            (
                "yapısal + adı kaldırılmış: sunucu adlandırır",
                t.clone(),
                with(&t, |d| {
                    let c = cm(d, "t_qty_check");
                    c.name = None;
                    c.kind = ck("qty > 0");
                }),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CHECK (qty > 0\n)"],
            ),
            // --- doğrulama durumu
            ("FK VALIDATE", nv("t_org_fkey"), t.clone(), vec!["ALTER TABLE public.t VALIDATE CONSTRAINT t_org_fkey"]),
            ("CHECK VALIDATE", nv("t_qty_check"), t.clone(), vec!["ALTER TABLE public.t VALIDATE CONSTRAINT t_qty_check"]),
            (
                "ad + VALIDATE (yeni adla, ad değişiminden sonra)",
                nv("t_org_fkey"),
                with(&t, |d| cm(d, "t_org_fkey").name = Some("org_ref".into())),
                vec!["ALTER TABLE public.t RENAME CONSTRAINT t_org_fkey TO org_ref", "ALTER TABLE public.t VALIDATE CONSTRAINT org_ref"],
            ),
            (
                "doğrulanmamış + yapısal, NOT VALID kalır",
                nv("t_qty_check"),
                with(&nv("t_qty_check"), |d| *expr_mut(cm(d, "t_qty_check")) = "qty > 0".into()),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty > 0\n) NOT VALID"],
            ),
            (
                "doğrulanmamış + yapısal + doğrulanmış istenir",
                nv("t_qty_check"),
                with(&t, |d| *expr_mut(cm(d, "t_qty_check")) = "qty > 0".into()),
                vec!["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty > 0\n)"],
            ),
            (
                "doğrulanmış + yapısal + NOT VALID ile yeniden ekleme",
                t.clone(),
                with(&t, |d| {
                    let c = cm(d, "t_org_fkey");
                    c.not_valid = true;
                    *fk_mut(c).6 = FkAction::Cascade;
                }),
                vec![
                    "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                    "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE CASCADE NOT VALID",
                ],
            ),
        ];
        let mut out = String::new();
        for (caption, old, new, want) in cases {
            let p = ok(&old, &new);
            assert_eq!(sqls(&p), want, "{caption}");
            out.push_str(&format!("## {caption}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn not_valid_rules() {
        let t = tbl();
        // Doğrulanmış kısıt tekrar NOT VALID yapılamaz (yapısal değişim yoksa).
        let e = errs(&t, &with(&t, |d| cm(d, "t_org_fkey").not_valid = true));
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].path, "constraints[3].notValid");
        assert!(e[0].message.contains("NOT VALID"), "{e:?}");
        // Yalnızca ad değişimi yapısal değildir: yine hata.
        let e = errs(
            &t,
            &with(&t, |d| {
                let c = cm(d, "t_qty_check");
                c.name = Some("x".into());
                c.not_valid = true;
            }),
        );
        assert_eq!(e[0].path, "constraints[2].notValid");
        // PK/UNIQUE NOT VALID olamaz (mevcut ya da yeni).
        let e = errs(&t, &with(&t, |d| cm(d, "t_pkey").not_valid = true));
        assert_eq!(e[0].path, "constraints[0].notValid");
        let e = errs(&t, &with(&t, |d| d.constraints.push(ConstraintDef { not_valid: true, ..con(None, Some("u"), uq(&["qty"], &[])) })));
        assert_eq!(e[0].path, "constraints[5].notValid");
        // Yapısal değişimli UNIQUE'te de.
        let e = errs(
            &t,
            &with(&t, |d| {
                let c = cm(d, "t_code_key");
                c.not_valid = true;
                include_mut(c).clear();
            }),
        );
        assert_eq!(e[0].path, "constraints[1].notValid");
        // Değişmeyen mevcut kısıtın adı boşaltılamaz.
        let e = errs(&t, &with(&t, |d| cm(d, "t_qty_check").name = None));
        assert_eq!(e[0].path, "constraints[2].name");
    }

    #[test]
    fn column_rename_propagated_into_constraints_is_not_a_change() {
        let t = tbl();
        // Kolon adları değişir, arayüz kısıt kolon listelerine (PK, UNIQUE + INCLUDE, FK yerel, kendi FK'sinin hedefi)
        // yayar: yalnızca RENAME COLUMN (gereksiz DROP+ADD yok).
        let new = with(&t, |d| {
            colm(d, "id").name = "key".into();
            colm(d, "code").name = "kod".into();
            colm(d, "note").name = "memo".into();
            colm(d, "org_id").name = "organization_id".into();
            colm(d, "parent_id").name = "parent".into();
            *cols_mut(cm(d, "t_pkey")) = s(&["key"]);
            *cols_mut(cm(d, "t_code_key")) = s(&["kod"]);
            *include_mut(cm(d, "t_code_key")) = s(&["memo"]);
            *cols_mut(cm(d, "t_org_fkey")) = s(&["organization_id"]);
            let f = fk_mut(cm(d, "t_parent_fkey"));
            *f.0 = s(&["parent"]);
            *f.3 = s(&["key"]);
        });
        let p = ok(&t, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t RENAME COLUMN id TO \"key\"",
                "ALTER TABLE public.t RENAME COLUMN code TO kod",
                "ALTER TABLE public.t RENAME COLUMN parent_id TO parent",
                "ALTER TABLE public.t RENAME COLUMN org_id TO organization_id",
                "ALTER TABLE public.t RENAME COLUMN note TO memo",
            ]
        );
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        // Tablo adı/şeması değişir; kendine başvuran FK'nin hedefi yayılsa da yayılmasa da değişiklik değildir.
        for propagate in [false, true] {
            let new = with(&t, |d| {
                d.name = "t2".into();
                d.schema = "s2".into();
                if propagate {
                    let f = fk_mut(cm(d, "t_parent_fkey"));
                    *f.1 = "s2".into();
                    *f.2 = "t2".into();
                }
            });
            assert_eq!(
                sqls(&ok(&t, &new)),
                ["ALTER TABLE public.t RENAME TO t2", "ALTER TABLE public.t2 SET SCHEMA s2"],
                "{propagate}"
            );
        }
        // Kendine başvurmayan FK'nin hedefi doğrudan karşılaştırılır: tablo adı değişse de `org` aynı kalır.
        let p = ok(&t, &with(&t, |d| d.name = "org".into()));
        assert_eq!(sqls(&p), ["ALTER TABLE public.t RENAME TO org"]);
    }

    #[test]
    fn column_rename_without_propagation() {
        let t = tbl();
        // Kısıt eski (artık olmayan) kolon adını gösteriyor: doğrulama hatası.
        let e = errs(&t, &with(&t, |d| colm(d, "code").name = "kod".into()));
        assert_eq!(e[0].path, "constraints[1].columns[0]");
        let e = errs(&t, &with(&t, |d| colm(d, "note").name = "memo".into()));
        assert_eq!(e[0].path, "constraints[1].include[0]");
        let e = errs(&t, &with(&t, |d| colm(d, "org_id").name = "o".into()));
        assert_eq!(e[0].path, "constraints[3].columns[0]");
        // Eski ad yeni bir kolona verildi: kısıt artık YENİ kolonu gösterir → yapısal (DROP önce, ADD en sonda).
        let p = ok(
            &t,
            &with(&t, |d| {
                colm(d, "note").name = "memo".into();
                d.columns.push(col("note", "text"));
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t DROP CONSTRAINT t_code_key",
                "ALTER TABLE public.t RENAME COLUMN note TO memo",
                "ALTER TABLE public.t ADD COLUMN note text",
                "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE (code) INCLUDE (note)",
            ]
        );
    }

    #[test]
    fn column_drop_and_constraints() {
        let t = tbl();
        // Kolon silinir, ona bağlı kısıt da kaldırılır: kısıt DROP (3) kolon DROP'tan (4) önce.
        let p = ok(
            &t,
            &with(&t, |d| {
                d.columns.retain(|c| c.name != "org_id");
                d.constraints.retain(|c| c.name.as_deref() != Some("t_org_fkey"));
            }),
        );
        assert_eq!(sqls(&p), ["ALTER TABLE public.t DROP CONSTRAINT t_org_fkey", "ALTER TABLE public.t DROP COLUMN org_id"]);
        // Kısıt silinen kolonu göstermeye devam ediyor: hata (kolon listesi ve INCLUDE).
        let e = errs(&t, &with(&t, |d| d.columns.retain(|c| c.name != "org_id")));
        assert_eq!(e[0].path, "constraints[3].columns[0]");
        let e = errs(&t, &with(&t, |d| d.columns.retain(|c| c.name != "note")));
        assert_eq!(e[0].path, "constraints[1].include[0]");
        let two = with(&t, |d| cols_mut(cm(d, "t_code_key")).push("qty".into()));
        let e = errs(&two, &with(&two, |d| d.columns.retain(|c| c.name != "qty")));
        assert_eq!(e[0].path, "constraints[1].columns[1]");
        // Kısıt güncellenir (silinen kolon listeden çıkar): DROP → DROP COLUMN → ADD.
        let p = ok(
            &t,
            &with(&t, |d| {
                d.columns.retain(|c| c.name != "note");
                include_mut(cm(d, "t_code_key")).clear();
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t DROP CONSTRAINT t_code_key",
                "ALTER TABLE public.t DROP COLUMN note",
                "ALTER TABLE public.t ADD CONSTRAINT t_code_key UNIQUE (code)",
            ]
        );
    }

    #[test]
    fn type_change_and_constraints() {
        let t = tbl();
        // Tip değişimi + yapısal kısıt değişimi: DROP CONSTRAINT → TYPE → ADD CONSTRAINT.
        let p = ok(
            &t,
            &with(&t, |d| {
                colm(d, "qty").data_type = "bigint".into();
                *expr_mut(cm(d, "t_qty_check")) = "qty > 0".into();
                colm(d, "org_id").data_type = "bigint".into();
                *fk_mut(cm(d, "t_org_fkey")).6 = FkAction::SetNull;
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t DROP CONSTRAINT t_org_fkey",
                "ALTER TABLE public.t DROP CONSTRAINT t_qty_check",
                "ALTER TABLE public.t ALTER COLUMN qty TYPE bigint",
                "ALTER TABLE public.t ALTER COLUMN org_id TYPE bigint",
                "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty > 0\n)",
                "ALTER TABLE public.t ADD CONSTRAINT t_org_fkey FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE SET NULL",
            ]
        );
        assert_eq!(p.warnings, [ADD_CONSTRAINT_WARNING, REWRITE_WARNING, LOCK_TIMEOUT_WARNING]);
        // Değişmeyen kısıtın kolon tipi değişir: kısıt yerinde kalır (PG yeniden doğrular/indeksi yeniden kurar) + uyarı.
        let p = ok(
            &t,
            &with(&t, |d| {
                colm(d, "code").data_type = "character varying(20)".into();
                colm(d, "org_id").data_type = "bigint".into();
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t ALTER COLUMN code TYPE character varying(20)",
                "ALTER TABLE public.t ALTER COLUMN org_id TYPE bigint",
            ]
        );
        assert_eq!(p.warnings.len(), 4, "{:?}", p.warnings);
        assert!(p.warnings[0].contains("'t_code_key'"), "{:?}", p.warnings);
        assert!(p.warnings[1].contains("'t_org_fkey'"), "{:?}", p.warnings);
        assert_eq!(p.warnings[2..], [REWRITE_WARNING, LOCK_TIMEOUT_WARNING]);
        // INCLUDE kolonu ve kolonu yeniden adlandırılan kısıt da (uyarı yeni kısıt adıyla).
        let p = ok(
            &t,
            &with(&t, |d| {
                let c = colm(d, "note");
                c.data_type = "character varying(10)".into();
                c.name = "memo".into();
                *include_mut(cm(d, "t_code_key")) = s(&["memo"]);
                cm(d, "t_code_key").name = Some("kod_key".into());
            }),
        );
        assert!(p.warnings[0].contains("'kod_key'"), "{:?}", p.warnings);
        // Takma adla "tip değişimi" değişim değildir: uyarı yok.
        let p = ok(&t, &with(&t, |d| colm(d, "code").data_type = "TEXT".into()));
        assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}", render(&p));
    }

    #[test]
    fn primary_key_change_warns_about_dependent_foreign_keys() {
        // Kendine başvuran FK'siz tablo (yalnızca PK).
        let t = with(&tbl(), |d| d.constraints.retain(|c| c.oid == Some(100)));
        // Yapısal değişim.
        let p = ok(&t, &with(&t, |d| cols_mut(cm(d, "t_pkey")).push("code".into())));
        assert_eq!(
            sqls(&p),
            ["ALTER TABLE public.t DROP CONSTRAINT t_pkey", "ALTER TABLE public.t ADD CONSTRAINT t_pkey PRIMARY KEY (id, code)"]
        );
        assert_eq!(p.statements[0].risk, Risk::Destructive);
        assert_eq!(p.statements[0].note.as_deref(), Some(DROP_CONSTRAINT_NOTE));
        assert_eq!(p.statements[1].risk, Risk::Safe);
        assert_eq!(p.statements[1].note.as_deref(), Some(ADD_CONSTRAINT_NOTE));
        assert_eq!(p.warnings, [PK_DROP_WARNING, ADD_CONSTRAINT_WARNING, LOCK_TIMEOUT_WARNING]);
        assert_eq!(p.risk(), Risk::Destructive);
        // Eski PK silinip başka kolona yeni PK (oid'siz yeni kısıt).
        let p = ok(&t, &with(&t, |d| d.constraints = vec![con(None, Some("t_code_pk"), pk(&["code"]))]));
        assert_eq!(
            sqls(&p),
            ["ALTER TABLE public.t DROP CONSTRAINT t_pkey", "ALTER TABLE public.t ADD CONSTRAINT t_code_pk PRIMARY KEY (code)"]
        );
        assert_eq!(p.warnings[0], PK_DROP_WARNING);
        // PK yalnızca yeniden adlandırılırsa uyarı yok.
        let p = ok(&t, &with(&t, |d| cm(d, "t_pkey").name = Some("pk".into())));
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        // UNIQUE silme PK uyarısı vermez (DROP notu bağımlı FK'leri anlatır).
        let p = ok(&tbl(), &with(&tbl(), |d| d.constraints.retain(|c| c.oid != Some(101))));
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn drops_put_foreign_keys_first_and_adds_put_them_last() {
        let t = tbl();
        // Kendine başvuran FK ve hedefi PK birlikte yeniden kurulur: FK önce düşer, PK önce eklenir.
        let new = with(&t, |d| {
            cols_mut(cm(d, "t_pkey")).push("code".into());
            let f = fk_mut(cm(d, "t_parent_fkey"));
            *f.0 = s(&["parent_id", "qty"]);
            *f.3 = s(&["id", "code"]);
            // Yeni FK listenin başında, yeni CHECK sonunda.
            d.constraints.insert(0, con(None, Some("f_new"), fk(&["org_id"], "org", &["id"])));
            d.constraints.push(con(None, Some("c_new"), ck("qty < 10")));
        });
        assert_eq!(
            sqls(&ok(&t, &new)),
            [
                "ALTER TABLE public.t DROP CONSTRAINT t_parent_fkey",
                "ALTER TABLE public.t DROP CONSTRAINT t_pkey",
                "ALTER TABLE public.t ADD CONSTRAINT t_pkey PRIMARY KEY (id, code)",
                "ALTER TABLE public.t ADD CONSTRAINT c_new CHECK (qty < 10\n)",
                "ALTER TABLE public.t ADD CONSTRAINT f_new FOREIGN KEY (org_id) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION",
                "ALTER TABLE public.t ADD CONSTRAINT t_parent_fkey FOREIGN KEY (parent_id, qty) REFERENCES public.t (id, code) ON UPDATE NO ACTION ON DELETE NO ACTION",
            ]
        );
    }

    #[test]
    fn constraint_name_reuse_chains_and_cycles() {
        let t = tbl();
        // Zincir: t_qty_check → x, t_code_key → t_qty_check (hedef adı önce boşalır).
        let p = ok(
            &t,
            &with(&t, |d| {
                cm(d, "t_qty_check").name = Some("x".into());
                cm(d, "t_code_key").name = Some("t_qty_check".into());
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t RENAME CONSTRAINT t_qty_check TO x",
                "ALTER TABLE public.t RENAME CONSTRAINT t_code_key TO t_qty_check",
            ]
        );
        // Takas: döngü → doğrulama hatası.
        let e = errs(
            &t,
            &with(&t, |d| {
                cm(d, "t_qty_check").name = Some("tmp".into());
                cm(d, "t_code_key").name = Some("t_qty_check".into());
                cm(d, "tmp").name = Some("t_code_key".into());
            }),
        );
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].path.starts_with("constraints[") && e[0].path.ends_with("].name"), "{e:?}");
        assert!(e[0].message.contains("döngüsel"), "{e:?}");
        // Silinen kısıtın adıyla yeni kısıt: DROP (3) önce, ADD (10) sonra.
        let p = ok(
            &t,
            &with(&t, |d| {
                d.constraints.retain(|c| c.oid != Some(102));
                d.constraints.push(con(None, Some("t_qty_check"), ck("qty > 0")));
            }),
        );
        assert_eq!(
            sqls(&p),
            ["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty > 0\n)"]
        );
        // Yeniden adlandırılan kısıtın eski adıyla yeni kısıt: RENAME (9) önce, ADD (10) sonra.
        let p = ok(
            &t,
            &with(&t, |d| {
                cm(d, "t_qty_check").name = Some("old_check".into());
                d.constraints.push(con(None, Some("t_qty_check"), ck("qty < 100")));
            }),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t RENAME CONSTRAINT t_qty_check TO old_check",
                "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty < 100\n)",
            ]
        );
    }

    #[test]
    fn oid_matching_errors() {
        let t = tbl();
        // Bayat oid: eski tanımda yok.
        let e = errs(&t, &with(&t, |d| d.constraints[2].oid = Some(999)));
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].path, "constraints[2].oid");
        assert!(e[0].message.contains("999") && e[0].message.contains("bayat"), "{e:?}");
        // Yinelenen oid (yeni taraf).
        let e = errs(&t, &with(&t, |d| d.constraints[3].oid = Some(101)));
        assert_eq!(e[0].path, "constraints[3].oid");
        // Eski taraf: oid yok / yinelenen / ad yok / ad geçersiz.
        for (old, path) in [
            (with(&t, |d| d.constraints[1].oid = None), "old.constraints[1].oid"),
            (with(&t, |d| d.constraints[2].oid = Some(100)), "old.constraints[2].oid"),
            (with(&t, |d| d.constraints[3].name = None), "old.constraints[3].name"),
            (with(&t, |d| d.constraints[3].name = Some("a\nb".into())), "old.constraints[3].name"),
            (with(&t, |d| d.constraints[3].name = Some(String::new())), "old.constraints[3].name"),
        ] {
            let e = errs(&old, &bare());
            assert_eq!(e[0].path, path, "{e:?}");
        }
        // Eski tarafın kısıt gövdesi SQL'e girmez: create-geçersiz eski CHECK ifadesi kabul (yalnızca silinir).
        let weird = with(&t, |d| *expr_mut(&mut d.constraints[2]) = "x'".into());
        assert_eq!(
            sqls(&ok(&weird, &with(&t, |d| d.constraints.retain(|c| c.oid != Some(102))))),
            ["ALTER TABLE public.t DROP CONSTRAINT t_qty_check"]
        );
        // Aynı adla sil + yeniden ekle (oid None): DROP + ADD (değişmemiş sayılmaz).
        let p = ok(&t, &with(&t, |d| d.constraints[2].oid = None));
        assert_eq!(
            sqls(&p),
            ["ALTER TABLE public.t DROP CONSTRAINT t_qty_check", "ALTER TABLE public.t ADD CONSTRAINT t_qty_check CHECK (qty >= 0\n)"]
        );
    }

    #[test]
    fn partition_and_inheritance_protect_constraints() {
        for flag in [0, 1] {
            let set = |d: &mut TableDef| {
                if flag == 0 {
                    d.is_partition = true
                } else {
                    d.inherits = true
                }
            };
            let old = with(&tbl(), set);
            let old_nv = with(&old, |d| cm(d, "t_org_fkey").not_valid = true);
            for (caption, from, new) in [
                ("ad", &old, with(&old, |d| cm(d, "t_pkey").name = Some("pk".into()))),
                ("silme", &old, with(&old, |d| d.constraints.truncate(4))),
                ("ekleme", &old, with(&old, |d| d.constraints.push(con(None, Some("c"), ck("qty > 0"))))),
                ("yapısal", &old, with(&old, |d| *expr_mut(cm(d, "t_qty_check")) = "qty > 0".into())),
                ("VALIDATE", &old_nv, old.clone()),
            ] {
                let e = errs(from, &new);
                assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("constraints", MANAGED), "{flag} {caption}");
            }
            // Sıra farkı değişiklik değildir.
            let p = ok(&old, &with(&old, |d| d.constraints.reverse()));
            assert!(p.statements.is_empty(), "{}", render(&p));
        }
    }

    /// Global Constraints'teki tam plan sırası (1–11) tek planda.
    #[test]
    fn full_plan_order() {
        let old = with(&tbl(), |d| {
            d.constraints.push(ConstraintDef { not_valid: true, ..con(Some(105), Some("t_pos_check"), ck("parent_id > 0")) });
        });
        let new = with(&old, |d| {
            d.name = "t2".into();
            d.schema = "s2".into();
            d.owner = Some("bob".into());
            d.comment = Some("Yeni".into());
            // (3) silinen kısıt + yapısal değişen kısıtlar.
            d.constraints.retain(|c| c.oid != Some(104));
            include_mut(cm(d, "t_code_key")).clear();
            {
                let c = cm(d, "t_qty_check");
                c.name = Some("t_qty_pos".into());
                *expr_mut(c) = "qty > 0".into();
            }
            // (4) kolon silme (INCLUDE'dan çıktı), (5) tip, (6) kolon adı (FK'ye yayıldı), (7) ekleme, (8) NOT NULL.
            d.columns.retain(|c| c.name != "note");
            colm(d, "qty").data_type = "bigint".into();
            colm(d, "org_id").name = "organization_id".into();
            *cols_mut(cm(d, "t_org_fkey")) = s(&["organization_id"]);
            d.columns.push(col("extra", "integer"));
            colm(d, "parent_id").not_null = true;
            // (9) kısıt adı, (10) ekleme (+ NOT VALID) ve VALIDATE.
            cm(d, "t_org_fkey").name = Some("t_org_fk".into());
            cm(d, "t_pos_check").not_valid = false;
            d.constraints.push(ConstraintDef { not_valid: true, ..con(None, Some("t_extra_check"), ck("extra > 0")) });
            d.constraints.push(con(None, Some("t_extra_fk"), fk(&["extra"], "org", &["id"])));
        });
        let p = ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t RENAME TO t2",
                "ALTER TABLE public.t2 SET SCHEMA s2",
                "ALTER TABLE s2.t2 DROP CONSTRAINT t_parent_fkey",
                "ALTER TABLE s2.t2 DROP CONSTRAINT t_code_key",
                "ALTER TABLE s2.t2 DROP CONSTRAINT t_qty_check",
                "ALTER TABLE s2.t2 DROP COLUMN note",
                "ALTER TABLE s2.t2 ALTER COLUMN qty TYPE bigint",
                "ALTER TABLE s2.t2 RENAME COLUMN org_id TO organization_id",
                "ALTER TABLE s2.t2 ADD COLUMN extra integer",
                "ALTER TABLE s2.t2 ALTER COLUMN parent_id SET NOT NULL",
                "ALTER TABLE s2.t2 RENAME CONSTRAINT t_org_fkey TO t_org_fk",
                "ALTER TABLE s2.t2 ADD CONSTRAINT t_code_key UNIQUE (code)",
                "ALTER TABLE s2.t2 ADD CONSTRAINT t_qty_pos CHECK (qty > 0\n)",
                "ALTER TABLE s2.t2 ADD CONSTRAINT t_extra_check CHECK (extra > 0\n) NOT VALID",
                "ALTER TABLE s2.t2 ADD CONSTRAINT t_extra_fk FOREIGN KEY (extra) REFERENCES public.org (id) ON UPDATE NO ACTION ON DELETE NO ACTION",
                "ALTER TABLE s2.t2 VALIDATE CONSTRAINT t_pos_check",
                "ALTER TABLE s2.t2 OWNER TO bob",
                "COMMENT ON TABLE s2.t2 IS 'Yeni'",
            ]
        );
        assert_eq!(p.risk(), Risk::DataLoss);
        // Deterministik.
        assert_eq!(ok(&old, &new), p);
        insta::assert_snapshot!(render(&p));
    }

    /// Kısıt ifade türlerinin riskleri ve notları.
    #[test]
    fn risks_and_notes() {
        let t = tbl();
        let nv = with(&t, |d| cm(d, "t_org_fkey").not_valid = true);
        let one = |old: &TableDef, new: &TableDef| {
            let p = ok(old, new);
            assert_eq!(p.statements.len(), 1, "{}", render(&p));
            p.statements[0].clone()
        };
        let st = one(&t, &with(&t, |d| d.constraints.retain(|c| c.oid != Some(102))));
        assert_eq!((st.risk, st.note.as_deref()), (Risk::Destructive, Some(DROP_CONSTRAINT_NOTE)));
        let st = one(&t, &with(&t, |d| cm(d, "t_qty_check").name = Some("x".into())));
        assert_eq!((st.risk, st.note.as_deref()), (Risk::Safe, Some(RENAME_CONSTRAINT_NOTE)));
        let st = one(&t, &with(&t, |d| cm(d, "t_pkey").name = Some("x".into())));
        assert_eq!((st.risk, st.note.as_deref()), (Risk::Safe, Some(RENAME_INDEX_CONSTRAINT_NOTE)));
        let st = one(&t, &with(&t, |d| cm(d, "t_code_key").name = Some("x".into())));
        assert_eq!(st.note.as_deref(), Some(RENAME_INDEX_CONSTRAINT_NOTE));
        let st = one(&nv, &t);
        assert_eq!((st.risk, st.note.as_deref()), (Risk::Safe, Some(VALIDATE_NOTE)));
        let add = |c: ConstraintDef| with(&t, |d| d.constraints.push(c));
        for (c, note, warn) in [
            (con(None, Some("c"), ck("qty > 0")), ADD_CONSTRAINT_NOTE, true),
            (con(None, Some("u"), uq(&["qty"], &[])), ADD_CONSTRAINT_NOTE, true),
            (con(None, Some("f"), fk(&["qty"], "org", &["id"])), ADD_FK_NOTE, true),
            (ConstraintDef { not_valid: true, ..con(None, Some("c"), ck("qty > 0")) }, ADD_NOT_VALID_NOTE, false),
            (ConstraintDef { not_valid: true, ..con(None, Some("f"), fk(&["qty"], "org", &["id"])) }, ADD_NOT_VALID_NOTE, false),
        ] {
            let p = ok(&t, &add(c));
            assert_eq!(p.statements.len(), 1);
            assert_eq!((p.statements[0].risk, p.statements[0].note.as_deref()), (Risk::Safe, Some(note)));
            if warn {
                assert_eq!(p.warnings, [ADD_CONSTRAINT_WARNING, LOCK_TIMEOUT_WARNING]);
            } else {
                assert!(p.warnings.is_empty(), "{:?}", p.warnings);
            }
        }
        // VALIDATE ve RENAME CONSTRAINT plan uyarısı üretmez.
        assert!(ok(&nv, &t).warnings.is_empty());
        // Kolon sırası uyarısı kısıt farkından bağımsızdır.
        let p = ok(&t, &with(&t, |d| d.columns.swap(4, 5)));
        assert_eq!(p.warnings, [COLUMN_ORDER_WARNING]);
    }

    #[test]
    fn composed_check_statement_is_scanned() {
        let t = tbl();
        let p = ok(&t, &with(&t, |d| d.constraints.push(con(None, Some("c"), ck("qty -- yorum\n> 0 /* ) */")))));
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD CONSTRAINT c CHECK (qty -- yorum\n> 0 /* ) */\n)"]);
        // Alan kapısı: dengesiz parantez, dış `;`, `--` ile biten ifade, boş.
        for bad in ["qty > 0) OR (true", "qty > 0; DROP TABLE t", "qty > 0 --", ""] {
            let e = errs(&t, &with(&t, |d| *expr_mut(cm(d, "t_qty_check")) = bad.into()));
            assert_eq!(e[0].path, "constraints[2].expr", "{bad:?}");
        }
    }

    /// Saldırgan girdi matrisi: alter'ın HER kısıt metin alanında girdi ya o alanda reddedilir ya da plan tek ifadeli,
    /// temiz ve dengeli kalır.
    #[test]
    fn hostile_input_matrix() {
        #[derive(Clone, Copy, PartialEq)]
        enum K {
            Name,
            Free,
        }
        type Set = fn(&mut TableDef, String);
        let old = tbl();
        // Temel yeni tanım: sona yeni bir CHECK eklendi (konum 5).
        let new_base = with(&old, |d| d.constraints.push(con(None, Some("c_new"), ck("qty < 100"))));
        let fields: Vec<(&str, K, &str, Set)> = vec![
            ("constraints[1].name", K::Name, "uq2", |d, v| d.constraints[1].name = Some(v)),
            ("constraints[5].name", K::Name, "c_new2", |d, v| d.constraints[5].name = Some(v)),
            ("constraints[2].expr", K::Free, "qty > 1", |d, v| *expr_mut(&mut d.constraints[2]) = v),
            ("constraints[5].expr", K::Free, "qty < 99", |d, v| *expr_mut(&mut d.constraints[5]) = v),
            ("constraints[3].refSchema", K::Name, "crm", |d, v| *fk_mut(&mut d.constraints[3]).1 = v),
            ("constraints[3].refTable", K::Name, "org2", |d, v| *fk_mut(&mut d.constraints[3]).2 = v),
            ("constraints[3].refColumns[0]", K::Name, "org_no", |d, v| fk_mut(&mut d.constraints[3]).3[0] = v),
            ("constraints[3].columns[0]", K::Name, "qty", |d, v| fk_mut(&mut d.constraints[3]).0[0] = v),
            ("constraints[1].columns[0]", K::Name, "qty", |d, v| cols_mut(&mut d.constraints[1])[0] = v),
            ("constraints[1].include[0]", K::Name, "qty", |d, v| include_mut(&mut d.constraints[1])[0] = v),
            ("constraints[0].columns[0]", K::Name, "code", |d, v| cols_mut(&mut d.constraints[0])[0] = v),
        ];
        let hostile: Vec<String> = [
            "x\"; DROP TABLE t; --",
            "x'; DROP TABLE t; --",
            "x); DROP TABLE t; --",
            "x\n); DROP TABLE t; --",
            "x\r\n) --",
            "x\\",
            "\\'; DROP TABLE t; --",
            "--",
            "a --",
            "a -- yorum",
            "/*",
            "*/",
            "a /* b",
            "$$",
            "$a$ x",
            "x$$ ; DROP TABLE t; $$",
            "1$$'$$ ) ; DROP TABLE t; --'",
            "1'x'",
            "'",
            "\"",
            "\"\"",
            "(",
            ")",
            ";",
            "a; ",
            "E'\\'; DROP TABLE t; --'",
            "\0",
            "a\0b",
            "\n",
            "a\nb",
            "\t",
            "\x1b",
            "\x7f",
            "",
            " ",
            "select",
            "user",
            "order",
            "Ünïcödé ş",
            "\u{202e}x",
            "\u{85}",
            "pg_catalog",
            "information_schema",
            "qty) NOT VALID",
            "true) NO INHERIT; --",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain(["a".repeat(63), "a".repeat(64), "ş".repeat(32)])
        .collect();
        let mut accepted = 0;
        for (path, kind, benign, set) in &fields {
            let mut b = new_base.clone();
            set(&mut b, benign.to_string());
            let base_len = ok(&old, &b).statements.len();
            for h in &hostile {
                let mut d = new_base.clone();
                set(&mut d, h.clone());
                let caption = format!("{path} ← {h:?}");
                match balter(&o(old.clone()), &o(d), &ctx()) {
                    Ok(p) => {
                        accepted += 1;
                        clean(&p);
                        assert_eq!(p.statements.len(), base_len, "{caption}\n{}", render(&p));
                        assert!(!h.contains('\0'), "{caption}");
                        match kind {
                            K::Name => assert!(
                                !h.is_empty()
                                    && h.len() <= 63
                                    && !h.chars().any(|c| c.is_ascii_control() || is_spoofing_format(c)),
                                "{caption}"
                            ),
                            K::Free => {
                                let r = scan_semicolons(h);
                                assert!(
                                    r.is_clean() && r.parens_balanced() && !r.ends_in_line_comment && !h.trim().is_empty(),
                                    "{caption}"
                                );
                            }
                        }
                    }
                    Err(e) => assert_eq!(e[0].path, *path, "{caption}: {e:?}"),
                }
            }
        }
        assert!(accepted > 40, "{accepted}");
    }
}
