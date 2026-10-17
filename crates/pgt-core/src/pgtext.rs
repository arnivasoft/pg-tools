//! Katalogdan okunan metinlerin UTF-8 sınırı.
//!
//! PostgreSQL katalogda geçersiz UTF-8 ad üretebilir: PG 14+ `makeMultirangeTypeName` uzun range adını ham 52 bayta
//! keser (çok baytlı karakterin ortasından), dizi tipi adı da ondan türer; `format_type`, `regtype::text`,
//! `pg_get_function_identity_arguments` … bu adı olduğu gibi döndürür. tokio-postgres `String` çözümünde bunu hata
//! sayar (`Row::get` panikler, `try_get` hata döner).
//!
//! **Tek yaklaşım (SQL değişmez, satır sınırında):** `text`/`name`/`varchar` sütununun ikili biçimi ham baytlardır;
//! `PgText` onları doğrulamadan alır.
//! - Gösterim (Explorer etiketi/ayrıntısı, özellikler, tamamlama, sonuç sütun tipi): [`lossy`] / [`lossy_opt`] —
//!   `String::from_utf8_lossy` (geçersiz bayt → U+FFFD).
//! - DDL ya da SQL'e geri yazılabilecek kimlik: kayıplı ad **asla** kullanılmaz. Betik üretimi [`ensure_utf8`] ile
//!   reddedilir; düzenleyici yükleyicileri [`is_utf8`] ile nesneyi salt okunur (`unsupported`) yükler.
//!
//! - Sunucu tarafında kodlamayı doğrulayan işlevler (`lower`, `ILIKE`, `convert_from` …) böyle bir adda sorgunun
//!   tamamını 22021 ile düşürür: o sorgular adı önce [`VALID_UTF8_ESCAPED_RE`] ile süzer.
//!
//! (`pgt-ddl` `load_range` aynı nedenle multirange adını `textsend` ile bayt olarak okur; PG'nin otomatik adıyla
//! bayt bayt karşılaştırması gerekir.)

use tokio_postgres::Row;
use tokio_postgres::row::RowIndex;
use tokio_postgres::types::{FromSql, Type};

use crate::{Error, Result};

/// SQL'de geçerli UTF-8 sınaması (PG 10+, hata vermeden): `encode(textsend(x), 'escape') ~ $n` (parametre olarak
/// geçilir). `escape` biçimi yüksek bitli baytları `\ooo`, ters bölüyü `\\` yazar (salt ASCII); düzenli ifade her yüksek
/// baytın doğru sayıda devam baytıyla (`\2oo` = 0x80–0xBF) geldiğini denetler (öncü 0xC0–0xDF/0xE0–0xEF/0xF0–0xF7).
pub const VALID_UTF8_ESCAPED_RE: &str = r"^(?:[^\\]|\\\\|\\3[0-3][0-7]\\2[0-7]{2}|\\3[45][0-7](?:\\2[0-7]{2}){2}|\\36[0-7](?:\\2[0-7]{2}){3})*$";

/// Metin sütununun doğrulanmamış ham baytları.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgText(pub Vec<u8>);

impl<'a> FromSql<'a> for PgText {
    fn from_sql(_: &Type, raw: &'a [u8]) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(PgText(raw.to_vec()))
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

impl PgText {
    /// Kayıplı çözüm; geçerli UTF-8'de bayt arabelleği yeniden kullanılır (ek ayırma yok).
    pub fn into_lossy(self) -> String {
        String::from_utf8(self.0).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }
}

/// Metin ya da metin dizisi (`text[]`, `name[]` …) sütununun geçerli UTF-8 olup olmadığı (`NULL` geçerli sayılır);
/// [`is_utf8`]'in sütun sınaması.
struct Utf8Ok(bool);

impl<'a> FromSql<'a> for Utf8Ok {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        if PgText::accepts(ty) {
            return Ok(Utf8Ok(std::str::from_utf8(raw).is_ok()));
        }
        let items = Vec::<Option<PgText>>::from_sql(ty, raw)?;
        Ok(Utf8Ok(items.iter().flatten().all(|t| std::str::from_utf8(&t.0).is_ok())))
    }

    fn from_sql_null(_: &Type) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Utf8Ok(true))
    }

    fn accepts(ty: &Type) -> bool {
        PgText::accepts(ty) || <Vec<Option<PgText>> as FromSql>::accepts(ty)
    }
}

/// Geçersiz UTF-8 içeren metin için ortak ileti (ad/değer içermez).
pub fn invalid_utf8_message() -> String {
    "katalogda geçerli UTF-8 olmayan bir ad var (PostgreSQL uzun çok baytlı adı kesmiş olabilir); DDL üretilmez".into()
}

/// Gösterim için: metin sütunu, geçersiz baytlar U+FFFD. Sütun yoksa ya da metin değilse panikler (`Row::get` gibi).
pub fn lossy<I: RowIndex + std::fmt::Display>(row: &Row, idx: I) -> String {
    row.get::<_, PgText>(idx).into_lossy()
}

/// [`lossy`]; `NULL` → `None`.
pub fn lossy_opt<I: RowIndex + std::fmt::Display>(row: &Row, idx: I) -> Option<String> {
    row.get::<_, Option<PgText>>(idx).map(PgText::into_lossy)
}

