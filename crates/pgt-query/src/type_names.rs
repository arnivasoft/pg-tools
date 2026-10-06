//! Sonuç sütunlarının kullanıcıya gösterilen tür adları (`bigint`, `double precision`, `character varying(50)` …).
//!
//! tokio-postgres yalnızca iç kısa adı (`int8`, `float8`, `varchar`) ve tür değiştiricisini (`typmod`) verir. Gösterim
//! adı PostgreSQL'in kendi `pg_catalog.format_type(oid, typmod)` işleviyle üretilir (dizi, alan (domain) ve kullanıcı
//! türleri sunucunun yazdığı gibi). Çalıştırıcı başına bir önbellek tutulur: bir ifade için yalnızca önbellekte
//! olmayan `(oid, typmod)` çiftleri tek bir katalog sorgusuyla (`FORMAT_TYPES_SQL`) sorulur; sorgu başarısız olursa
//! kısa ada düşülür (kullanıcının ifadesi bu yüzden asla başarısız olmaz).

use std::collections::HashMap;

/// Sırayı koruyarak `(oid, typmod)` çiftlerinin gösterim adları. Salt okunur; şema nitelemeli adlar arama yolundan
/// etkilenmez (`pg_catalog.` ön ekleri). Çok argümanlı `unnest` yalnızca niteliksiz yazılabildiği için `ROWS FROM`
/// (PG 9.4+).
pub(crate) const FORMAT_TYPES_SQL: &str = "SELECT pg_catalog.format_type(t.o, t.m) \
     FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.oid[]), pg_catalog.unnest($2::pg_catalog.int4[])) \
     WITH ORDINALITY AS t(o, m, i) ORDER BY t.i";

/// Önbellekteki en çok giriş; aşılırsa önbellek boşaltılır (bellek sınırlı kalsın).
pub(crate) const CACHE_MAX: usize = 4096;

/// PostgreSQL'in ilk kullanıcı nesnesi oid'i (`FirstNormalObjectId`); altındakiler yerleşik türlerdir.
const FIRST_NORMAL_OID: u32 = 16384;

pub(crate) type TypeKey = (u32, i32);

#[derive(Default)]
pub(crate) struct TypeNameCache {
    names: HashMap<TypeKey, String>,
}

impl TypeNameCache {
    /// Önbellekte olmayan çiftler, ilk görülme sırasıyla ve tekrarsız.
    pub(crate) fn missing(&self, keys: &[TypeKey]) -> Vec<TypeKey> {
        let mut out: Vec<TypeKey> = Vec::new();
        for k in keys {
            if !self.names.contains_key(k) && !out.contains(k) {
                out.push(*k);
            }
        }
        out
    }

    /// `asked` için sunucunun yanıtı (`asked` ile aynı sırada). Satır sayısı uymazsa ya da bir ad `NULL` ise o
    /// çift önbelleğe yazılmaz (bir sonraki ifadede yeniden sorulur).
    pub(crate) fn store(&mut self, asked: &[TypeKey], names: Vec<Option<String>>) {
        if names.len() != asked.len() {
            return;
        }
        if self.names.len() + asked.len() > CACHE_MAX {
            self.names.clear();
        }
        for (k, name) in asked.iter().zip(names) {
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                self.names.insert(*k, name);
            }
        }
    }

    /// Gösterim adı; bilinmiyorsa `fallback` (iç kısa ad).
    pub(crate) fn display(&self, key: TypeKey, fallback: &str) -> String {
        self.names.get(&key).cloned().unwrap_or_else(|| fallback.to_string())
    }

    /// Kullanıcı türlerinin adları DDL ya da `search_path` değişikliğiyle değişebilir (yeniden adlandırma, şema
    /// nitelemesi): yerleşik olmayan türlerin girdileri atılır. Yerleşik türlerin adı sabittir.
    pub(crate) fn forget_user_types(&mut self) {
        self.names.retain(|(oid, _), _| *oid < FIRST_NORMAL_OID);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.names.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn missing_is_deduplicated_and_skips_cached() {
        let mut c = TypeNameCache::default();
        assert_eq!(c.missing(&[(20, -1), (1043, 54), (20, -1)]), vec![(20, -1), (1043, 54)]);
        c.store(&[(20, -1)], vec![some("bigint")]);
        assert_eq!(c.missing(&[(20, -1), (1043, 54), (1043, 14)]), vec![(1043, 54), (1043, 14)]);
        assert!(c.missing(&[(20, -1)]).is_empty(), "önbellekteki çift yeniden sorulmaz");
    }

    #[test]
    fn display_uses_cache_then_falls_back_to_short_name() {
        let mut c = TypeNameCache::default();
        c.store(&[(20, -1), (1043, 54)], vec![some("bigint"), some("character varying(50)")]);
        assert_eq!(c.display((20, -1), "int8"), "bigint");
        assert_eq!(c.display((1043, 54), "varchar"), "character varying(50)");
        // Aynı oid, farklı typmod ayrı girdidir.
        assert_eq!(c.display((1043, 14), "varchar"), "varchar");
        assert_eq!(c.display((701, -1), "float8"), "float8");
    }

    #[test]
    fn mismatched_or_null_answers_are_not_cached() {
        let mut c = TypeNameCache::default();
        c.store(&[(20, -1), (701, -1)], vec![some("bigint")]);
        assert_eq!(c.len(), 0, "satır sayısı uymayan yanıt yok sayılır");
        c.store(&[(20, -1), (701, -1)], vec![None, some("double precision")]);
        assert_eq!(c.display((20, -1), "int8"), "int8");
        assert_eq!(c.display((701, -1), "float8"), "double precision");
        assert_eq!(c.missing(&[(20, -1), (701, -1)]), vec![(20, -1)]);
    }

    #[test]
    fn user_types_are_forgotten_builtins_kept() {
        let mut c = TypeNameCache::default();
        c.store(&[(20, -1), (16500, -1)], vec![some("bigint"), some("app.money_amount")]);
        c.forget_user_types();
        assert_eq!(c.display((20, -1), "int8"), "bigint");
        assert_eq!(c.display((16500, -1), "money_amount"), "money_amount");
    }

    #[test]
    fn cache_is_bounded() {
        let mut c = TypeNameCache::default();
        let keys: Vec<TypeKey> = (0..CACHE_MAX as i32).map(|m| (1043, m)).collect();
        c.store(&keys, keys.iter().map(|_| some("x")).collect());
        assert_eq!(c.len(), CACHE_MAX);
        c.store(&[(20, -1)], vec![some("bigint")]);
        assert_eq!(c.len(), 1, "sınır aşılınca önbellek boşaltılır");
        assert_eq!(c.display((20, -1), "int8"), "bigint");
    }
}
