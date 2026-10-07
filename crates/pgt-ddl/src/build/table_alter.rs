//! Tablo alter'ı: kimlik (`attnum`) eşlemeli kolon farkı, `oid` eşlemeli kısıt farkı (`table_constraint.rs`) ve
//! tablo düzeyi değişiklikler.
//!
//! Kararlar:
//! - Eski tanım yalnızca SQL'e giren kimlik alanları için doğrulanır (şema, ad, kolon ve kısıt adları) + eşleme
//!   anahtarları `attnum`/`oid`; yüklenmiş tanımın create-geçerliliği gerekmez (eski tip/ifade/varsayılan/kısıt
//!   gövdesi hiç SQL'e girmez).
//!   Yeni tanım `validate_table` ile tam doğrulanır (create-only kurallar hariç).
//! - İfade sırası sabittir (Global Constraints): (1) tablo `RENAME TO`, (2) `SET SCHEMA`, (3) silinen/yapısal değişen
//!   kısıtların `DROP CONSTRAINT`'i (eski kısıt adıyla; PG kolon silince/tip değiştirince bağlı kısıtı kendiliğinden
//!   düşürebilir ya da hata verir — bu yüzden önce), (4) `DROP COLUMN`, (5) `ALTER COLUMN … TYPE`, (6) `RENAME COLUMN`,
//!   (7) `ADD COLUMN`, (8) nullability/varsayılan/kimlik/üretilmiş ifade, (9) `RENAME CONSTRAINT` (eski → yeni ad),
//!   (10) `ADD CONSTRAINT` (yeni tanım, yeni kolon adları) ve ardından `VALIDATE CONSTRAINT` (yeni adla), (11) sahip,
//!   tablespace, kalıcılık, fillfactor, yorumlar. Adım 2'den sonra her ifade YENİ nitelikli tablo adını kullanır.
//!   `TYPE` (5) yeniden adlandırmadan (6) önce çalıştığından ESKİ kolon adını (USING ifadesi de eski adları görür),
//!   adım 8 ve sonrası YENİ kolon adını kullanır.
//! - Değişmeyen kısıtın kolonunun tipi değişirse kısıt yerinde kalır (PG tip değişiminde kısıtı yeniden doğrular,
//!   PK/UNIQUE indeksini yeniden kurar); plana uyarı eklenir.
//! - Tip değişen kolonun eski varsayılanı varsa adım 5'te `TYPE`'tan hemen önce `DROP DEFAULT` üretilir (PG eski
//!   varsayılanı yeni tipe dönüştürmeye çalışır ve çoğu zaman başarısız olur); yeni varsayılan varsa adım 8'de metni
//!   eskisiyle aynı olsa bile `SET DEFAULT` ile yeniden kurulur.
//! - "Tip değişti mi" kararı `canonical_type` ile verilir (takma ad `varchar(20)` = `character varying(20)`);
//!   üretilen SQL kullanıcının tip metnini kullanır.
//! - Adım 8'de her kolonun ifadeleri birlikte, şu sırayla üretilir: `DROP IDENTITY`, `DROP EXPRESSION`,
//!   `SET|DROP DEFAULT`, `SET|DROP NOT NULL`, `ADD GENERATED … AS IDENTITY` / `SET GENERATED`, `SET EXPRESSION`.
//!   Gerekçe: PG kimlik eklemeden önce kolonun NOT NULL olmasını ve varsayılansız olmasını ister; kimlikli kolonda
//!   `DROP NOT NULL` ve `SET DEFAULT` reddedilir (önce `DROP IDENTITY`).
//! - Serbest SQL alanları (`default`, `generated.expr`, `using`) `validate_table` içinde `check_free_sql`'den geçer,
//!   `wrap` ile yerleşir ve serbest SQL içeren her NİHAİ ifade (CHECK'li `ADD CONSTRAINT` dahil tüm kısıt
//!   eklemeleri) ayrıca `composed_is_clean` ile taranır.
//! - Kolon sırası değiştirilemez: fark yalnızca ekleme/silme/değişim üzerinden üretilir, sıra farkı uyarıdır.
//! - Gerçek sunucularda (PG 10–18, Task 5) doğrulanan kurallar yerel hatadır: bölümlenmiş tabloda (yüklenmişte
//!   `partition_text` dolu) kalıcılık değişimi (PG 10–17 sessizce yoksayar, PG 18 reddeder), PG 12 öncesi
//!   `SET TABLESPACE` (yoksayılır), fillfactor ve PG 18 öncesi `NOT VALID` FK; PG 18'de CHECK'li tabloda sanal kolonun
//!   `SET EXPRESSION`'ı. Üretilmiş kolonun ifadesinde geçen kolonun tip değişimi (sunucu reddeder) yalnızca uyarıdır:
//!   ifadenin kullandığı kolonlar yerelde kesin bilinemez.

use std::collections::{HashMap, HashSet, VecDeque};

use super::freesql::{composed_is_clean, wrap};
use super::table::{
    column_comment_sql, column_sql, has_comment, is_partitioned, table_comment_sql, validate_table, version_err,
};
use super::table_constraint::{
    ADD_CONSTRAINT_WARNING, ConstraintDiff, DROP_CONSTRAINT_NOTE, PK_DROP_WARNING, VALIDATE_NOTE, add_note,
    add_path, add_sql, check_new_oids, constraint_columns, diff as constraint_diff, drop_sql, is_pk, rename_note,
    rename_sql, validate_old_constraints, validate_sql,
};
use super::type_canon::canonical_type;
use super::freesql::check_free_sql;
use super::type_syntax::validate_type;
use super::{BuildCtx, check_old_ident, err, stmt};
use crate::load::is_unsupported_check;
use crate::model::{ColumnDef, ConstraintKind, FieldError, IdentityKind, Persistence, TableDef};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident};

const RENAME_TABLE_NOTE: &str = "Tabloya adla başvuran fonksiyonlar ve uygulama sorguları etkilenir";
const SET_SCHEMA_NOTE: &str =
    "Tablonun indeksleri, kısıtları ve kolonlarına bağlı diziler de taşınır; adla başvuran bağımlılar etkilenir";
const DROP_COLUMN_NOTE: &str = "Kolondaki veri kalıcı silinir";
const TYPE_NOTE: &str = "Tabloyu yeniden yazar; bağlı indeks/kısıtlar yeniden oluşturulur";
const TYPE_USING_HINT: &str = "; otomatik dönüşüm yoksa USING gerekir (USING ifadesi eski kolon adlarını kullanır)";
const TYPE_USING_NOTE: &str = "; USING ifadesi eski kolon adlarını kullanır";
const TYPE_DROP_DEFAULT_NOTE: &str =
    "Tip değişiminden önce varsayılan kaldırılır (PG eski varsayılanı yeni tipe dönüştüremeyebilir); yeni varsayılan sonra kurulur";
const RENAME_COLUMN_NOTE: &str = "Kolona adla başvuran fonksiyonlar ve uygulama sorguları etkilenir";
const ADD_NOT_NULL_NOTE: &str = "Mevcut satırlar varsa başarısız olur";
const ADD_GENERATED_NOT_NULL_NOTE: &str = "İfade mevcut bir satırda NULL üretirse başarısız olur";
const SET_NOT_NULL_NOTE: &str = "Tabloyu tarar; NULL değer içeren satır varsa başarısız olur";
const ADD_IDENTITY_NOTE: &str = "Kimlik kolonu NOT NULL olmalıdır; değerler kolona bağlı yeni bir diziden üretilir; yeni dizi 1'den başlar ve mevcut değerlerle çakışabilir (RESTART WITH gerekebilir)";
const DROP_IDENTITY_NOTE: &str = "Kimliğe bağlı dizi silinir; mevcut değerler korunur";
const DROP_EXPRESSION_NOTE: &str = "Kolon sıradan kolona dönüşür; mevcut değerler korunur";
const SET_EXPRESSION_NOTE: &str = "Tüm satırlar için değer yeni ifadeyle yeniden hesaplanır";
const REWRITE_NOTE: &str = "Tabloyu yeniden yazar; ACCESS EXCLUSIVE kilidi alır";
const FILLFACTOR_NOTE: &str = "Yalnızca yeni yazılan sayfaları etkiler; mevcut sayfalar için VACUUM FULL gerekir";

pub(crate) const REWRITE_WARNING: &str = "Tablo yeniden yazılır: işlem boyunca ACCESS EXCLUSIVE kilidi alınır ve tablo okuma ve yazmaya kapalı kalır; büyük tablolarda uzun sürer";
pub(crate) const LOCK_TIMEOUT_WARNING: &str = "Uzun süren ALTER TABLE kilit beklerken tablodaki diğer sorguları da bekletir; uygulamadan önce oturumda lock_timeout ayarlamayı düşünün (ör. SET lock_timeout = '5s')";
pub(crate) const COLUMN_ORDER_WARNING: &str = "Kolon sırası değiştirilemez; yeni sıra yoksayılır (mevcut kolonların sırası korunur, yeni kolonlar sona eklenir)";
const MANAGED_MSG: &str = "Bölüm/kalıtımlı tablonun kolon ve kısıtları ana tablodan yönetilir.";
const COMPOSED_MSG: &str =
    "Serbest SQL alanları birleşince geçersiz ifade oluşuyor (ör. satır yorumuyla biten ifade).";

const LOAD_FLAG_MSG: &str = "Bu alan yüklemeden gelir ve değiştirilemez; tanımı yeniden yükleyin.";
const PARTITION_BY_MSG: &str =
    "Bölümleme anahtarı sonradan değiştirilemez; yalnızca tablo oluşturulurken belirlenir.";
const PARTITIONED_ADD_IDENTITY_MSG: &str = "Bölümlü/kalıtımlı tabloya mevcut alt tablolar varken kimlik kolonu eklenemez (PG <17). Bölüm olup olmadığı burada bilinmediği için bölümlenmiş tabloda her zaman reddedilir; kolonu kimliksiz ekleyip ardından kimlik ekleyin (ADD GENERATED … AS IDENTITY) ya da PG 17+ kullanın.";
const PARTITIONED_PERSISTENCE_MSG: &str = "Bölümlenmiş tablonun kalıcılığı (LOGGED/UNLOGGED) değiştirilemez; bölümleri ayrı ayrı değiştirin.";
pub(crate) const TABLESPACE_CLEAR_MSG: &str =
    "Tablespace kaldırılamaz; varsayılana dönmek için pg_default'u açıkça seçin.";

/// Yeni tanımdaki her kolon: konumu, eski karşılığı (korunan) ya da `None` (eklenen).
type Matched<'a> = Vec<(usize, Option<&'a ColumnDef>, &'a ColumnDef)>;

/// Tablo alter'ı. Hata sırası: yeni tanımın tam doğrulaması, eski tanımın kimlik alanları (`old.` önekiyle), attnum
/// eşlemesi, alter'a özgü kurallar, ifade üretimi sırasındaki hatalar (yeniden adlandırma döngüsü, nihai
/// ifade taraması).
pub fn alter(old: &TableDef, new: &TableDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let e = validate_table(new, ctx);
    let old_errs = validate_old(old);
    // Eşleme tutarlılığı iki taraf da kendi başına geçerliyken denetlenir (bozuk eski tanım "bayat" sanılmasın).
    let mut map_errs = vec![];
    check_new_attnums(old, new, &mut map_errs);
    check_new_oids(old, new, &mut map_errs);
    if !old_errs.is_empty() || !map_errs.is_empty() {
        // Eşleme kurulamıyor: hata önceliği yeni tanım → eski tanım → eşleme.
        return Err([e, old_errs, map_errs].into_iter().find(|v| !v.is_empty()).unwrap_or_default());
    }
    let by_attnum: HashMap<i16, &ColumnDef> = old
        .columns
        .iter()
        .filter_map(|c| c.attnum.map(|n| (n, c)))
        .collect();
    let matched: Matched = new
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| (i, c.attnum.map(|n| by_attnum[&n]), c))
        .collect();
    // Kolon yeniden adlandırma eşlemesi (eski ad → yeni ad; silinen kolonlar yok): kısıt karşılaştırması bunu kullanır.
    let col_map: HashMap<&str, &str> = matched
        .iter()
        .filter_map(|(_, oc, c)| oc.map(|oc| (oc.name.as_str(), c.name.as_str())))
        .collect();
    let cons = constraint_diff(old, new, &col_map);
    // Dokunulmamış yüklü alanlar create-time dilbilgisinden/kapılarından geçirilmez (örn. `geometry(Point,4326)`
    // tip metni ya da biçim karakterli bir ad, tablonun yeniden adlandırılmasını/yorumunu engellemesin); yalnızca
    // değişen ya da ifadeye yazılan alanlar doğrulanır (yazım anı doğrulaması `build` içinde).
    let e = drop_untouched_errors(e, old, new, &matched, &cons);
    if !e.is_empty() {
        return Err(e);
    }
    let e = check_alter_rules(old, new, &matched, &cons, ctx);
    if !e.is_empty() {
        return Err(e);
    }
    build(old, new, &matched, &cons, ctx)
}

/// `validate_table(new)` hatalarından, dokunulmamış YÜKLÜ alanlara ait olanları atar: aynı değerle korunan kolon
/// alanları (ad, tip, collation, varsayılan, üretilmiş ifade, yorum, using), korunan ve yeniden kurulmayan
/// kısıtların (ad dışı) alanları ve eski değerle aynı tablo alanları. Yinelenen ad hataları (başka bir
/// satırın değişikliğinden doğar) ve satır düzeyi hatalar korunur.
fn drop_untouched_errors(
    errs: Vec<FieldError>,
    old: &TableDef,
    new: &TableDef,
    matched: &Matched,
    cons: &ConstraintDiff,
) -> Vec<FieldError> {
    if errs.is_empty() {
        return errs;
    }
    let by_oid: HashMap<u32, &crate::model::ConstraintDef> =
        old.constraints.iter().filter_map(|c| c.oid.map(|n| (n, c))).collect();
    let added: HashSet<usize> = cons.adds.iter().map(|(i, _)| *i).collect();
    let dup = |m: &str| m.contains("birden fazla");
    errs.into_iter()
        .filter(|x| {
            let path = x.path.as_str();
            // Tablo düzeyi alanlar: eski değerle aynıysa yüklüdür.
            let same_table_field = match path {
                "name" => new.name == old.name,
                "schema" => new.schema == old.schema,
                "owner" => new.owner == old.owner,
                "tablespace" => new.tablespace == old.tablespace,
                "comment" => new.comment == old.comment,
                _ => false,
            };
            if same_table_field {
                return false;
            }
            if let Some(rest) = path.strip_prefix("columns[")
                && let Some((idx, field)) = rest.split_once("].")
                && let Ok(i) = idx.parse::<usize>()
                && let Some((_, Some(oc), c)) = matched.get(i)
            {
                let same = match field {
                    "name" => oc.name == c.name,
                    "dataType" => oc.data_type == c.data_type,
                    "collation" => oc.collation == c.collation,
                    "default" => oc.default == c.default && oc.identity == c.identity && oc.generated == c.generated,
                    "generated" => oc.generated == c.generated && oc.identity == c.identity,
                    "comment" => oc.comment == c.comment,
                    _ => false,
                };
                return !(same && !dup(&x.message));
            }
            if let Some(rest) = path.strip_prefix("constraints[")
                && let Some((idx, field)) = rest.split_once("].")
                && let Ok(i) = idx.parse::<usize>()
                && !added.contains(&i)
                && let Some(c) = new.constraints.get(i)
                && let Some(oc) = c.oid.and_then(|n| by_oid.get(&n).copied())
            {
                if field == "name" {
                    return !(oc.name == c.name && !dup(&x.message));
                }
                return false;
            }
            true
        })
        .collect()
}