/// Metin dizisi (`text[]`) sütunu, öğeler kayıplı; `NULL` öğe boş metin. Sütun yoksa ya da dizi değilse panikler.
pub fn lossy_vec<I: RowIndex + std::fmt::Display>(row: &Row, idx: I) -> Vec<String> {
    row.get::<_, Vec<Option<PgText>>>(idx).into_iter().map(|t| t.map(PgText::into_lossy).unwrap_or_default()).collect()
}

/// tokio-postgres istemci tarafı çözüm hatası geçersiz UTF-8 metinden mi (ör. `prepare` sonuç tipini kendi tip
/// sorgusuyla çözerken PG'nin kestiği tip adına çarptı). Sunucu hatası değildir; kaynak zincirinde `Utf8Error` aranır.
pub fn is_invalid_utf8_error(e: &tokio_postgres::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        if s.is::<std::str::Utf8Error>() || s.is::<std::string::FromUtf8Error>() {
            return true;
        }
        source = s.source();
    }
    false
}

/// Satırın tüm metin ve metin dizisi sütunları geçerli UTF-8 mi (diğer sütunlara bakılmaz).
pub fn is_utf8(row: &Row) -> bool {
    row.columns()
        .iter()
        .enumerate()
        .all(|(i, c)| !Utf8Ok::accepts(c.type_()) || row.try_get::<_, Utf8Ok>(i).is_ok_and(|u| u.0))
}

/// DDL betiği üretimi için: tüm satırların metin sütunları geçerli UTF-8 değilse `Error::Invalid`.
pub fn ensure_utf8(rows: &[Row]) -> Result<()> {
    if rows.iter().all(is_utf8) { Ok(()) } else { Err(Error::Invalid(invalid_utf8_message())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_text_accepts_text_like_types_only() {
        for ty in [Type::TEXT, Type::NAME, Type::VARCHAR, Type::BPCHAR] {
            assert!(PgText::accepts(&ty), "{ty}");
        }
        for ty in [Type::INT4, Type::BYTEA, Type::TEXT_ARRAY, Type::OID] {
            assert!(!PgText::accepts(&ty), "{ty}");
        }
    }

    #[test]
    fn pg_text_keeps_raw_bytes_and_decodes_lossily() {
        let raw = [b"a\xC5\x9F".as_slice(), &[0xC5], b"_multirange"].concat();
        let t = PgText::from_sql(&Type::NAME, &raw).unwrap();
        assert_eq!(t.0, raw);
        assert_eq!(t.into_lossy(), "aş\u{FFFD}_multirange");
        assert_eq!(PgText::from_sql(&Type::TEXT, "ş".as_bytes()).unwrap().into_lossy(), "ş");
    }

    #[test]
    fn into_lossy_reuses_the_buffer_for_valid_utf8() {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice("şema".as_bytes());
        let ptr = bytes.as_ptr();
        let s = PgText(bytes).into_lossy();
        assert_eq!((s.as_str(), s.as_ptr()), ("şema", ptr), "geçerli yolda yeni ayırma yok");
    }

    /// PG ikili tek boyutlu dizi: ndim, has_null, öğe oid, uzunluk, alt sınır, öğeler (uzunluk + bayt; -1 = NULL).
    fn array(oid: u32, items: &[Option<&[u8]>]) -> Vec<u8> {
        let mut b = vec![];
        b.extend(1i32.to_be_bytes());
        b.extend(i32::from(items.iter().any(Option::is_none)).to_be_bytes());
        b.extend(oid.to_be_bytes());
        b.extend((items.len() as i32).to_be_bytes());
        b.extend(1i32.to_be_bytes());
        for it in items {
            match it {
                Some(v) => {
                    b.extend((v.len() as i32).to_be_bytes());
                    b.extend_from_slice(v);
                }
                None => b.extend((-1i32).to_be_bytes()),
            }
        }
        b
    }

    #[test]
    fn utf8_check_covers_text_and_text_arrays() {
        let bad: &[u8] = b"a\xC5_multirange";
        for ty in [Type::TEXT_ARRAY, Type::NAME_ARRAY, Type::VARCHAR_ARRAY, Type::TEXT, Type::NAME] {
            assert!(Utf8Ok::accepts(&ty), "{ty}");
        }
        assert!(!Utf8Ok::accepts(&Type::INT4) && !Utf8Ok::accepts(&Type::INT4_ARRAY));
        assert!(Utf8Ok::from_sql(&Type::TEXT, "ş".as_bytes()).unwrap().0);
        assert!(!Utf8Ok::from_sql(&Type::NAME, bad).unwrap().0);
        let ok = array(25, &[Some("ş".as_bytes()), None, Some(b"x")]);
        assert!(Utf8Ok::from_sql(&Type::TEXT_ARRAY, &ok).unwrap().0);
        let ko = array(19, &[Some(b"x"), Some(bad)]);
        assert!(!Utf8Ok::from_sql(&Type::NAME_ARRAY, &ko).unwrap().0);
        assert!(Utf8Ok::from_sql_null(&Type::TEXT_ARRAY).unwrap().0, "NULL geçerli");
        let v = Vec::<Option<PgText>>::from_sql(&Type::NAME_ARRAY, &ko).unwrap();
        assert_eq!(v.into_iter().map(|t| t.unwrap().into_lossy()).collect::<Vec<_>>(), ["x", "a\u{FFFD}_multirange"]);
    }

    #[test]
    fn invalid_utf8_message_is_named_and_carries_no_name() {
        let m = invalid_utf8_message();
        assert!(m.contains("UTF-8") && m.contains("DDL"), "{m}");
        assert!(matches!(Error::Invalid(m), Error::Invalid(_)));
    }
}