/// Eski tanım: yalnızca SQL'e giren kimlik alanları (şema, ad, kolon ve kısıt adları) ve eşleme anahtarları
/// `attnum`/`oid`.
fn validate_old(old: &TableDef) -> Vec<FieldError> {
    let mut e = vec![];
    let before = e.len();
    check_old_ident("old.schema", &old.schema, &mut e);
    if e.len() == before && super::is_system_schema(&old.schema) {
        e.push(err("old.schema", "Sistem şemalarındaki tablolar düzenlenemez (pg_*, information_schema)."));
    }
    check_old_ident("old.name", &old.name, &mut e);
    let mut seen = HashSet::new();
    let mut names = HashSet::new();
    for (i, c) in old.columns.iter().enumerate() {
        let np = format!("old.columns[{i}].name");
        let before = e.len();
        check_old_ident(&np, &c.name, &mut e);
        if e.len() == before && !names.insert(c.name.as_str()) {
            e.push(err(&np, "Kolon adı eski tanımda birden fazla; tanımı yeniden yükleyin."));
        }
        let p = format!("old.columns[{i}].attnum");
        match c.attnum {
            None => e.push(err(
                &p,
                "Yüklenmiş kolonun attnum değeri yok; tanımı yeniden yükleyin.",
            )),
            Some(n) if !seen.insert(n) => e.push(err(
                &p,
                format!("attnum {n} birden fazla kolonda; tanımı yeniden yükleyin."),
            )),
            Some(_) => {}
        }
    }
    validate_old_constraints(old, &mut e);
    e
}

/// Yeni tanımın attnum'ları: benzersiz ve eski tanımda var (yoksa tanım bayat ya da oynanmış).
fn check_new_attnums(old: &TableDef, new: &TableDef, e: &mut Vec<FieldError>) {
    let known: HashSet<i16> = old.columns.iter().filter_map(|c| c.attnum).collect();
    let mut seen = HashSet::new();
    for (i, c) in new.columns.iter().enumerate() {
        let Some(n) = c.attnum else { continue };
        let p = format!("columns[{i}].attnum");
        if !seen.insert(n) {
            e.push(err(&p, format!("attnum {n} birden fazla kolonda.")));
        } else if !known.contains(&n) {
            e.push(err(
                &p,
                format!("attnum {n} mevcut tabloda yok; tanım bayat, yeniden yükleyin."),
            ));
        }
    }
}

/// Tip metni kanonik biçimde farklı mı (bkz. `canonical_type`).
fn type_changed(a: &ColumnDef, b: &ColumnDef) -> bool {
    canonical_type(&a.data_type) != canonical_type(&b.data_type)
}

/// TYPE ifadesi gerekir mi: tip (kanonik) ya da collation değişti.
fn retyped(oc: &ColumnDef, c: &ColumnDef) -> bool {
    type_changed(oc, c) || oc.collation != c.collation
}

/// Kolon yorum dışı (ve `using` dışı) alanlarda değişmedi mi.
fn same_shape(a: &ColumnDef, b: &ColumnDef) -> bool {
    a.name == b.name
        && !type_changed(a, b)
        && a.not_null == b.not_null
        && a.default == b.default
        && a.identity == b.identity
        && a.generated == b.generated
        && a.collation == b.collation
}

/// Kolon farkı var mı (silme, ekleme ya da korunan kolonda yorum dışı değişim).
fn columns_changed(old: &TableDef, matched: &Matched) -> bool {
    let kept = matched.iter().filter(|(_, oc, _)| oc.is_some()).count();
    kept != old.columns.len()
        || kept != matched.len()
        || matched
            .iter()
            .any(|(_, oc, c)| oc.is_some_and(|oc| !same_shape(oc, c)))
}

/// Alter'a özgü kurallar (yeni tanım tek başına geçerli olsa da eski tanımla birlikte geçersiz olan değişiklikler).
fn check_alter_rules(
    old: &TableDef,
    new: &TableDef,
    matched: &Matched,
    cons: &ConstraintDiff,
    ctx: &BuildCtx,
) -> Vec<FieldError> {
    let mut e = vec![];
    if old.is_partition != new.is_partition {
        e.push(err("isPartition", LOAD_FLAG_MSG));
    }
    if old.inherits != new.inherits {
        e.push(err("inherits", LOAD_FLAG_MSG));
    }
    // Bölüm/kalıtımlı tablo: kolon ve kısıtlar ana tablodan yönetilir; ad/şema/sahip/tablespace/yorumlar serbest.
    let managed = old.is_partition || old.inherits;
    if managed && columns_changed(old, matched) {
        e.push(err("columns", MANAGED_MSG));
    }
    if old.partition_by != new.partition_by {
        e.push(err("partitionBy", PARTITION_BY_MSG));
    }
    if old.tablespace.is_some() && new.tablespace.is_none() {
        e.push(err("tablespace", TABLESPACE_CLEAR_MSG));
    }
    // Bölümlenmiş tablonun kendi depolaması yoktur (gerçek sunucuda doğrulandı, Task 5):
    if is_partitioned(old) {
        // PG 10–17 `SET LOGGED|UNLOGGED`'ı sessizce yoksayar (yeniden yüklemede değişmemiş görünür), PG 18 reddeder.
        if old.persistence != new.persistence {
            e.push(err("persistence", PARTITIONED_PERSISTENCE_MSG));
        }
        // PG 10–11 `SET TABLESPACE`'i sessizce yoksayar; PG 12+ yeni bölümlerin varsayılanını ayarlar.
        if old.tablespace != new.tablespace && new.tablespace.is_some() && ctx.version_num < 120000 {
            e.push(version_err("tablespace", "Bölümlenmiş tabloda SET TABLESPACE", 12));
        }
        if old.fillfactor != new.fillfactor {
            e.push(err("fillfactor", "Bölümlenmiş tabloda fillfactor belirtilemez."));
        }
        // PG 11–17: "cannot add NOT VALID foreign key on partitioned table"; PG 18 kabul eder.
        if ctx.version_num < 180000 {
            for (i, c) in &cons.adds {
                if c.not_valid && matches!(c.kind, ConstraintKind::ForeignKey { .. }) {
                    e.push(version_err(
                        &format!("constraints[{i}].notValid"),
                        "Bölümlenmiş tabloya NOT VALID yabancı anahtar eklemek",
                        18,
                    ));
                }
            }
        }
    }
    // PG 10–16: "cannot recursively add identity column to table that has child tables" — bölümü olan bölümlenmiş
    // tabloya kimlikli kolon EKLENEMEZ (gerçek sunucuda doğrulandı, Task 5; PG 17+ kabul eder). Bölümün varlığı yerelde
    // bilinmediğinden yüklenmiş bölümlenmiş tabloda bölüm olduğu varsayılır. Mevcut kolona `ADD GENERATED … AS IDENTITY`
    // her sürümde kabul edilir. Kalıtım EBEVEYNİ de (tüm sürümlerde) reddeder ama tablonun çocuğu olup olmadığı tanımda
    // yoktur → sunucuya bırakılır.
    if is_partitioned(old) && ctx.version_num < 170000 {
        for (i, oc, c) in matched {
            if oc.is_none() && c.identity.is_some() {
                e.push(err(&format!("columns[{i}].identity"), PARTITIONED_ADD_IDENTITY_MSG));
            }
        }
    }
    // PG 18: "SET EXPRESSION is not supported for virtual generated columns in tables with check constraints"
    // (gerçek sunucuda doğrulandı, Task 5). Adım 8'de tabloda kalan CHECK'ler: silinmeyen eski CHECK'ler ve
    // desteklenmeyen listesindeki CHECK'ler (ör. NOT ENFORCED).
    let has_checks = old.constraints.iter().any(|oc| {
        matches!(oc.kind, ConstraintKind::Check { .. }) && !cons.drops.iter().any(|d| d.oid == oc.oid)
    }) || old.unsupported.iter().any(|u| is_unsupported_check(u));
    if !managed {
        for (i, oc, c) in matched {
            let Some(oc) = oc else { continue };
            let p = format!("columns[{i}].generated");
            match (&oc.generated, &c.generated) {
                (None, Some(_)) => e.push(err(
                    &p,
                    "Mevcut kolonu üretilmiş kolona çevirmek desteklenmez; yeni kolon ekleyin.",
                )),
                (Some(_), None) if ctx.version_num < 130000 => {
                    e.push(version_err(&p, "Üretilmiş ifadeyi kaldırmak (DROP EXPRESSION)", 13));
                }
                // PG 18: DROP EXPRESSION yalnızca STORED kolonda; sanal kolonun değerleri saklanmadığından korunamaz.
                (Some(og), None) if !og.stored => e.push(err(
                    &p,
                    "Sanal üretilmiş kolonun ifadesi kaldırılamaz (DROP EXPRESSION yalnızca STORED); kolonu silip yeniden ekleyin.",
                )),
                (Some(og), Some(ng)) if og.stored != ng.stored => e.push(err(
                    &p,
                    "Üretilmiş kolonun STORED/VIRTUAL türü değiştirilemez; kolonu silip yeniden ekleyin.",
                )),
                (Some(og), Some(ng)) if og.expr != ng.expr && ctx.version_num < 170000 => e.push(err(
                    &p,
                    "Üretilmiş ifadeyi değiştirmek (SET EXPRESSION) PG 17+ gerektirir; kolonu silip yeniden ekleyin.",
                )),
                (Some(og), Some(ng)) if og.expr != ng.expr && !og.stored && has_checks => e.push(err(
                    &p,
                    "CHECK kısıtı olan tabloda sanal üretilmiş kolonun ifadesi değiştirilemez (PG 18); kolonu silip yeniden ekleyin ya da CHECK kısıtlarını aynı düzenlemede kaldırın.",
                )),
                _ => {}
            }
        }
    }
    // Bölüm/kalıtımlı tabloda herhangi bir kısıt farkı (sıra hariç) hatadır; diğer tablolarda fark kuralları.
    if managed && (cons.changed() || !cons.errs.is_empty()) {
        e.push(err("constraints", MANAGED_MSG));
    } else {
        e.extend(cons.errs.iter().cloned());
    }
    e
}

/// Korunan kolonların göreli sırası değişti mi ya da yeni kolon korunan bir kolonun önüne mi kondu (ADD sona ekler).
fn order_changed(old: &TableDef, new: &TableDef) -> bool {
    let pos: HashMap<i16, usize> = old
        .columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.attnum.map(|n| (n, i)))
        .collect();
    let mut last: Option<usize> = None;
    let mut added_before = false;
    for c in &new.columns {
        match c.attnum {
            None => added_before = true,
            Some(n) => {
                let p = pos[&n];
                if added_before || last.is_some_and(|l| p < l) {
                    return true;
                }
                last = Some(p);
            }
        }
    }
    false
}

/// İfade metninde `name` kolonuna başvuru var mı (yaklaşık): tek tırnaklı dizgeler atlanır; tırnaksız sözcükler küçük
/// harfe katlanarak, çift tırnaklı adlar aynen karşılaştırılır. Yalnızca uyarı üretmek için kullanılır.
/// Bilinen boşluklar (uyarı olduğundan kabul): `E'..'` kaçışları ve `$$` dizgeleri ayrıştırılmaz; katlama Unicode
/// `to_lowercase` iledir (PG yalnızca ASCII harfleri katlar).
fn expr_mentions(expr: &str, name: &str) -> bool {
    let c: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            q @ ('\'' | '"') => {
                i += 1;
                let mut text = String::new();
                while i < c.len() {
                    if c[i] == q {
                        if c.get(i + 1) == Some(&q) {
                            text.push(q);
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    text.push(c[i]);
                    i += 1;
                }
                i += 1;
                if q == '"' && text == name {
                    return true;
                }
            }
            ch if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                    i += 1;
                }
                let w: String = c[start..i].iter().collect::<String>().to_lowercase();
                if w == name {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Boş yorum `None` sayılır (bkz. `has_comment`).
fn norm_comment(c: &Option<String>) -> Option<String> {
    c.clone().filter(|s| !s.is_empty())
}

fn identity_sql(k: IdentityKind) -> &'static str {
    match k {
        IdentityKind::Always => "ALWAYS",
        IdentityKind::ByDefault => "BY DEFAULT",
    }
}

#[derive(Default)]
struct Out {
    statements: Vec<Statement>,
    errs: Vec<FieldError>,
    /// Tabloyu yeniden yazan ifade var (uyarı + lock_timeout önerisi).
    rewrite: bool,
    /// Uzun sürebilen (tarayan) ifade var (lock_timeout önerisi).
    long: bool,
}

impl Out {
    fn push(&mut self, sql: String, risk: Risk, note: Option<&str>) {
        self.statements.push(stmt(sql, risk, note));
    }

    /// Serbest SQL içeren ifade: alan kapısına ek olarak NİHAİ ifade de taranır.
    fn push_free(&mut self, path: &str, sql: String, risk: Risk, note: Option<&str>) {
        if composed_is_clean(&sql) {
            self.push(sql, risk, note);
        } else {
            self.errs.push(err(path, COMPOSED_MSG));
        }
    }
}

/// Yeniden adlandırma sırası: (konum, eski ad, yeni ad) kayıtları, hedef adı boşalan sırayla (a→b, b→c zincirinde
/// önce b→c). Doğrusal: bekleyen kaynak adları kümesi + hedef ad → kayıt eşlemesi; bir yeniden adlandırma çıkınca
/// kaynak adını hedefleyen (hedefler benzersiz olduğundan en çok bir) kayıt hazır olur. Döngüde (a↔b) kalan ilk
/// kaydın konumu ikinci değerdir.
fn rename_order(renames: &[(usize, &str, &str)]) -> (Vec<usize>, Option<usize>) {
    let mut pending_from: HashSet<&str> = renames.iter().map(|(_, from, _)| *from).collect();
    let by_target: HashMap<&str, usize> = renames.iter().enumerate().map(|(k, (_, _, to))| (*to, k)).collect();
    let mut done = vec![false; renames.len()];
    let mut order = Vec::with_capacity(renames.len());
    let mut ready: VecDeque<usize> = (0..renames.len())
        .filter(|&k| !pending_from.contains(renames[k].2))
        .collect();
    while let Some(k) = ready.pop_front() {
        let (_, from, _) = renames[k];
        done[k] = true;
        pending_from.remove(from);
        order.push(k);
        if let Some(&next) = by_target.get(from)
            && !done[next]
        {
            ready.push_back(next);
        }
    }
    (order, done.iter().position(|d| !d).map(|k| renames[k].0))
}

fn build(
    old: &TableDef,
    new: &TableDef,
    matched: &Matched,
    cons: &ConstraintDiff,
    ctx: &BuildCtx,
) -> Result<Plan, Vec<FieldError>> {
    let mut out = Out::default();
    let mut warnings = vec![];
    if order_changed(old, new) {
        warnings.push(COLUMN_ORDER_WARNING.to_string());
    }
    let kept: Vec<(usize, &ColumnDef, &ColumnDef)> = matched
        .iter()
        .filter_map(|(i, oc, c)| oc.map(|oc| (*i, oc, *c)))
        .collect();

    // (1) tablo adı, (2) şema: sonraki tüm ifadeler yeni nitelikli adı kullanır.
    let old_q = qualified(&old.schema, &old.name);
    if old.name != new.name {
        out.push(
            format!("ALTER TABLE {old_q} RENAME TO {}", quote_ident(&new.name)),
            Risk::Safe,
            Some(RENAME_TABLE_NOTE),
        );
    }
    if old.schema != new.schema {
        out.push(
            format!(
                "ALTER TABLE {} SET SCHEMA {}",
                qualified(&old.schema, &new.name),
                quote_ident(&new.schema)
            ),
            Risk::Safe,
            Some(SET_SCHEMA_NOTE),
        );
    }
    let q = qualified(&new.schema, &new.name);

    // (3) silinen ve yapısal değişen kısıtlar (eski adla; FK'ler önce): kolon silme/tip değişiminden önce.
    let mut pk_dropped = false;
    for oc in &cons.drops {
        // Eski kısıt adı `validate_old_constraints` ile doğrulandı (dolu).
        if let Some(n) = &oc.name {
            out.push(drop_sql(&q, n), Risk::Destructive, Some(DROP_CONSTRAINT_NOTE));
        }
        pk_dropped |= is_pk(oc);
    }

    // (4) silinen kolonlar (eski sırayla).
    let kept_attnums: HashSet<i16> = kept.iter().filter_map(|(_, oc, _)| oc.attnum).collect();
    // PG, silinen kolona başvuran tablo kısıtlarını (CHECK, EXCLUDE) CASCADE'siz de kendiliğinden düşürür: planda
    // görünmez ve yeniden yüklenen tanım yeni tanımdan farklı olur → uyarı.
    let dropped_cols: Vec<&ColumnDef> = old
        .columns
        .iter()
        .filter(|oc| oc.attnum.is_some_and(|n| !kept_attnums.contains(&n)))
        .collect();
    for dc in &dropped_cols {
        for k in &cons.kept {
            if let ConstraintKind::Check { expr, .. } = &k.kind
                && expr_mentions(expr, &dc.name)
            {
                warnings.push(format!(
                    "'{}' kolonu silindiğinde ona başvuran '{}' CHECK kısıtı PostgreSQL tarafından kendiliğinden düşürülür; kısıtı yeni tanımdan da kaldırın ya da kolonu koruyun.",
                    dc.name,
                    k.name.as_deref().unwrap_or("?")
                ));
            }
        }
    }
    if !dropped_cols.is_empty() && !old.unsupported.is_empty() {
        warnings.push(
            "Tabloda desteklenmeyen kısıtlar var (EXCLUDE, tetikleyici vb.); silinen kolona başvuranlar PostgreSQL tarafından kendiliğinden düşürülür ve burada görünmez."
                .to_string(),
        );
    }
    for oc in &old.columns {
        if oc.attnum.is_some_and(|n| !kept_attnums.contains(&n)) {
            out.push(
                format!("ALTER TABLE {q} DROP COLUMN {}", quote_ident(&oc.name)),
                Risk::DataLoss,
                Some(DROP_COLUMN_NOTE),
            );
        }
    }

    // (5) tip/collation değişimi: yeniden adlandırmadan önce, ESKİ kolon adıyla.
    for (i, oc, c) in &kept {
        if retyped(oc, c) {
            if oc.default.is_some() {
                out.push(
                    format!("ALTER TABLE {q} ALTER COLUMN {} DROP DEFAULT", quote_ident(&oc.name)),
                    Risk::Safe,
                    Some(TYPE_DROP_DEFAULT_NOTE),
                );
            }
            // Tip metni ham yerleşir: değişmemiş yüklü metin (yalnızca collation değişimi) yazım anında doğrulanır.
            // Collation yalnızca TYPE ifadesinin parçası olarak değişir (COLLATE yoksa yeni tipin varsayılan collation'ı).
            if let Err(m) = validate_type(&c.data_type) {
                out.errs.push(err(&format!("columns[{i}].dataType"), m));
                continue;
            }
            let mut sql = format!(
                "ALTER TABLE {q} ALTER COLUMN {} TYPE {}",
                quote_ident(&oc.name),
                c.data_type
            );
            if let Some(cl) = &c.collation {
                sql.push_str(&format!(" COLLATE {}", quote_ident(cl)));
            }
            let note = match &c.using {
                Some(u) => {
                    sql.push_str(&format!(" USING {}", wrap(u)));
                    format!("{TYPE_NOTE}{TYPE_USING_NOTE}")
                }
                None => format!("{TYPE_NOTE}{TYPE_USING_HINT}"),
            };
            out.push_free(&format!("columns[{i}].using"), sql, Risk::DataLoss, Some(&note));
            out.rewrite = true;
        } else if c.using.is_some() {
            warnings.push(format!(
                "'{}' kolonunun tipi değişmediği için USING yoksayıldı.",
                c.name
            ));
        }
    }

    // (6) kolon adları: zincirler (a→b, b→c) hedef adı boşalan sırayla; döngü (a↔b) doğrulama hatası.
    let renames: Vec<(usize, &str, &str)> = kept
        .iter()
        .filter(|(_, oc, c)| oc.name != c.name)
        .map(|(i, oc, c)| (*i, oc.name.as_str(), c.name.as_str()))
        .collect();
    let (order, cycle) = rename_order(&renames);
    for k in order {
        let (_, from, to) = renames[k];
        out.push(
            format!(
                "ALTER TABLE {q} RENAME COLUMN {} TO {}",
                quote_ident(from),
                quote_ident(to)
            ),
            Risk::Safe,
            Some(RENAME_COLUMN_NOTE),
        );
    }
    if let Some(i) = cycle {
        out.errs.push(err(
            &format!("columns[{i}].name"),
            "Kolon adları döngüsel olarak değiştirilemez (ör. a↔b takası); önce ara bir adla kaydedin.",
        ));
    }

    // (7) eklenen kolonlar (create'teki kolon biçimi; tabloya sona eklenir).
    for (i, oc, c) in matched {
        if oc.is_some() {
            continue;
        }
        if c.using.is_some() {
            warnings.push(format!(
                "'{}' yeni bir kolon; USING yalnızca tip değişiminde kullanılır, yoksayıldı.",
                c.name
            ));
        }
        // Kimlik değerleri diziden, varsayılan değer ifadeden gelir; üretilmiş ifade NULL üretebilir.
        let fails_on_rows = c.not_null && c.default.is_none() && c.identity.is_none();
        let (risk, note) = if !fails_on_rows {
            (Risk::Safe, None)
        } else if c.generated.is_some() {
            (Risk::Destructive, Some(ADD_GENERATED_NOT_NULL_NOTE))
        } else {
            (Risk::Destructive, Some(ADD_NOT_NULL_NOTE))
        };
        // Tabloyu yeniden yazan eklemeler: kimlik, STORED üretilmiş, PG 11 öncesinde varsayılanlı kolon (değişken
        // varsayılan PG 11+'da da yeniden yazar ama yerelde anlaşılamaz).
        if c.identity.is_some()
            || c.generated.as_ref().is_some_and(|g| g.stored)
            || (c.default.is_some() && ctx.version_num < 110000)
        {
            out.rewrite = true;
        }
        out.push_free(
            &format!("columns[{i}]"),
            format!("ALTER TABLE {q} ADD COLUMN {}", column_sql(c)),
            risk,
            note,
        );
    }

    // (8) nullability/varsayılan/kimlik/üretilmiş ifade (YENİ kolon adıyla; kolon başına sabit iç sıra, bkz. modül).
    for (i, oc, c) in &kept {
        let ac = |rest: &str| format!("ALTER TABLE {q} ALTER COLUMN {} {rest}", quote_ident(&c.name));
        if oc.identity.is_some() && c.identity.is_none() {
            out.push(ac("DROP IDENTITY"), Risk::Safe, Some(DROP_IDENTITY_NOTE));
        }
        if oc.generated.is_some() && c.generated.is_none() {
            out.push(ac("DROP EXPRESSION"), Risk::Safe, Some(DROP_EXPRESSION_NOTE));
        }
        // Tip değişiminde eski varsayılan adım 5'te kaldırıldı: yeni varsayılan (aynı metin olsa da) yeniden kurulur.
        if retyped(oc, c) && oc.default.is_some() {
            if let Some(d) = &c.default {
                // Varsayılan metin değişmemiş olabilir (yüklü): yazım anında doğrulanır.
                let before = out.errs.len();
                check_free_sql(&format!("columns[{i}].default"), d, &mut out.errs);
                if out.errs.len() == before {
                    out.push_free(
                        &format!("columns[{i}].default"),
                        ac(&format!("SET DEFAULT {}", wrap(d))),
                        Risk::Safe,
                        None,
                    );
                }
            }
        } else if oc.default != c.default {
            match &c.default {
                Some(d) => out.push_free(
                    &format!("columns[{i}].default"),
                    ac(&format!("SET DEFAULT {}", wrap(d))),
                    Risk::Safe,
                    None,
                ),
                None => out.push(ac("DROP DEFAULT"), Risk::Safe, None),
            }
        }
        if oc.not_null != c.not_null {
            if c.not_null {
                out.push(ac("SET NOT NULL"), Risk::Destructive, Some(SET_NOT_NULL_NOTE));
                out.long = true;
            } else {
                out.push(ac("DROP NOT NULL"), Risk::Safe, None);
            }
        }
        match (oc.identity, c.identity) {
            (None, Some(k)) => out.push(
                ac(&format!("ADD GENERATED {} AS IDENTITY", identity_sql(k))),
                Risk::Safe,
                Some(ADD_IDENTITY_NOTE),
            ),
            (Some(a), Some(k)) if a != k => out.push(
                ac(&format!("SET GENERATED {}", identity_sql(k))),
                Risk::Safe,
                None,
            ),
            _ => {}
        }
        if let (Some(og), Some(ng)) = (&oc.generated, &c.generated)
            && og.expr != ng.expr
        {
            out.push_free(
                &format!("columns[{i}].generated"),
                ac(&format!("SET EXPRESSION AS {}", wrap(&ng.expr))),
                Risk::Destructive,
                Some(SET_EXPRESSION_NOTE),
            );
            // STORED değerler yeniden hesaplanıp yazılır; VIRTUAL değerler saklanmaz.
            if ng.stored {
                out.rewrite = true;
            }
        }
    }

    // (9) kısıt adları (eski → yeni; zincirler kolonlardaki gibi sıralı, döngü hatası).
    let (order, cycle) = rename_order(&cons.renames);
    for k in order {
        let (i, from, to) = cons.renames[k];
        out.push(rename_sql(&q, from, to), Risk::Safe, Some(rename_note(&new.constraints[i])));
    }
    if let Some(i) = cycle {
        out.errs.push(err(
            &format!("constraints[{i}].name"),
            "Kısıt adları döngüsel olarak değiştirilemez (ör. a↔b takası); önce ara bir adla kaydedin.",
        ));
    }

    // (10) yeni ve yeniden kurulan kısıtlar (yeni tanım; FK'ler sonda), ardından doğrulamalar (yeni adla).
    let mut scans = false;
    for (i, c) in &cons.adds {
        let (note, scan) = add_note(c);
        scans |= scan;
        out.push_free(&add_path(*i, c), add_sql(&q, c), Risk::Safe, Some(note));
    }
    for n in &cons.validates {
        out.push(validate_sql(&q, n), Risk::Safe, Some(VALIDATE_NOTE));
    }
    if scans {
        out.long = true;
    }

    // (11) tablo özellikleri ve yorumlar.
    if let Some(ow) = &new.owner
        && old.owner.as_ref() != Some(ow)
    {
        out.push(format!("ALTER TABLE {q} OWNER TO {}", quote_ident(ow)), Risk::Safe, None);
    }
    if old.tablespace != new.tablespace
        && let Some(ts) = &new.tablespace
    {
        out.push(
            format!("ALTER TABLE {q} SET TABLESPACE {}", quote_ident(ts)),
            Risk::Safe,
            Some(REWRITE_NOTE),
        );
        out.rewrite = true;
    }
    if old.persistence != new.persistence {
        let what = match new.persistence {
            Persistence::Permanent => "LOGGED",
            Persistence::Unlogged => "UNLOGGED",
        };
        // UNLOGGED: WAL'a yazılmaz (kilit/çökmede tablo boşalır, yedek sunucularda veri yok) → Destructive.
        let risk = if new.persistence == Persistence::Unlogged { Risk::Destructive } else { Risk::Safe };
        out.push(format!("ALTER TABLE {q} SET {what}"), risk, Some(REWRITE_NOTE));
        out.rewrite = true;
    }
    if old.fillfactor != new.fillfactor {
        let sql = match new.fillfactor {
            Some(n) => format!("ALTER TABLE {q} SET (fillfactor = {n})"),
            None => format!("ALTER TABLE {q} RESET (fillfactor)"),
        };
        out.push(sql, Risk::Safe, Some(FILLFACTOR_NOTE));
    }
    if norm_comment(&old.comment) != norm_comment(&new.comment) {
        out.push(table_comment_sql(&q, &norm_comment(&new.comment)), Risk::Safe, None);
    }
    for (_, oc, c) in matched {
        let changed = match oc {
            Some(oc) => norm_comment(&oc.comment) != norm_comment(&c.comment),
            None => has_comment(&c.comment),
        };
        if changed {
            out.push(
                column_comment_sql(new, &c.name, &norm_comment(&c.comment)),
                Risk::Safe,
                None,
            );
        }
    }

    if !out.errs.is_empty() {
        return Err(out.errs);
    }
    // Yerinde kalan kısıtın kolonunun tipi değişiyor: PG kısıtı tip değişimi sırasında yeniden doğrular/kurar.
    let retyped_cols: HashSet<&str> = kept
        .iter()
        .filter(|(_, oc, c)| retyped(oc, c))
        .map(|(_, _, c)| c.name.as_str())
        .collect();
    for c in &cons.kept {
        if constraint_columns(c).iter().any(|n| retyped_cols.contains(n)) {
            warnings.push(format!(
                "'{}' kısıtının kolon tipi değişiyor; kısıt yerinde kalır, PG tip değişimi sırasında onu yeniden doğrular (PRIMARY KEY/UNIQUE indeksi yeniden oluşturulur) ve tip uyumsuzsa uygulama başarısız olur.",
                c.name.as_deref().unwrap_or_default()
            ));
        }
    }
    // PG 12+ "cannot alter type of a column used by a generated column" (gerçek sunucuda doğrulandı, Task 5). İfadenin
    // hangi kolonları kullandığı yerelde kesin bilinemez: ifadede adı geçen kolon için uyarı (yanlış pozitif zararsız).
    for (_, oc, c) in kept.iter().filter(|(_, oc, c)| retyped(oc, c)) {
        for (_, _, g) in &kept {
            if let Some(gd) = &g.generated
                && g.name != c.name
                && (expr_mentions(&gd.expr, &oc.name) || expr_mentions(&gd.expr, &c.name))
            {
                warnings.push(format!(
                    "'{}' kolonu üretilmiş '{}' kolonunun ifadesinde geçiyor; PG üretilmiş kolonun kullandığı kolonun tipini değiştirmeye izin vermez (önce üretilmiş kolonu silin, sonra yeniden ekleyin).",
                    c.name, g.name
                ));
            }
        }
    }
    if pk_dropped {
        warnings.push(PK_DROP_WARNING.to_string());
    }
    if scans {
        warnings.push(ADD_CONSTRAINT_WARNING.to_string());
    }
    if out.rewrite {
        warnings.push(REWRITE_WARNING.to_string());
    }
    if out.rewrite || out.long {
        warnings.push(LOCK_TIMEOUT_WARNING.to_string());
    }
    Ok(Plan {
        statements: out.statements,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::type_syntax::{is_spoofing_format, validate_type};
    use crate::build::{alter as balter, assert_single_statements, render, scan_semicolons};
    use crate::model::*;

    fn v(n: u32) -> BuildCtx {
        BuildCtx {
            version_num: n,
            ..Default::default()
        }
    }
    fn ctx() -> BuildCtx {
        v(180000)
    }
    fn o(d: TableDef) -> ObjectDef {
        ObjectDef::Table(d)
    }
    fn col(n: &str, ty: &str) -> ColumnDef {
        ColumnDef {
            attnum: None,
            name: n.into(),
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
    /// Yüklenmiş kolon (attnum dolu).
    fn lc(n: i16, name: &str, ty: &str) -> ColumnDef {
        ColumnDef {
            attnum: Some(n),
            ..col(name, ty)
        }
    }
    fn base() -> TableDef {
        TableDef {
            schema: "public".into(),
            name: "t".into(),
            owner: None,
            tablespace: None,
            persistence: Persistence::Permanent,
            columns: vec![lc(1, "id", "integer")],
            constraints: vec![],
            partition_by: None,
            fillfactor: None,
            comment: None,
            partition_text: None,
            is_partition: false,
            inherits: false,
            unsupported: vec![],
        }
    }
    fn pk_id() -> ConstraintDef {
        ConstraintDef {
            oid: Some(100),
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
    /// Yüklenmiş zengin tablo: kimlik PK, collation + yorum, varsayılan, üretilmiş, sıradan, NULL'lanabilir tamsayı,
    /// PK dışı kimlik.
    fn loaded() -> TableDef {
        TableDef {
            owner: Some("app".into()),
            tablespace: Some("fast".into()),
            fillfactor: Some(70),
            comment: Some("Tablo".into()),
            columns: vec![
                ColumnDef {
                    not_null: true,
                    identity: Some(IdentityKind::Always),
                    ..lc(1, "id", "bigint")
                },
                ColumnDef {
                    not_null: true,
                    collation: Some("C".into()),
                    comment: Some("ad".into()),
                    ..lc(2, "caption", "text")
                },
                ColumnDef {
                    not_null: true,
                    default: Some("0".into()),
                    ..lc(3, "qty", "integer")
                },
                ColumnDef {
                    generated: Some(GeneratedDef {
                        expr: "qty * 2".into(),
                        stored: true,
                    }),
                    ..lc(4, "total", "numeric(10,2)")
                },
                lc(5, "note", "text"),
                lc(6, "amount", "integer"),
                ColumnDef {
                    not_null: true,
                    identity: Some(IdentityKind::ByDefault),
                    ..lc(7, "seq", "integer")
                },
            ],
            constraints: vec![pk_id()],
            ..base()
        }
    }
    fn with(d: &TableDef, f: impl FnOnce(&mut TableDef)) -> TableDef {
        let mut d = d.clone();
        f(&mut d);
        d
    }
    fn idx(d: &TableDef, name: &str) -> usize {
        d.columns.iter().position(|c| c.name == name).unwrap()
    }
    fn colm<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ColumnDef {
        let i = idx(d, name);
        &mut d.columns[i]
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn clean(p: &Plan) {
        assert_single_statements(p);
        for s in &p.statements {
            let r = scan_semicolons(&s.sql);
            assert!(
                r.is_clean() && r.parens_balanced() && !r.ends_in_line_comment,
                "{}",
                s.sql
            );
            assert!(s.transactional, "{}", s.sql);
        }
    }
    fn ok(old: &TableDef, new: &TableDef, c: &BuildCtx) -> Plan {
        let p = balter(&o(old.clone()), &o(new.clone()), c)
            .unwrap_or_else(|e| panic!("reddedildi: {e:?}\n{new:?}"));
        clean(&p);
        p
    }
    fn errs(old: &TableDef, new: &TableDef, c: &BuildCtx) -> Vec<FieldError> {
        match balter(&o(old.clone()), &o(new.clone()), c) {
            Ok(p) => panic!("hata bekleniyordu:\n{}", render(&p)),
            Err(e) => e,
        }
    }

    #[test]
    fn noop_is_empty_plan() {
        let unlogged = TableDef {
            persistence: Persistence::Unlogged,
            ..loaded()
        };
        let virt = with(&loaded(), |d| {
            colm(d, "total").generated.as_mut().unwrap().stored = false;
        });
        let parted = TableDef {
            columns: vec![
                ColumnDef { not_null: true, ..lc(1, "id", "bigint") },
                lc(2, "created", "timestamp with time zone"),
            ],
            partition_by: Some(PartitionByDef {
                strategy: PartitionStrategy::Range,
                keys: vec![
                    PartitionKey::Column { name: "created".into() },
                    PartitionKey::Expression { sql: "date_trunc('day', created)".into() },
                ],
            }),
            partition_text: Some("RANGE (created, date_trunc('day'::text, created))".into()),
            ..base()
        };
        let part = TableDef {
            is_partition: true,
            unsupported: vec!["EXCLUDE kısıtı x".into()],
            ..loaded()
        };
        let inh = TableDef { inherits: true, ..loaded() };
        for d in [base(), loaded(), unlogged, virt, parted, part, inh] {
            let p = ok(&d, &d, &ctx());
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}", render(&p));
        }
        // Boş yorum `None` sayılır; sahip kaldırma (None) değişiklik değildir (sahip temizlenemez).
        let p = ok(
            &with(&loaded(), |d| {
                d.comment = None;
                colm(d, "note").comment = Some(String::new());
            }),
            &with(&loaded(), |d| {
                d.comment = Some(String::new());
                d.owner = None;
            }),
            &ctx(),
        );
        assert!(p.statements.is_empty(), "{}", render(&p));
    }

    /// Her değişiklik türü tek başına: (etiket, sürüm, eski, yeni, beklenen ifadeler).
    #[test]
    fn single_changes_exact_and_snapshot() {
        let l = loaded();
        let unlogged = TableDef { persistence: Persistence::Unlogged, ..loaded() };
        let add = |c: ColumnDef| with(&l, |d| d.columns.push(c));
        let cases: Vec<(&str, u32, TableDef, TableDef, Vec<&str>)> = vec![
            ("tablo adı", 120000, l.clone(), with(&l, |d| d.name = "t2".into()), vec!["ALTER TABLE public.t RENAME TO t2"]),
            ("şema", 120000, l.clone(), with(&l, |d| d.schema = "s2".into()), vec!["ALTER TABLE public.t SET SCHEMA s2"]),
            (
                "kolon silme",
                120000,
                l.clone(),
                with(&l, |d| d.columns.retain(|c| c.name != "note")),
                vec!["ALTER TABLE public.t DROP COLUMN note"],
            ),
            (
                "tip USING'siz",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "note").data_type = "character varying(20)".into()),
                vec!["ALTER TABLE public.t ALTER COLUMN note TYPE character varying(20)"],
            ),
            (
                "tip USING'li",
                120000,
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "amount");
                    c.data_type = "bigint".into();
                    c.using = Some("amount::bigint".into());
                }),
                vec!["ALTER TABLE public.t ALTER COLUMN amount TYPE bigint USING (amount::bigint\n)"],
            ),
            (
                "tip + collation korunur",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").data_type = "character varying(100)".into()),
                vec!["ALTER TABLE public.t ALTER COLUMN caption TYPE character varying(100) COLLATE \"C\""],
            ),
            (
                "yalnız collation değişimi",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").collation = Some("POSIX".into())),
                vec!["ALTER TABLE public.t ALTER COLUMN caption TYPE text COLLATE \"POSIX\""],
            ),
            (
                "collation kaldırma (tipin varsayılanı)",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").collation = None),
                vec!["ALTER TABLE public.t ALTER COLUMN caption TYPE text"],
            ),
            (
                "kolon adı",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "note").name = "memo".into()),
                vec!["ALTER TABLE public.t RENAME COLUMN note TO memo"],
            ),
            ("ekleme NULL'lanabilir", 120000, l.clone(), add(col("extra", "text")), vec!["ALTER TABLE public.t ADD COLUMN extra text"]),
            (
                "ekleme NOT NULL varsayılansız",
                120000,
                l.clone(),
                add(ColumnDef { not_null: true, ..col("extra", "text") }),
                vec!["ALTER TABLE public.t ADD COLUMN extra text NOT NULL"],
            ),
            (
                "ekleme NOT NULL varsayılanlı",
                120000,
                l.clone(),
                add(ColumnDef { not_null: true, default: Some("1".into()), ..col("extra", "integer") }),
                vec!["ALTER TABLE public.t ADD COLUMN extra integer DEFAULT (1\n) NOT NULL"],
            ),
            (
                "ekleme kimlik",
                120000,
                l.clone(),
                add(ColumnDef { not_null: true, identity: Some(IdentityKind::ByDefault), ..col("extra", "bigint") }),
                vec!["ALTER TABLE public.t ADD COLUMN extra bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL"],
            ),
            (
                "ekleme üretilmiş STORED",
                120000,
                l.clone(),
                add(ColumnDef {
                    generated: Some(GeneratedDef { expr: "qty + 1".into(), stored: true }),
                    ..col("extra", "integer")
                }),
                vec!["ALTER TABLE public.t ADD COLUMN extra integer GENERATED ALWAYS AS (qty + 1\n) STORED"],
            ),
            (
                "ekleme üretilmiş NOT NULL",
                120000,
                l.clone(),
                add(ColumnDef {
                    not_null: true,
                    generated: Some(GeneratedDef { expr: "qty + 1".into(), stored: true }),
                    ..col("extra", "integer")
                }),
                vec!["ALTER TABLE public.t ADD COLUMN extra integer GENERATED ALWAYS AS (qty + 1\n) STORED NOT NULL"],
            ),
            (
                "ekleme collation + yorum",
                120000,
                l.clone(),
                add(ColumnDef { collation: Some("C".into()), comment: Some("e'".into()), ..col("Extra", "text") }),
                vec![
                    "ALTER TABLE public.t ADD COLUMN \"Extra\" text COLLATE \"C\"",
                    "COMMENT ON COLUMN public.t.\"Extra\" IS 'e'''",
                ],
            ),
            (
                "SET NOT NULL",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "note").not_null = true),
                vec!["ALTER TABLE public.t ALTER COLUMN note SET NOT NULL"],
            ),
            (
                "DROP NOT NULL",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "qty").not_null = false),
                vec!["ALTER TABLE public.t ALTER COLUMN qty DROP NOT NULL"],
            ),
            (
                "SET DEFAULT",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "note").default = Some("'x'".into())),
                vec!["ALTER TABLE public.t ALTER COLUMN note SET DEFAULT ('x'\n)"],
            ),
            (
                "varsayılan değişimi",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "qty").default = Some("1 -- bir\n".into())),
                vec!["ALTER TABLE public.t ALTER COLUMN qty SET DEFAULT (1 -- bir\n\n)"],
            ),
            (
                "DROP DEFAULT",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "qty").default = None),
                vec!["ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT"],
            ),
            (
                "kimlik ekle (NOT NULL önce)",
                120000,
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "amount");
                    c.identity = Some(IdentityKind::Always);
                    c.not_null = true;
                }),
                vec![
                    "ALTER TABLE public.t ALTER COLUMN amount SET NOT NULL",
                    "ALTER TABLE public.t ALTER COLUMN amount ADD GENERATED ALWAYS AS IDENTITY",
                ],
            ),
            (
                "varsayılan → kimlik (DROP DEFAULT önce)",
                120000,
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "qty");
                    c.default = None;
                    c.identity = Some(IdentityKind::ByDefault);
                }),
                vec![
                    "ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT",
                    "ALTER TABLE public.t ALTER COLUMN qty ADD GENERATED BY DEFAULT AS IDENTITY",
                ],
            ),
            (
                "kimlik kaldır",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "seq").identity = None),
                vec!["ALTER TABLE public.t ALTER COLUMN seq DROP IDENTITY"],
            ),
            (
                "kimlik kaldır + DROP NOT NULL + varsayılan (DROP IDENTITY önce)",
                120000,
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "seq");
                    c.identity = None;
                    c.not_null = false;
                    c.default = Some("7".into());
                }),
                vec![
                    "ALTER TABLE public.t ALTER COLUMN seq DROP IDENTITY",
                    "ALTER TABLE public.t ALTER COLUMN seq SET DEFAULT (7\n)",
                    "ALTER TABLE public.t ALTER COLUMN seq DROP NOT NULL",
                ],
            ),
            (
                "kimlik türü ALWAYS",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "seq").identity = Some(IdentityKind::Always)),
                vec!["ALTER TABLE public.t ALTER COLUMN seq SET GENERATED ALWAYS"],
            ),
            (
                "kimlik türü BY DEFAULT",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "id").identity = Some(IdentityKind::ByDefault)),
                vec!["ALTER TABLE public.t ALTER COLUMN id SET GENERATED BY DEFAULT"],
            ),
            (
                "üretilmiş ifade kaldır (PG 13)",
                130000,
                l.clone(),
                with(&l, |d| colm(d, "total").generated = None),
                vec!["ALTER TABLE public.t ALTER COLUMN total DROP EXPRESSION"],
            ),
            (
                "üretilmiş ifade kaldır + varsayılan",
                130000,
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "total");
                    c.generated = None;
                    c.default = Some("0".into());
                }),
                vec![
                    "ALTER TABLE public.t ALTER COLUMN total DROP EXPRESSION",
                    "ALTER TABLE public.t ALTER COLUMN total SET DEFAULT (0\n)",
                ],
            ),
            (
                "üretilmiş ifade değişimi (PG 17)",
                170000,
                l.clone(),
                with(&l, |d| colm(d, "total").generated.as_mut().unwrap().expr = "qty * 3".into()),
                vec!["ALTER TABLE public.t ALTER COLUMN total SET EXPRESSION AS (qty * 3\n)"],
            ),
            (
                "sanal üretilmiş ifade değişimi (PG 18)",
                180000,
                with(&l, |d| colm(d, "total").generated.as_mut().unwrap().stored = false),
                with(&l, |d| {
                    let g = colm(d, "total").generated.as_mut().unwrap();
                    g.stored = false;
                    g.expr = "qty * 3".into();
                }),
                vec!["ALTER TABLE public.t ALTER COLUMN total SET EXPRESSION AS (qty * 3\n)"],
            ),
            (
                "kolon yorumu değişimi",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").comment = Some("isim".into())),
                vec!["COMMENT ON COLUMN public.t.caption IS 'isim'"],
            ),
            (
                "kolon yorumu ekleme",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "note").comment = Some("not".into())),
                vec!["COMMENT ON COLUMN public.t.note IS 'not'"],
            ),
            (
                "kolon yorumu silme",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").comment = None),
                vec!["COMMENT ON COLUMN public.t.caption IS NULL"],
            ),
            (
                "kolon yorumu boş = silme",
                120000,
                l.clone(),
                with(&l, |d| colm(d, "caption").comment = Some(String::new())),
                vec!["COMMENT ON COLUMN public.t.caption IS NULL"],
            ),
            ("sahip", 120000, l.clone(), with(&l, |d| d.owner = Some("bob".into())), vec!["ALTER TABLE public.t OWNER TO bob"]),
            (
                "tablespace",
                120000,
                l.clone(),
                with(&l, |d| d.tablespace = Some("slow".into())),
                vec!["ALTER TABLE public.t SET TABLESPACE slow"],
            ),
            (
                "tablespace pg_default",
                120000,
                l.clone(),
                with(&l, |d| d.tablespace = Some("pg_default".into())),
                vec!["ALTER TABLE public.t SET TABLESPACE pg_default"],
            ),
            (
                "tablespace ilk kez",
                120000,
                base(),
                TableDef { tablespace: Some("fast".into()), ..base() },
                vec!["ALTER TABLE public.t SET TABLESPACE fast"],
            ),
            ("UNLOGGED", 120000, l.clone(), unlogged.clone(), vec!["ALTER TABLE public.t SET UNLOGGED"]),
            ("LOGGED", 120000, unlogged, l.clone(), vec!["ALTER TABLE public.t SET LOGGED"]),
            (
                "fillfactor değişimi",
                120000,
                l.clone(),
                with(&l, |d| d.fillfactor = Some(80)),
                vec!["ALTER TABLE public.t SET (fillfactor = 80)"],
            ),
            (
                "fillfactor sıfırlama",
                120000,
                l.clone(),
                with(&l, |d| d.fillfactor = None),
                vec!["ALTER TABLE public.t RESET (fillfactor)"],
            ),
            (
                "tablo yorumu",
                120000,
                l.clone(),
                with(&l, |d| d.comment = Some("Yeni".into())),
                vec!["COMMENT ON TABLE public.t IS 'Yeni'"],
            ),
            (
                "tablo yorumu silme",
                120000,
                l.clone(),
                with(&l, |d| d.comment = None),
                vec!["COMMENT ON TABLE public.t IS NULL"],
            ),
        ];
        let mut out = String::new();
        for (caption, ver, old, new, want) in cases {
            let p = ok(&old, &new, &v(ver));
            assert_eq!(sqls(&p), want, "{caption}");
            out.push_str(&format!("## {caption} @{ver}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn combined_order_uses_new_names() {
        let old = loaded();
        let new = with(&old, |d| {
            d.name = "t2".into();
            d.schema = "s2".into();
            d.columns.retain(|c| c.name != "note");
            {
                // Tip (eski adla) + yeniden adlandırma + yorum (yeni adla) aynı kolonda.
                let c = colm(d, "caption");
                c.data_type = "character varying(100)".into();
                c.name = "full_name".into();
                c.comment = Some("isim".into());
            }
            {
                let c = colm(d, "amount");
                c.data_type = "bigint".into();
                c.using = Some("amount::bigint".into());
                c.default = Some("5".into());
                c.not_null = true;
            }
            colm(d, "seq").identity = Some(IdentityKind::Always);
            colm(d, "total").generated.as_mut().unwrap().expr = "qty * 3".into();
            d.columns.push(ColumnDef { not_null: true, default: Some("1".into()), ..col("extra", "integer") });
            d.owner = Some("bob".into());
            d.tablespace = Some("slow".into());
            d.persistence = Persistence::Unlogged;
            d.fillfactor = Some(80);
            d.comment = Some("Yeni".into());
        });
        let p = ok(&old, &new, &v(170000));
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t RENAME TO t2",
                "ALTER TABLE public.t2 SET SCHEMA s2",
                "ALTER TABLE s2.t2 DROP COLUMN note",
                "ALTER TABLE s2.t2 ALTER COLUMN caption TYPE character varying(100) COLLATE \"C\"",
                "ALTER TABLE s2.t2 ALTER COLUMN amount TYPE bigint USING (amount::bigint\n)",
                "ALTER TABLE s2.t2 RENAME COLUMN caption TO full_name",
                "ALTER TABLE s2.t2 ADD COLUMN extra integer DEFAULT (1\n) NOT NULL",
                "ALTER TABLE s2.t2 ALTER COLUMN total SET EXPRESSION AS (qty * 3\n)",
                "ALTER TABLE s2.t2 ALTER COLUMN amount SET DEFAULT (5\n)",
                "ALTER TABLE s2.t2 ALTER COLUMN amount SET NOT NULL",
                "ALTER TABLE s2.t2 ALTER COLUMN seq SET GENERATED ALWAYS",
                "ALTER TABLE s2.t2 OWNER TO bob",
                "ALTER TABLE s2.t2 SET TABLESPACE slow",
                "ALTER TABLE s2.t2 SET UNLOGGED",
                "ALTER TABLE s2.t2 SET (fillfactor = 80)",
                "COMMENT ON TABLE s2.t2 IS 'Yeni'",
                "COMMENT ON COLUMN s2.t2.full_name IS 'isim'",
            ]
        );
        assert_eq!(p.warnings, [REWRITE_WARNING, LOCK_TIMEOUT_WARNING]);
        assert_eq!(p.risk(), Risk::DataLoss);
        // Deterministik.
        assert_eq!(ok(&old, &new, &v(170000)), p);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn column_renames_are_ordered_and_cycles_rejected() {
        let ab = TableDef {
            columns: vec![lc(1, "a", "integer"), lc(2, "b", "integer"), lc(3, "c", "integer")],
            ..base()
        };
        // Zincir: a→b, b→c (c silinir) → önce c silinir, sonra b→c, sonra a→b.
        let chain = with(&ab, |d| {
            d.columns.retain(|c| c.name != "c");
            d.columns[0].name = "b".into();
            d.columns[1].name = "c".into();
        });
        assert_eq!(
            sqls(&ok(&ab, &chain, &ctx())),
            [
                "ALTER TABLE public.t DROP COLUMN c",
                "ALTER TABLE public.t RENAME COLUMN b TO c",
                "ALTER TABLE public.t RENAME COLUMN a TO b",
            ]
        );
        // Silinen kolonun adını alan yeniden adlandırma: silme önce.
        let take = with(&ab, |d| {
            d.columns.retain(|c| c.name != "b");
            d.columns[0].name = "b".into();
        });
        assert_eq!(
            sqls(&ok(&ab, &take, &ctx())),
            ["ALTER TABLE public.t DROP COLUMN b", "ALTER TABLE public.t RENAME COLUMN a TO b"]
        );
        // Yeniden adlandırılan kolonun eski adıyla yeni kolon: ekleme sonra.
        let reuse = with(&ab, |d| {
            d.columns[0].name = "a_old".into();
            d.columns.push(col("a", "text"));
        });
        assert_eq!(
            sqls(&ok(&ab, &reuse, &ctx())),
            ["ALTER TABLE public.t RENAME COLUMN a TO a_old", "ALTER TABLE public.t ADD COLUMN a text"]
        );
        // Takas (döngü): doğrulama hatası.
        let swap = with(&ab, |d| {
            d.columns[0].name = "b".into();
            d.columns[1].name = "a".into();
        });
        let e = errs(&ab, &swap, &ctx());
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].path, "columns[0].name");
        assert!(e[0].message.contains("döngüsel"));
    }

    #[test]
    fn generated_transitions_and_version_gates() {
        let l = loaded();
        let drop_expr = with(&l, |d| colm(d, "total").generated = None);
        let set_expr = with(&l, |d| colm(d, "total").generated.as_mut().unwrap().expr = "qty * 3".into());
        let virt_old = with(&l, |d| colm(d, "total").generated.as_mut().unwrap().stored = false);
        let virt_set = with(&virt_old, |d| colm(d, "total").generated.as_mut().unwrap().expr = "qty * 3".into());
        let virt_drop = with(&virt_old, |d| colm(d, "total").generated = None);
        let to_gen = with(&l, |d| {
            colm(d, "note").generated = Some(GeneratedDef { expr: "'x'".into(), stored: true });
        });
        let p_gen = "columns[3].generated";
        // (etiket, eski, yeni, sürüm, None = geçerli / Some(ileti parçası))
        let cases: Vec<(&str, &TableDef, &TableDef, u32, Option<&str>)> = vec![
            ("DROP EXPRESSION 12", &l, &drop_expr, 120000, Some("PG 13+")),
            ("DROP EXPRESSION 13", &l, &drop_expr, 130000, None),
            ("DROP EXPRESSION 18", &l, &drop_expr, 180000, None),
            ("SET EXPRESSION 12", &l, &set_expr, 120000, Some("PG 17+")),
            ("SET EXPRESSION 13", &l, &set_expr, 130000, Some("PG 17+")),
            ("SET EXPRESSION 17", &l, &set_expr, 170000, None),
            ("SET EXPRESSION 18", &l, &set_expr, 180000, None),
            ("sanal SET EXPRESSION 18", &virt_old, &virt_set, 180000, None),
            ("sanal DROP EXPRESSION 18", &virt_old, &virt_drop, 180000, Some("STORED")),
            ("STORED → VIRTUAL", &l, &virt_old, 180000, Some("STORED/VIRTUAL")),
            ("VIRTUAL → STORED", &virt_old, &l, 180000, Some("STORED/VIRTUAL")),
            ("sanal yeni taraf 17", &virt_old, &virt_set, 170000, Some("PG 18+")),
        ];
        for (caption, old, new, ver, want) in cases {
            match want {
                None => {
                    ok(old, new, &v(ver));
                }
                Some(m) => {
                    let e = errs(old, new, &v(ver));
                    assert_eq!(e[0].path, p_gen, "{caption}: {e:?}");
                    assert!(e[0].message.contains(m), "{caption}: {e:?}");
                }
            }
        }
        // Mevcut kolonu üretilmiş kolona çevirmek desteklenmez.
        let e = errs(&l, &to_gen, &v(180000));
        assert_eq!(e[0].path, "columns[4].generated");
        assert!(e[0].message.contains("yeni kolon ekleyin"), "{e:?}");
        // Kimlik → üretilmiş de aynı kural.
        let e = errs(
            &l,
            &with(&l, |d| {
                let c = colm(d, "seq");
                c.identity = None;
                c.generated = Some(GeneratedDef { expr: "1".into(), stored: true });
            }),
            &v(180000),
        );
        assert_eq!(e[0].path, "columns[6].generated");
        // Yeni sanal kolon eklemek PG 18 ister (create kuralı).
        let add_virt = with(&l, |d| {
            d.columns.push(ColumnDef { generated: Some(GeneratedDef { expr: "1".into(), stored: false }), ..col("x", "integer") })
        });
        assert_eq!(errs(&l, &add_virt, &v(170000))[0].path, "columns[7].generated");
        ok(&l, &add_virt, &v(180000));
        // Üretilmiş kolonsuz tablo PG 10'da sıradan alter'larla çalışır.
        let plain = TableDef { columns: vec![lc(1, "a", "integer")], ..base() };
        ok(&plain, &with(&plain, |d| d.columns[0].not_null = true), &v(100000));
    }

    #[test]
    fn attnum_matching_errors() {
        let l = loaded();
        // Bayat attnum: eski tanımda yok.
        let e = errs(&l, &with(&l, |d| d.columns[4].attnum = Some(99)), &ctx());
        assert_eq!(e[0].path, "columns[4].attnum");
        assert!(e[0].message.contains("99"));
        // Yinelenen attnum (yeni taraf).
        let e = errs(&l, &with(&l, |d| d.columns[5].attnum = Some(5)), &ctx());
        assert_eq!(e[0].path, "columns[5].attnum");
        // Eski tarafta attnum yok / yinelenen.
        let e = errs(&with(&l, |d| d.columns[2].attnum = None), &l, &ctx());
        assert_eq!(e[0].path, "old.columns[2].attnum");
        let e = errs(&with(&l, |d| d.columns[3].attnum = Some(1)), &l, &ctx());
        assert_eq!(e[0].path, "old.columns[3].attnum");
        // attnum None = yeni kolon; korunan kolonla aynı ad yeni tarafın yinelenme kuralına takılır.
        let e = errs(&l, &with(&l, |d| d.columns.push(col("note", "text"))), &ctx());
        assert_eq!(e[0].path, "columns[7].name");
        // Aynı adla sil + yeniden ekle (attnum None): DROP + ADD (yeniden adlandırma değil).
        let readd = with(&l, |d| {
            d.columns.retain(|c| c.name != "note");
            d.columns.push(col("note", "integer"));
        });
        assert_eq!(
            sqls(&ok(&l, &readd, &ctx())),
            ["ALTER TABLE public.t DROP COLUMN note", "ALTER TABLE public.t ADD COLUMN note integer"]
        );
    }

    #[test]
    fn column_order_warning() {
        let l = loaded();
        // Korunan kolonların sırası değişti → uyarı, ifade yok.
        let swapped = with(&l, |d| d.columns.swap(4, 5));
        let p = ok(&l, &swapped, &ctx());
        assert!(p.statements.is_empty());
        assert_eq!(p.warnings, [COLUMN_ORDER_WARNING]);
        // Yeni kolon araya eklendi → uyarı + ADD (sona eklenir).
        let mid = with(&l, |d| d.columns.insert(1, col("x", "text")));
        let p = ok(&l, &mid, &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD COLUMN x text"]);
        assert_eq!(p.warnings, [COLUMN_ORDER_WARNING]);
        // Silme sırayı bozmaz; sona ekleme uyarı vermez.
        let p = ok(
            &l,
            &with(&l, |d| {
                d.columns.remove(4);
                d.columns.push(col("x", "text"));
            }),
            &ctx(),
        );
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn using_without_type_change_is_ignored_with_warning() {
        let l = loaded();
        let p = ok(&l, &with(&l, |d| colm(d, "note").using = Some("note || 'x'".into())), &ctx());
        assert!(p.statements.is_empty());
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("'note'") && p.warnings[0].contains("USING"), "{:?}", p.warnings);
        // Yeni kolonda da yoksayılır (ADD COLUMN USING almaz).
        let p = ok(&l, &with(&l, |d| d.columns.push(ColumnDef { using: Some("1".into()), ..col("x", "text") })), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD COLUMN x text"]);
        assert_eq!(p.warnings.len(), 1);
        // Yoksayılsa bile serbest SQL kapısından geçer.
        let e = errs(&l, &with(&l, |d| colm(d, "note").using = Some("1; DROP TABLE t".into())), &ctx());
        assert_eq!(e[0].path, "columns[4].using");
        // Collation değişimi USING'i anlamlı kılar.
        let p = ok(
            &l,
            &with(&l, |d| {
                let c = colm(d, "caption");
                c.collation = Some("POSIX".into());
                c.using = Some("caption".into());
            }),
            &ctx(),
        );
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ALTER COLUMN caption TYPE text COLLATE \"POSIX\" USING (caption\n)"]);
        assert_eq!(p.warnings, [REWRITE_WARNING, LOCK_TIMEOUT_WARNING]);
    }

    #[test]
    fn partition_and_inheritance_protection() {
        for flag in ["isPartition", "inherits"] {
            let set = |d: &mut TableDef| {
                if flag == "isPartition" {
                    d.is_partition = true
                } else {
                    d.inherits = true
                }
            };
            let old = with(&loaded(), set);
            // Serbest: ad, şema, sahip, tablespace, yorumlar (kolon yorumu dahil).
            let free = with(&old, |d| {
                d.name = "t2".into();
                d.schema = "s2".into();
                d.owner = Some("bob".into());
                d.tablespace = Some("slow".into());
                d.comment = Some("x".into());
                colm(d, "note").comment = Some("n".into());
            });
            let p = ok(&old, &free, &ctx());
            assert_eq!(p.statements.len(), 6, "{flag}: {}", render(&p));
            // Kolon farkları → `columns` hatası.
            for (caption, new) in [
                ("silme", with(&old, |d| d.columns.retain(|c| c.name != "note"))),
                ("ekleme", with(&old, |d| d.columns.push(col("x", "text")))),
                ("tip", with(&old, |d| colm(d, "note").data_type = "integer".into())),
                ("ad", with(&old, |d| colm(d, "note").name = "memo".into())),
                ("NOT NULL", with(&old, |d| colm(d, "note").not_null = true)),
                ("varsayılan", with(&old, |d| colm(d, "note").default = Some("'x'".into()))),
                ("kimlik", with(&old, |d| colm(d, "seq").identity = Some(IdentityKind::Always))),
                ("collation", with(&old, |d| colm(d, "caption").collation = None)),
            ] {
                let e = errs(&old, &new, &ctx());
                assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("columns", MANAGED_MSG), "{flag} {caption}");
            }
            // Kısıt farkı → `constraints` hatası.
            let e = errs(&old, &with(&old, |d| d.constraints[0].name = Some("pk2".into())), &ctx());
            assert_eq!((e[0].path.as_str(), e[0].message.as_str()), ("constraints", MANAGED_MSG), "{flag}");
            // Yükleme bayrağı değiştirilemez.
            let e = errs(
                &old,
                &with(&old, |d| {
                    d.is_partition = false;
                    d.inherits = false;
                }),
                &ctx(),
            );
            assert_eq!(e[0].path, flag);
        }
        // Bölümleme anahtarı değiştirilemez (ekleme/kaldırma/değişim).
        let parted = TableDef {
            columns: vec![lc(1, "id", "bigint"), lc(2, "created", "date")],
            partition_by: Some(PartitionByDef {
                strategy: PartitionStrategy::Range,
                keys: vec![PartitionKey::Column { name: "created".into() }],
            }),
            ..base()
        };
        for new in [
            with(&parted, |d| d.partition_by.as_mut().unwrap().strategy = PartitionStrategy::List),
            with(&parted, |d| d.partition_by.as_mut().unwrap().keys[0] = PartitionKey::Column { name: "id".into() }),
            with(&parted, |d| d.partition_by = None),
        ] {
            assert_eq!(errs(&parted, &new, &ctx())[0].path, "partitionBy");
        }
        let unparted = with(&parted, |d| d.partition_by = None);
        assert_eq!(errs(&unparted, &parted, &ctx())[0].path, "partitionBy");
        // Bölümlenmiş ANA tabloda kolon değişiklikleri serbesttir (bölümlere yayılır).
        let p = ok(&parted, &with(&parted, |d| d.columns.push(col("x", "text"))), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD COLUMN x text"]);
    }

    #[test]
    fn old_side_validates_identity_only() {
        // Create doğrulamasına takılan yüklenmiş tanım: garip tip, kapanmamış varsayılan, dengesiz üretilmiş ifade,
        // NOT NULL tutarsızlığı (kimlik), geçersiz sahip/tablespace/fillfactor, NUL'lu yorum. Hiçbiri SQL'e girmez.
        let old = TableDef {
            owner: Some("\n".into()),
            tablespace: Some(String::new()),
            fillfactor: Some(1),
            comment: Some("a\0".into()),
            columns: vec![
                ColumnDef { identity: Some(IdentityKind::Always), ..lc(1, "id", "bigint") },
                ColumnDef { default: Some("x'".into()), ..lc(2, "a", "garip tip ;") },
                ColumnDef { generated: Some(GeneratedDef { expr: "1)".into(), stored: true }), ..lc(3, "g", "integer") },
            ],
            ..base()
        };
        let new = TableDef {
            owner: Some("app".into()),
            tablespace: Some("fast".into()),
            fillfactor: None,
            comment: None,
            columns: vec![
                ColumnDef { not_null: true, identity: Some(IdentityKind::Always), ..lc(1, "id", "bigint") },
                ColumnDef { default: Some("0".into()), ..lc(2, "a", "integer") },
                ColumnDef { generated: Some(GeneratedDef { expr: "id + 1".into(), stored: true }), ..lc(3, "g", "integer") },
            ],
            ..base()
        };
        let p = ok(&old, &new, &ctx());
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t ALTER COLUMN a DROP DEFAULT",
                "ALTER TABLE public.t ALTER COLUMN a TYPE integer",
                "ALTER TABLE public.t ALTER COLUMN id SET NOT NULL",
                "ALTER TABLE public.t ALTER COLUMN a SET DEFAULT (0\n)",
                "ALTER TABLE public.t ALTER COLUMN g SET EXPRESSION AS (id + 1\n)",
                "ALTER TABLE public.t OWNER TO app",
                "ALTER TABLE public.t SET TABLESPACE fast",
                "ALTER TABLE public.t RESET (fillfactor)",
                "COMMENT ON TABLE public.t IS NULL",
            ]
        );
        // Eski tarafın SQL'e giren alanları doğrulanır (`old.` önekiyle).
        for (bad, path) in [
            (TableDef { schema: "pg_catalog".into(), ..base() }, "old.schema"),
            (TableDef { name: "t\0".into(), ..base() }, "old.name"),
            (TableDef { name: String::new(), ..base() }, "old.name"),
            (TableDef { columns: vec![lc(1, "i\0d", "integer")], ..base() }, "old.columns[0].name"),
            (TableDef { columns: vec![lc(1, "", "integer")], ..base() }, "old.columns[0].name"),
            (TableDef { columns: vec![lc(1, "a", "integer"), lc(2, "a", "text")], ..base() }, "old.columns[1].name"),
        ] {
            let e = errs(&bad, &base(), &ctx());
            assert_eq!(e[0].path, path, "{e:?}");
        }
        // Yeni taraf tam doğrulanır (ve eski taraftan önce raporlanır).
        let e = errs(
            &TableDef { name: "\n".into(), ..base() },
            &with(&base(), |d| d.columns[0].data_type = "integer; DROP TABLE t".into()),
            &ctx(),
        );
        assert_eq!(e[0].path, "columns[0].dataType");
        // Silinen kolonu kullanan kısıt yeni tarafta reddedilir.
        let l = loaded();
        let e = errs(&l, &with(&l, |d| d.columns.retain(|c| c.name != "id")), &ctx());
        assert_eq!(e[0].path, "constraints[0].columns[0]");
        // Create-only kurallar alter'da uygulanmaz: yüklenmiş `notValid` kısıtı kabul.
        let nv = with(&l, |d| {
            d.constraints.push(ConstraintDef {
                oid: Some(101),
                name: Some("ck".into()),
                kind: ConstraintKind::Check { expr: "qty >= 0".into(), no_inherit: false },
                deferrable: false,
                initially_deferred: false,
                not_valid: true,
            })
        });
        assert!(ok(&nv, &nv, &ctx()).statements.is_empty());
    }

    #[test]
    fn tablespace_clearing_is_an_error() {
        let l = loaded();
        let e = errs(&l, &with(&l, |d| d.tablespace = None), &ctx());
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].path, "tablespace");
        assert!(e[0].message.contains("pg_default"));
        // Tablespace'siz eski tanımda None → None değişiklik değildir.
        assert!(ok(&base(), &base(), &ctx()).statements.is_empty());
    }

    /// Her ifade türünün riski tek tabloda: (ifadeyi tanıtan parça, eski, yeni, sürüm, beklenen risk).
    #[test]
    fn risk_classes_for_every_statement_kind() {
        let l = loaded();
        let unlogged = TableDef { persistence: Persistence::Unlogged, ..loaded() };
        let add = |c: ColumnDef| with(&l, |d| d.columns.push(c));
        let generated = |stored| Some(GeneratedDef { expr: "qty + 1".into(), stored });
        // Yüklenmiş (oid'li) ya da yeni (oid'siz) CHECK.
        let check = |oid, not_valid| ConstraintDef {
            oid,
            name: Some("ck".into()),
            kind: ConstraintKind::Check { expr: "qty >= 0".into(), no_inherit: false },
            deferrable: false,
            initially_deferred: false,
            not_valid,
        };
        let with_check = |oid, not_valid| with(&l, |d| d.constraints.push(check(oid, not_valid)));
        let cases: Vec<(&str, TableDef, TableDef, u32, Risk)> = vec![
            (" RENAME TO ", l.clone(), with(&l, |d| d.name = "t2".into()), 120000, Risk::Safe),
            (" SET SCHEMA ", l.clone(), with(&l, |d| d.schema = "s".into()), 120000, Risk::Safe),
            (" DROP COLUMN ", l.clone(), with(&l, |d| d.columns.retain(|c| c.name != "note")), 120000, Risk::DataLoss),
            (" TYPE ", l.clone(), with(&l, |d| colm(d, "note").data_type = "integer".into()), 120000, Risk::DataLoss),
            (" RENAME COLUMN ", l.clone(), with(&l, |d| colm(d, "note").name = "m".into()), 120000, Risk::Safe),
            (" ADD COLUMN x text", l.clone(), add(col("x", "text")), 120000, Risk::Safe),
            (
                " ADD COLUMN x text NOT NULL",
                l.clone(),
                add(ColumnDef { not_null: true, ..col("x", "text") }),
                120000,
                Risk::Destructive,
            ),
            (
                " ADD COLUMN x text DEFAULT",
                l.clone(),
                add(ColumnDef { not_null: true, default: Some("''".into()), ..col("x", "text") }),
                120000,
                Risk::Safe,
            ),
            (
                " ADD COLUMN x bigint GENERATED BY DEFAULT AS IDENTITY",
                l.clone(),
                add(ColumnDef { not_null: true, identity: Some(IdentityKind::ByDefault), ..col("x", "bigint") }),
                120000,
                Risk::Safe,
            ),
            (
                " ADD COLUMN x integer GENERATED ALWAYS AS (qty + 1\n) STORED",
                l.clone(),
                add(ColumnDef { generated: generated(true), ..col("x", "integer") }),
                120000,
                Risk::Safe,
            ),
            (
                " ADD COLUMN x integer GENERATED ALWAYS AS (qty + 1\n) STORED NOT NULL",
                l.clone(),
                add(ColumnDef { not_null: true, generated: generated(true), ..col("x", "integer") }),
                120000,
                Risk::Destructive,
            ),
            (" SET NOT NULL", l.clone(), with(&l, |d| colm(d, "note").not_null = true), 120000, Risk::Destructive),
            (" DROP NOT NULL", l.clone(), with(&l, |d| colm(d, "qty").not_null = false), 120000, Risk::Safe),
            (" SET DEFAULT ", l.clone(), with(&l, |d| colm(d, "note").default = Some("''".into())), 120000, Risk::Safe),
            (" DROP DEFAULT", l.clone(), with(&l, |d| colm(d, "qty").default = None), 120000, Risk::Safe),
            (
                " ADD GENERATED ",
                l.clone(),
                with(&l, |d| {
                    let c = colm(d, "amount");
                    c.identity = Some(IdentityKind::Always);
                    c.not_null = true;
                }),
                120000,
                Risk::Safe,
            ),
            (" SET GENERATED ", l.clone(), with(&l, |d| colm(d, "seq").identity = Some(IdentityKind::Always)), 120000, Risk::Safe),
            (" DROP IDENTITY", l.clone(), with(&l, |d| colm(d, "seq").identity = None), 120000, Risk::Safe),
            (" DROP EXPRESSION", l.clone(), with(&l, |d| colm(d, "total").generated = None), 130000, Risk::Safe),
            (
                " SET EXPRESSION ",
                l.clone(),
                with(&l, |d| colm(d, "total").generated.as_mut().unwrap().expr = "1".into()),
                170000,
                Risk::Destructive,
            ),
            (" OWNER TO ", l.clone(), with(&l, |d| d.owner = Some("b".into())), 120000, Risk::Safe),
            (" SET TABLESPACE ", l.clone(), with(&l, |d| d.tablespace = Some("s".into())), 120000, Risk::Safe),
            (" SET UNLOGGED", l.clone(), unlogged.clone(), 120000, Risk::Destructive),
            (" SET LOGGED", unlogged, l.clone(), 120000, Risk::Safe),
            (" SET (fillfactor", l.clone(), with(&l, |d| d.fillfactor = Some(50)), 120000, Risk::Safe),
            (" RESET (fillfactor)", l.clone(), with(&l, |d| d.fillfactor = None), 120000, Risk::Safe),
            ("COMMENT ON TABLE ", l.clone(), with(&l, |d| d.comment = None), 120000, Risk::Safe),
            ("COMMENT ON COLUMN ", l.clone(), with(&l, |d| colm(d, "note").comment = Some("c".into())), 120000, Risk::Safe),
            // Kısıt ifadeleri (Task 4).
            (" DROP CONSTRAINT ", l.clone(), with(&l, |d| d.constraints.clear()), 120000, Risk::Destructive),
            (" RENAME CONSTRAINT ", l.clone(), with(&l, |d| d.constraints[0].name = Some("pk2".into())), 120000, Risk::Safe),
            (" ADD CONSTRAINT ck CHECK ", l.clone(), with_check(None, false), 120000, Risk::Safe),
            (" ADD CONSTRAINT ck CHECK (qty >= 0
) NOT VALID", l.clone(), with_check(None, true), 120000, Risk::Safe),
            (" VALIDATE CONSTRAINT ", with_check(Some(101), true), with_check(Some(101), false), 120000, Risk::Safe),
        ];
        let mut seen = HashSet::new();
        for (marker, old, new, ver, risk) in cases {
            let p = ok(&old, &new, &v(ver));
            // Her üretilen ifade türü tabloda (ilk iki sözcükten sonraki eylem) bir kez geçer.
            for s in &p.statements {
                seen.insert(statement_kind(&s.sql));
            }
            let hits: Vec<&Statement> = p.statements.iter().filter(|s| s.sql.contains(marker)).collect();
            assert_eq!(hits.len(), 1, "{marker:?}\n{}", render(&p));
            assert_eq!(hits[0].risk, risk, "{marker:?}");
            // Destructive/DataLoss ifadelerin ve tüm kısıt ifadelerinin hepsi notludur.
            if risk != Risk::Safe || marker.contains("CONSTRAINT") {
                assert!(hits[0].note.is_some(), "{marker:?}");
            }
        }
        // Tablo `build`'in ürettiği her ifade türünü kapsar.
        let all: HashSet<&str> = [
            "RENAME TO", "SET SCHEMA", "DROP CONSTRAINT", "DROP COLUMN", "ALTER COLUMN DROP DEFAULT",
            "ALTER COLUMN TYPE", "RENAME COLUMN", "ADD COLUMN", "ALTER COLUMN DROP IDENTITY",
            "ALTER COLUMN DROP EXPRESSION", "ALTER COLUMN SET DEFAULT", "ALTER COLUMN SET NOT NULL",
            "ALTER COLUMN DROP NOT NULL", "ALTER COLUMN ADD GENERATED", "ALTER COLUMN SET GENERATED",
            "ALTER COLUMN SET EXPRESSION", "RENAME CONSTRAINT", "ADD CONSTRAINT", "VALIDATE CONSTRAINT", "OWNER TO",
            "SET TABLESPACE", "SET LOGGED", "SET UNLOGGED", "SET (fillfactor", "RESET (fillfactor)",
            "COMMENT ON TABLE", "COMMENT ON COLUMN",
        ]
        .into_iter()
        .collect();
        assert_eq!(seen, all);
    }

    /// İfade türü: tablo adından sonraki eylem sözcükleri (kolon/kısıt adı hariç).
    fn statement_kind(sql: &str) -> &'static str {
        const KINDS: [&str; 27] = [
            "ALTER COLUMN DROP DEFAULT", "ALTER COLUMN TYPE", "ALTER COLUMN DROP IDENTITY", "ALTER COLUMN DROP EXPRESSION",
            "ALTER COLUMN SET DEFAULT", "ALTER COLUMN SET NOT NULL", "ALTER COLUMN DROP NOT NULL",
            "ALTER COLUMN ADD GENERATED", "ALTER COLUMN SET GENERATED", "ALTER COLUMN SET EXPRESSION", "RENAME TO",
            "SET SCHEMA", "DROP CONSTRAINT", "DROP COLUMN", "RENAME COLUMN", "ADD COLUMN", "RENAME CONSTRAINT",
            "ADD CONSTRAINT", "VALIDATE CONSTRAINT", "OWNER TO", "SET TABLESPACE", "SET LOGGED", "SET UNLOGGED",
            "SET (fillfactor", "RESET (fillfactor)", "COMMENT ON TABLE", "COMMENT ON COLUMN",
        ];
        if sql.starts_with("COMMENT ON TABLE") {
            return "COMMENT ON TABLE";
        }
        if sql.starts_with("COMMENT ON COLUMN") {
            return "COMMENT ON COLUMN";
        }
        // "ALTER TABLE <q> <eylem…>": kolon ifadelerinde kolon adı atlanır.
        let rest = sql.splitn(4, ' ').nth(3).unwrap_or_default();
        let rest = match rest.strip_prefix("ALTER COLUMN ") {
            Some(r) => format!("ALTER COLUMN {}", r.split_once(' ').map(|x| x.1).unwrap_or_default()),
            None => rest.to_string(),
        };
        // ADD CONSTRAINT'in adsız biçimi (ADD CHECK …) de aynı türdür.
        if rest.starts_with("ADD ") && !rest.starts_with("ADD COLUMN") {
            return "ADD CONSTRAINT";
        }
        KINDS
            .into_iter()
            .find(|k| rest.starts_with(k))
            .unwrap_or_else(|| panic!("bilinmeyen ifade türü: {sql}"))
    }

    #[test]
    fn notes_and_plan_warnings() {
        let l = loaded();
        let note = |old: &TableDef, new: &TableDef, ver: u32| {
            ok(old, new, &v(ver)).statements[0].note.clone().unwrap_or_default()
        };
        assert_eq!(note(&l, &with(&l, |d| d.columns.retain(|c| c.name != "note")), 120000), DROP_COLUMN_NOTE);
        assert_eq!(
            note(&l, &with(&l, |d| colm(d, "note").data_type = "integer".into()), 120000),
            format!("{TYPE_NOTE}{TYPE_USING_HINT}")
        );
        assert_eq!(
            note(
                &l,
                &with(&l, |d| {
                    let c = colm(d, "note");
                    c.data_type = "integer".into();
                    c.using = Some("note::integer".into());
                }),
                120000
            ),
            format!("{TYPE_NOTE}{TYPE_USING_NOTE}")
        );
        assert_eq!(
            note(&l, &with(&l, |d| d.columns.push(ColumnDef { not_null: true, ..col("x", "text") })), 120000),
            ADD_NOT_NULL_NOTE
        );
        // Yeniden yazım uyarısı + lock_timeout: TYPE, SET TABLESPACE, SET LOGGED/UNLOGGED, STORED SET EXPRESSION.
        let both = [REWRITE_WARNING, LOCK_TIMEOUT_WARNING];
        for (new, ver) in [
            (with(&l, |d| colm(d, "note").data_type = "integer".into()), 120000),
            (with(&l, |d| d.tablespace = Some("s".into())), 120000),
            (with(&l, |d| d.persistence = Persistence::Unlogged), 120000),
            (with(&l, |d| colm(d, "total").generated.as_mut().unwrap().expr = "1".into()), 170000),
        ] {
            assert_eq!(ok(&l, &new, &v(ver)).warnings, both);
        }
        // SET NOT NULL tarar (yeniden yazmaz): yalnızca lock_timeout.
        assert_eq!(ok(&l, &with(&l, |d| colm(d, "note").not_null = true), &ctx()).warnings, [LOCK_TIMEOUT_WARNING]);
        // Ucuz değişiklikler uyarısız.
        for new in [
            with(&l, |d| d.name = "x".into()),
            with(&l, |d| d.columns.retain(|c| c.name != "note")),
            with(&l, |d| colm(d, "qty").default = None),
            with(&l, |d| d.fillfactor = Some(50)),
        ] {
            assert!(ok(&l, &new, &ctx()).warnings.is_empty());
        }
    }

    /// Saldırgan girdi matrisi: alter'ın HER yeni metin alanında girdi ya o alanda reddedilir ya da plan tek ifadeli,
    /// temiz ve dengeli kalır.
    #[test]
    fn hostile_input_matrix() {
        #[derive(Clone, Copy, PartialEq)]
        enum K {
            Name,
            Type,
            Free,
            Comment,
        }
        type Set = fn(&mut TableDef, String);
        // Temel yeni tanım: `amount` tipi değişiyor (USING anlamlı), sona `extra` ekleniyor (indeks 7).
        let old = loaded();
        let new_base = with(&old, |d| {
            colm(d, "amount").data_type = "bigint".into();
            d.columns.push(col("extra", "text"));
        });
        let fields: Vec<(&str, K, &str, Set)> = vec![
            ("name", K::Name, "t2", |d, v| d.name = v),
            ("schema", K::Name, "s2", |d, v| d.schema = v),
            ("owner", K::Name, "bob", |d, v| d.owner = Some(v)),
            ("tablespace", K::Name, "slow", |d, v| d.tablespace = Some(v)),
            ("comment", K::Comment, "Yeni", |d, v| d.comment = Some(v)),
            ("columns[1].name", K::Name, "full", |d, v| d.columns[1].name = v),
            ("columns[7].name", K::Name, "extra2", |d, v| d.columns[7].name = v),
            ("columns[4].dataType", K::Type, "integer", |d, v| d.columns[4].data_type = v),
            ("columns[7].dataType", K::Type, "integer", |d, v| d.columns[7].data_type = v),
            ("columns[4].default", K::Free, "'x'", |d, v| d.columns[4].default = Some(v)),
            ("columns[7].default", K::Free, "'x'", |d, v| d.columns[7].default = Some(v)),
            ("columns[5].using", K::Free, "amount::bigint", |d, v| d.columns[5].using = Some(v)),
            ("columns[3].generated", K::Free, "qty * 3", |d, v| {
                d.columns[3].generated.as_mut().unwrap().expr = v
            }),
            ("columns[7].generated", K::Free, "qty + 1", |d, v| {
                d.columns[7].generated = Some(GeneratedDef { expr: v, stored: true })
            }),
            ("columns[1].collation", K::Name, "POSIX", |d, v| d.columns[1].collation = Some(v)),
            ("columns[4].comment", K::Comment, "c", |d, v| d.columns[4].comment = Some(v)),
            ("columns[7].comment", K::Comment, "c", |d, v| d.columns[7].comment = Some(v)),
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
            "integer NOT NULL",
            "integer USING c",
            "text) WITH (oids = true",
            "\"My Type\"[]",
            "numeric(10,2)",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain(["a".repeat(63), "a".repeat(64), "ş".repeat(32)])
        .collect();
        let mut accepted = 0;
        for (path, kind, benign, set) in &fields {
            let mut b = new_base.clone();
            set(&mut b, benign.to_string());
            let base_len = ok(&old, &b, &ctx()).statements.len();
            for h in &hostile {
                let mut d = new_base.clone();
                set(&mut d, h.clone());
                let caption = format!("{path} ← {h:?}");
                match balter(&o(old.clone()), &o(d), &ctx()) {
                    Ok(p) => {
                        accepted += 1;
                        clean(&p);
                        // Boş yorum `None` sayılır (eklenen kolonda ifade düşer); diğerlerinde ifade sayısı sabit.
                        let n = p.statements.len();
                        assert!(
                            n == base_len || (*kind == K::Comment && h.is_empty() && n + 1 == base_len),
                            "{caption}\n{}",
                            render(&p)
                        );
                        assert!(!p.statements.iter().any(|s| s.sql.ends_with(" IS ''")), "{caption}");
                        assert!(!h.contains('\0'), "{caption}");
                        match kind {
                            K::Name => assert!(
                                !h.is_empty()
                                    && h.len() <= 63
                                    && !h.chars().any(|c| c.is_ascii_control() || is_spoofing_format(c)),
                                "{caption}"
                            ),
                            K::Type => assert!(validate_type(h).is_ok(), "{caption}"),
                            K::Free => {
                                let r = scan_semicolons(h);
                                assert!(
                                    r.is_clean() && r.parens_balanced() && !r.ends_in_line_comment && !h.trim().is_empty(),
                                    "{caption}"
                                );
                            }
                            K::Comment => {}
                        }
                    }
                    Err(e) => assert_eq!(e[0].path, *path, "{caption}: {e:?}"),
                }
            }
        }
        assert!(accepted > 100, "{accepted}");
    }

    #[test]
    fn composed_free_sql_statements_are_scanned() {
        // Alan kapısı `--` ile biten alanı zaten reddeder; `wrap` kapanışı yeni satıra koyar. Nihai ifade taraması
        // ikinci savunma hattıdır: kabul edilen her yorumlu alan nihai ifadede dengeli kalır.
        let l = loaded();
        let new = with(&l, |d| {
            let c = colm(d, "amount");
            c.data_type = "bigint".into();
            c.using = Some("amount -- dönüşüm\n::bigint".into());
            c.default = Some("1 -- bir\n".into());
            colm(d, "total").generated.as_mut().unwrap().expr = "qty -- x\n* 3".into();
            d.columns.push(ColumnDef { default: Some("2 /* ) */".into()), ..col("x", "integer") });
        });
        let p = ok(&l, &new, &ctx());
        assert_eq!(p.statements.len(), 4, "{}", render(&p));
        for s in &p.statements {
            assert!(composed_is_clean(&s.sql), "{}", s.sql);
        }
        // Alan içi dengesizlik birleşik ifadede dengeli görünebilir: alan kapısı yakalar.
        let e = errs(&l, &with(&l, |d| colm(d, "note").default = Some("1) , (2".into())), &ctx());
        assert_eq!(e[0].path, "columns[4].default");
    }

    #[test]
    fn type_aliases_are_not_changes() {
        // Yüklemedeki `format_type` çıktısı ile istemcinin takma adı eşit sayılır: boş plan.
        let old = TableDef {
            columns: vec![
                lc(1, "a", "character varying(20)"),
                lc(2, "b", "integer"),
                lc(3, "c", "timestamp(3) with time zone"),
                lc(4, "d", "numeric(10,2)"),
                lc(5, "e", "integer[]"),
                lc(6, "f", "character(1)"),
            ],
            ..base()
        };
        let draft = with(&old, |d| {
            d.columns[0].data_type = "varchar(20)".into();
            d.columns[1].data_type = "INT4".into();
            d.columns[2].data_type = "timestamptz(3)".into();
            d.columns[3].data_type = "decimal(10, 2)".into();
            d.columns[4].data_type = "int[][]".into();
            d.columns[5].data_type = "char".into();
        });
        assert!(ok(&old, &draft, &ctx()).statements.is_empty());
        // Bölüm/kalıtımlı tabloda da kolon farkı sayılmaz (MANAGED hatası yok).
        for flag in [0, 1] {
            let set = |d: &mut TableDef| {
                if flag == 0 {
                    d.is_partition = true
                } else {
                    d.inherits = true
                }
            };
            let p = ok(&with(&old, set), &with(&draft, set), &ctx());
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}", render(&p));
        }
        // Gerçek değişimde SQL kullanıcının metnini taşır; değiştirici anlamlıdır.
        let p = ok(&old, &with(&old, |d| d.columns[0].data_type = "varchar(30)".into()), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ALTER COLUMN a TYPE varchar(30)"]);
        let p = ok(&old, &with(&old, |d| d.columns[2].data_type = "timestamptz".into()), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ALTER COLUMN c TYPE timestamptz"]);
    }

    #[test]
    fn serial_types_are_rejected() {
        let l = loaded();
        for t in ["serial", "bigserial", "pg_catalog.serial4"] {
            let e = errs(&l, &with(&l, |d| colm(d, "note").data_type = t.into()), &ctx());
            assert_eq!(e[0].path, "columns[4].dataType", "{t}");
            let e = errs(&l, &with(&l, |d| d.columns.push(col("x", t))), &ctx());
            assert_eq!(e[0].path, "columns[7].dataType", "{t}");
        }
    }

    #[test]
    fn type_change_drops_and_restores_default() {
        let l = loaded();
        // Eski varsayılan + tip değişimi + aynı yeni varsayılan: DROP DEFAULT (TYPE'tan önce, eski adla) → TYPE →
        // yeniden adlandırma → SET DEFAULT (yeni adla).
        let p = ok(
            &l,
            &with(&l, |d| {
                let c = colm(d, "qty");
                c.data_type = "bigint".into();
                c.name = "quantity".into();
            }),
            &ctx(),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT",
                "ALTER TABLE public.t ALTER COLUMN qty TYPE bigint",
                "ALTER TABLE public.t RENAME COLUMN qty TO quantity",
                "ALTER TABLE public.t ALTER COLUMN quantity SET DEFAULT (0\n)",
            ]
        );
        assert_eq!(p.statements[0].risk, Risk::Safe);
        assert_eq!(p.statements[0].note.as_deref(), Some(TYPE_DROP_DEFAULT_NOTE));
        // Eski varsayılan + tip değişimi + farklı yeni varsayılan.
        let p = ok(
            &l,
            &with(&l, |d| {
                let c = colm(d, "qty");
                c.data_type = "text".into();
                c.default = Some("'yok'".into());
            }),
            &ctx(),
        );
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT",
                "ALTER TABLE public.t ALTER COLUMN qty TYPE text",
                "ALTER TABLE public.t ALTER COLUMN qty SET DEFAULT ('yok'\n)",
            ]
        );
        // Eski varsayılan + tip değişimi + yeni varsayılan yok: tek DROP DEFAULT (adım 8'de tekrar yok).
        let p = ok(
            &l,
            &with(&l, |d| {
                let c = colm(d, "qty");
                c.data_type = "bigint".into();
                c.default = None;
            }),
            &ctx(),
        );
        assert_eq!(
            sqls(&p),
            ["ALTER TABLE public.t ALTER COLUMN qty DROP DEFAULT", "ALTER TABLE public.t ALTER COLUMN qty TYPE bigint"]
        );
        // Varsayılansız kolonda tip değişimi: DROP DEFAULT yok; yalnız collation değişimi de TYPE sayılır.
        let p = ok(&l, &with(&l, |d| colm(d, "note").data_type = "integer".into()), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ALTER COLUMN note TYPE integer"]);
        // Takma adla "tip değişimi" değişim değildir: varsayılan da dokunulmaz.
        assert!(ok(&l, &with(&l, |d| colm(d, "qty").data_type = "int4".into()), &ctx()).statements.is_empty());
    }

    #[test]
    fn add_column_rewrite_warnings() {
        let l = loaded();
        let add = |c: ColumnDef| with(&l, |d| d.columns.push(c));
        let both = [REWRITE_WARNING, LOCK_TIMEOUT_WARNING];
        let cases: Vec<(TableDef, u32, bool)> = vec![
            (add(ColumnDef { not_null: true, identity: Some(IdentityKind::Always), ..col("x", "bigint") }), 120000, true),
            (
                add(ColumnDef { generated: Some(GeneratedDef { expr: "qty + 1".into(), stored: true }), ..col("x", "integer") }),
                120000,
                true,
            ),
            (
                add(ColumnDef { generated: Some(GeneratedDef { expr: "qty + 1".into(), stored: false }), ..col("x", "integer") }),
                180000,
                false,
            ),
            (add(ColumnDef { default: Some("1".into()), ..col("x", "integer") }), 120000, false),
            (add(col("x", "integer")), 120000, false),
        ];
        for (new, ver, rewrite) in cases {
            let p = ok(&l, &new, &v(ver));
            if rewrite {
                assert_eq!(p.warnings, both, "{}", render(&p));
            } else {
                assert!(p.warnings.is_empty(), "{}", render(&p));
            }
        }
        // PG 10: varsayılanlı ekleme tabloyu yeniden yazar.
        let plain = TableDef { columns: vec![lc(1, "a", "integer")], ..base() };
        let p = ok(&plain, &with(&plain, |d| d.columns.push(ColumnDef { default: Some("1".into()), ..col("x", "integer") })), &v(100000));
        assert_eq!(p.warnings, both);
    }

    #[test]
    fn long_rename_chain_is_linear() {
        // 1.600 kolonluk zincir (c0→c1, c1→c2, …): her adım bir sonrakini serbest bırakır.
        const N: i16 = 1600;
        let old = TableDef { columns: (0..N).map(|i| lc(i + 1, &format!("c{i}"), "integer")).collect(), ..base() };
        let new = with(&old, |d| {
            for (i, c) in d.columns.iter_mut().enumerate() {
                c.name = format!("c{}", i + 1);
            }
        });
        let started = std::time::Instant::now();
        let p = ok(&old, &new, &ctx());
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(p.statements.len(), N as usize);
        assert_eq!(p.statements[0].sql, "ALTER TABLE public.t RENAME COLUMN c1599 TO c1600");
        assert_eq!(p.statements[N as usize - 1].sql, "ALTER TABLE public.t RENAME COLUMN c0 TO c1");
    }

    #[test]
    fn zero_column_tables() {
        let empty = TableDef { columns: vec![], ..base() };
        // Kolonsuz tablo düzenlenebilir: boş değişiklik boş plan, ad değişimi çalışır.
        assert!(ok(&empty, &empty, &ctx()).statements.is_empty());
        let p = ok(&empty, &with(&empty, |d| d.name = "t2".into()), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t RENAME TO t2"]);
        // Son kolonu silmek geçerli (DataLoss).
        let one = base();
        let p = ok(&one, &empty, &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t DROP COLUMN id"]);
        assert_eq!(p.risk(), Risk::DataLoss);
        // Kolonsuz tabloya kolon eklemek.
        let p = ok(&empty, &with(&empty, |d| d.columns.push(col("a", "text"))), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD COLUMN a text"]);
        // Create hâlâ en az bir kolon ister.
        let e = crate::build::create(&o(TableDef { columns: vec![], ..base() }), &ctx()).unwrap_err();
        assert_eq!(e[0].path, "columns");
    }

    /// Yüklenmiş bölümlenmiş tablo (partition_by yok, partition_text var).
    fn loaded_parted() -> TableDef {
        TableDef {
            columns: vec![ColumnDef { not_null: true, ..lc(1, "id", "integer") }, lc(2, "r", "integer")],
            partition_text: Some("LIST (id)".into()),
            ..base()
        }
    }

    /// Gerçek sunucu gidiş-dönüşlerinin (Task 5) ortaya çıkardığı alter kuralları (bölümlenmiş tablo).
    #[test]
    fn partitioned_parent_rules_found_on_real_servers() {
        let p = loaded_parted();
        // Kalıcılık: PG 10–17 sessizce yoksayar, PG 18 reddeder → her sürümde hata.
        for ver in [100000, 170000, 180000] {
            let e = errs(&p, &with(&p, |d| d.persistence = Persistence::Unlogged), &v(ver));
            assert_eq!((e.len(), e[0].path.as_str()), (1, "persistence"), "{ver}: {e:?}");
        }
        let up = TableDef { persistence: Persistence::Unlogged, ..loaded_parted() };
        assert_eq!(errs(&up, &with(&up, |d| d.persistence = Persistence::Permanent), &ctx())[0].path, "persistence");
        // Sıradan tabloda SET UNLOGGED serbest.
        assert_eq!(sqls(&ok(&base(), &with(&base(), |d| d.persistence = Persistence::Unlogged), &ctx())), ["ALTER TABLE public.t SET UNLOGGED"]);
        // SET TABLESPACE: PG 10–11 yoksayar (hata), PG 12+ geçerli.
        let e = errs(&p, &with(&p, |d| d.tablespace = Some("fast".into())), &v(110000));
        assert_eq!((e.len(), e[0].path.as_str()), (1, "tablespace"), "{e:?}");
        assert_eq!(sqls(&ok(&p, &with(&p, |d| d.tablespace = Some("fast".into())), &v(120000))), ["ALTER TABLE public.t SET TABLESPACE fast"]);
        // fillfactor: bölümlenmiş tabloda depolama parametresi yok.
        let e = errs(&p, &with(&p, |d| d.fillfactor = Some(70)), &ctx());
        assert_eq!((e.len(), e[0].path.as_str()), (1, "fillfactor"), "{e:?}");
        // NOT VALID FK: PG 11–17 sunucu reddeder → hata; PG 18 geçerli. Geçerli (NOT VALID'siz) FK her sürümde serbest.
        let fk = |not_valid| ConstraintDef {
            oid: None,
            name: Some("p_fk".into()),
            kind: ConstraintKind::ForeignKey {
                columns: vec!["r".into()],
                ref_schema: "public".into(),
                ref_table: "u".into(),
                ref_columns: vec!["id".into()],
                match_type: FkMatch::Simple,
                on_update: FkAction::NoAction,
                on_delete: FkAction::NoAction,
            },
            deferrable: false,
            initially_deferred: false,
            not_valid,
        };
        let nv = with(&p, |d| d.constraints.push(fk(true)));
        for ver in [110000, 170000] {
            let e = errs(&p, &nv, &v(ver));
            assert_eq!((e.len(), e[0].path.as_str()), (1, "constraints[0].notValid"), "{ver}: {e:?}");
            assert!(e[0].message.contains("PG 18+"));
            ok(&p, &with(&p, |d| d.constraints.push(fk(false))), &v(ver));
        }
        assert!(sqls(&ok(&p, &nv, &v(180000)))[0].ends_with(" NOT VALID"));
        // Sıradan tabloya NOT VALID FK her sürümde serbest.
        let plain = TableDef { partition_text: None, ..loaded_parted() };
        let p2 = ok(&plain, &with(&plain, |d| d.constraints.push(fk(true))), &v(110000));
        assert!(sqls(&p2)[0].ends_with(" NOT VALID"));
        // Ad, yorum, sahip bölümlenmiş tabloda serbest.
        ok(&p, &with(&p, |d| {
            d.name = "p2".into();
            d.comment = Some("x".into());
            d.owner = Some("app".into());
        }), &v(100000));
    }

    /// PG 10–16: bölümü olan bölümlenmiş tabloya kimlikli kolon eklenemez (gerçek sunucu, Task 5); PG 17+ serbest.
    #[test]
    fn add_identity_column_to_partitioned_parent() {
        let p = loaded_parted();
        let add = |k| with(&p, |d| d.columns.push(ColumnDef { identity: Some(k), not_null: true, ..col("nid", "bigint") }));
        for ver in [100000, 120000, 160000] {
            for k in [IdentityKind::Always, IdentityKind::ByDefault] {
                let e = errs(&p, &add(k), &v(ver));
                assert_eq!((e.len(), e[0].path.as_str()), (1, "columns[2].identity"), "{ver}: {e:?}");
                assert!(e[0].message.contains("PG <17"));
            }
        }
        for ver in [170000, 180000] {
            assert!(sqls(&ok(&p, &add(IdentityKind::Always), &v(ver)))[0].contains("GENERATED ALWAYS AS IDENTITY"));
        }
        // Mevcut kolona kimlik eklemek (ADD GENERATED) her sürümde kabul edilir.
        let existing = with(&p, |d| colm(d, "r").not_null = true);
        let to_ident = with(&existing, |d| colm(d, "r").identity = Some(IdentityKind::Always));
        let plan = ok(&existing, &to_ident, &v(100000));
        assert!(sqls(&plan)[0].ends_with("ADD GENERATED ALWAYS AS IDENTITY"), "{:?}", sqls(&plan));
        // Sıradan tablo (çocukları bilinmez): sunucuya bırakılır.
        let plain = TableDef { partition_text: None, ..loaded_parted() };
        ok(&plain, &with(&plain, |d| d.columns.push(ColumnDef { identity: Some(IdentityKind::Always), not_null: true, ..col("nid", "bigint") })), &v(100000));
    }

    /// PG 18: CHECK'li tabloda sanal kolonun SET EXPRESSION'ı sunucuda desteklenmez (gerçek sunucu, Task 5).
    #[test]
    fn virtual_set_expression_with_check_constraints() {
        let virt = TableDef {
            columns: vec![
                lc(1, "a", "integer"),
                ColumnDef { generated: Some(GeneratedDef { expr: "a * 2".into(), stored: false }), ..lc(2, "b", "integer") },
            ],
            ..base()
        };
        let check = ConstraintDef {
            oid: Some(7),
            name: Some("a_ck".into()),
            kind: ConstraintKind::Check { expr: "a > 0".into(), no_inherit: false },
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
        };
        let set_expr = |d: &mut TableDef| colm(d, "b").generated.as_mut().unwrap().expr = "a * 3".into();
        // CHECK yok: geçerli.
        assert_eq!(sqls(&ok(&virt, &with(&virt, set_expr), &ctx())), ["ALTER TABLE public.t ALTER COLUMN b SET EXPRESSION AS (a * 3
)"]);
        // Kalan CHECK (ya da desteklenmeyen CHECK) varken hata.
        let with_ck = with(&virt, |d| d.constraints.push(check.clone()));
        let e = errs(&with_ck, &with(&with_ck, set_expr), &ctx());
        assert_eq!((e.len(), e[0].path.as_str()), (1, "columns[1].generated"), "{e:?}");
        // Etiket yükleyicinin sabitinden: sabit değişse de kural çalışır (tek kaynak).
        let unsup = TableDef { unsupported: vec![format!("{} c (NOT ENFORCED)", crate::load::CHECK_LABEL)], ..virt.clone() };
        assert_eq!(errs(&unsup, &with(&unsup, set_expr), &ctx())[0].path, "columns[1].generated");
        // Aynı düzenlemede CHECK silinirse (adım 3) geçerli; yeni CHECK eklemek (adım 10) engel değil.
        let p = ok(&with_ck, &with(&virt, set_expr), &ctx());
        assert_eq!(sqls(&p)[0], "ALTER TABLE public.t DROP CONSTRAINT a_ck");
        let mut add = with(&virt, set_expr);
        add.constraints.push(ConstraintDef { oid: None, ..check.clone() });
        ok(&virt, &add, &ctx());
        // STORED kolonda CHECK engel değil.
        let stored = with(&with_ck, |d| colm(d, "b").generated.as_mut().unwrap().stored = true);
        ok(&stored, &with(&stored, set_expr), &ctx());
    }

    /// PG 12+: üretilmiş kolonun kullandığı kolonun tipi değiştirilemez (gerçek sunucu, Task 5): plan uyarısı.
    #[test]
    fn type_change_of_column_used_by_generated_column_warns() {
        let l = loaded();
        // `total` = qty * 2.
        let p = ok(&l, &with(&l, |d| colm(d, "qty").data_type = "bigint".into()), &ctx());
        assert!(p.warnings.iter().any(|w| w.contains("'qty' kolonu üretilmiş 'total'")), "{:?}", p.warnings);
        // Kullanılmayan kolon: uyarı yok.
        let p = ok(&l, &with(&l, |d| colm(d, "amount").data_type = "bigint".into()), &ctx());
        assert!(!p.warnings.iter().any(|w| w.contains("üretilmiş")), "{:?}", p.warnings);
        // Üretilmiş kolonun kendi tipi: uyarı yok.
        let p = ok(&l, &with(&l, |d| colm(d, "total").data_type = "numeric(12,2)".into()), &ctx());
        assert!(!p.warnings.iter().any(|w| w.contains("ifadesinde geçiyor")), "{:?}", p.warnings);
        assert!(expr_mentions("qty * 2", "qty"));
        assert!(expr_mentions("\"Qty\" * 2", "Qty"));
        assert!(!expr_mentions("\"Qty\" * 2", "qty"));
        assert!(expr_mentions("QTY + 1", "qty"));
        assert!(!expr_mentions("'qty' || x", "qty"));
        assert!(!expr_mentions("qty2 + 1", "qty"));
        assert!(!expr_mentions("'it''s qty' || x", "qty"));
    }

    /// Yükleyicinin döndürüp create dilbilgisinin reddettiği alanlar: PostGIS tip metni, 128 karakter üstü tip, biçim
    /// karakterli ad, `$`-bitişik tanımlayıcı içeren varsayılan/CHECK.
    fn weird() -> TableDef {
        TableDef {
            columns: vec![
                ColumnDef { not_null: true, ..lc(1, "id", "integer") },
                lc(2, "geom", "public.geometry(Point,4326)"),
                lc(3, "wide", &format!("\"{}\".\"{}\"", "a".repeat(60), "b".repeat(60))),
                ColumnDef { default: Some("x1$".into()), ..lc(4, "dflt", "integer") },
                lc(5, "a\u{200c}b", "text"),
            ],
            constraints: vec![ConstraintDef {
                oid: Some(300),
                name: Some("ck\u{202e}".into()),
                kind: ConstraintKind::Check { expr: "x1$ > 0".into(), no_inherit: false },
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
            }],
            comment: Some("eski".into()),
            ..base()
        }
    }

    #[test]
    fn untouched_loaded_fields_are_not_revalidated() {
        let w = weird();
        // Salt tablo düzeyi değişiklikler: yeniden adlandırma, şema, sahip, yorum.
        for (label, new) in [
            ("yorum", with(&w, |d| d.comment = Some("yeni".into()))),
            ("ad", with(&w, |d| d.name = "t2".into())),
            ("şema", with(&w, |d| d.schema = "s2".into())),
            ("sahip", with(&w, |d| d.owner = Some("app".into()))),
        ] {
            let p = ok(&w, &new, &ctx());
            assert_eq!(p.statements.len(), 1, "{label}: {}", render(&p));
            for s in &p.statements {
                assert!(!s.sql.contains("geometry"), "{label}: {}", s.sql);
            }
        }
        // Dokunulmamış tanım: boş plan.
        assert!(ok(&w, &w, &ctx()).statements.is_empty());
        // Kolon yorumu/yeni kolon eklemek de weird alanlara dokunmaz.
        let p = ok(&w, &with(&w, |d| d.columns.push(col("extra", "text"))), &ctx());
        assert_eq!(sqls(&p), ["ALTER TABLE public.t ADD COLUMN extra text"]);
        // Biçim karakterli kolon yeniden adlandırılabilir (yeni ad doğrulanır), CHECK'e dokunulmaz.
        let p = ok(&w, &with(&w, |d| d.columns[4].name = "plain".into()), &ctx());
        assert_eq!(sqls(&p).len(), 1, "{}", render(&p));
    }

    #[test]
    fn changed_or_written_loaded_fields_are_validated() {
        let w = weird();
        // Değişen alan create kurallarıyla doğrulanır.
        let e = errs(&w, &with(&w, |d| d.columns[1].data_type = "public.geometry(Point,3857)".into()), &ctx());
        assert_eq!(e[0].path, "columns[1].dataType");
        let e = errs(&w, &with(&w, |d| d.columns[3].default = Some("x1$ + 1".into())), &ctx());
        assert_eq!(e[0].path, "columns[3].default");
        let e = errs(&w, &with(&w, |d| d.columns[4].name = "a\nb".into()), &ctx());
        assert_eq!(e[0].path, "columns[4].name");
        let e = errs(&w, &with(&w, |d| d.name = "t\n".into()), &ctx());
        assert_eq!(e[0].path, "name");
        // Yinelenen ad (değişmeyen kolonun adıyla çakışma) yine yakalanır.
        let e = errs(&w, &with(&w, |d| d.columns[3].name = "id".into()), &ctx());
        assert!(e.iter().any(|x| x.message.contains("birden fazla")), "{e:?}");
        // Yalnızca collation değişimi TYPE'ı aynı (yüklü) tip metniyle yeniden yazar: yazım anında doğrulanır, hata verir.
        let e = errs(&w, &with(&w, |d| d.columns[1].collation = Some("C".into())), &ctx());
        assert_eq!(e[0].path, "columns[1].dataType");
        // Tip değişiminde değişmemiş (yüklü) varsayılan yeniden yazılır: o da yazım anında doğrulanır.
        let e = errs(&w, &with(&w, |d| d.columns[3].data_type = "bigint".into()), &ctx());
        assert_eq!(e[0].path, "columns[3].default");
    }

    #[test]
    fn dropping_a_column_warns_about_checks_that_pg_drops_silently() {
        let l = with(&loaded(), |d| {
            d.constraints.push(ConstraintDef {
                oid: Some(110),
                name: Some("ck_amount".into()),
                kind: ConstraintKind::Check { expr: "amount > 0 OR note IS NOT NULL".into(), no_inherit: false },
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
            })
        });
        let p = ok(&l, &with(&l, |d| d.columns.retain(|c| c.name != "amount")), &ctx());
        assert!(
            p.warnings.iter().any(|w| w.contains("'amount'") && w.contains("ck_amount") && w.contains("CHECK")),
            "{:?}",
            p.warnings
        );
        // Başvurmayan kolon silinince uyarı yok.
        let p = ok(&l, &with(&l, |d| d.columns.retain(|c| c.name != "seq")), &ctx());
        assert!(!p.warnings.iter().any(|w| w.contains("CHECK")), "{:?}", p.warnings);
        // Desteklenmeyen kısıt (EXCLUDE) varken kolon silme: genel uyarı.
        let ex = with(&loaded(), |d| d.unsupported = vec!["EXCLUDE ex".into()]);
        let p = ok(&ex, &with(&ex, |d| d.columns.retain(|c| c.name != "note")), &ctx());
        assert!(p.warnings.iter().any(|w| w.contains("desteklenmeyen")), "{:?}", p.warnings);
        let p = ok(&ex, &with(&ex, |d| d.comment = None), &ctx());
        assert!(!p.warnings.iter().any(|w| w.contains("desteklenmeyen")));
    }

    #[test]
    fn cross_field_errors_on_untouched_loaded_fields_are_kept() {
        // Kimlik eklenen kolon varsayılan değerli kalamaz (varsayılan metni değişmemiş olsa da).
        let mut l = loaded();
        colm(&mut l, "amount").default = Some("nextval('s'::regclass)".into());
        let e = errs(&l, &with(&l, |d| colm(d, "amount").identity = Some(IdentityKind::Always)), &ctx());
        assert!(e.iter().any(|x| x.path == "columns[5].default" && x.message.contains("Kimlik kolonu")), "{e:?}");
        // Üretilmiş kolona kimlik eklenemez.
        let e = errs(&l, &with(&l, |d| colm(d, "total").identity = Some(IdentityKind::Always)), &ctx());
        assert!(e.iter().any(|x| x.path == "columns[3].generated" && x.message.contains("hem kimlik")), "{e:?}");
    }

    #[test]
    fn using_is_always_validated_even_when_equal_on_both_sides() {
        let old = with(&loaded(), |d| colm(d, "amount").using = Some("x'".into()));
        let new = with(&old, |d| colm(d, "amount").data_type = "bigint".into());
        let e = errs(&old, &new, &ctx());
        assert!(e.iter().any(|x| x.path == "columns[5].using"), "{e:?}");
    }
}
