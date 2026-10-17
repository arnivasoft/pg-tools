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
    pub fn lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

/// Geçersiz UTF-8 içeren metin için ortak ileti (ad/değer içermez).
pub fn invalid_utf8_message() -> String {
    "katalogda geçerli UTF-8 olmayan bir ad var (PostgreSQL uzun çok baytlı adı kesmiş olabilir); DDL üretilmez".into()
}

/// Gösterim için: metin sütunu, geçersiz baytlar U+FFFD. Sütun yoksa ya da metin değilse panikler (`Row::get` gibi).
pub fn lossy<I: RowIndex + std::fmt::Display>(row: &Row, idx: I) -> String {
    row.get::<_, PgText>(idx).lossy()
}

/// [`lossy`]; `NULL` → `None`.
pub fn lossy_opt<I: RowIndex + std::fmt::Display>(row: &Row, idx: I) -> Option<String> {
    row.get::<_, Option<PgText>>(idx).map(|t| t.lossy())
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

/// Satırın tüm metin sütunları geçerli UTF-8 mi (metin olmayan sütunlara bakılmaz).
pub fn is_utf8(row: &Row) -> bool {
    row.columns().iter().enumerate().all(|(i, c)| {
        !PgText::accepts(c.type_())
            || row.try_get::<_, Option<PgText>>(i).ok().flatten().is_none_or(|t| std::str::from_utf8(&t.0).is_ok())
    })
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
        assert_eq!(t.lossy(), "aş\u{FFFD}_multirange");
        assert_eq!(PgText::from_sql(&Type::TEXT, "ş".as_bytes()).unwrap().lossy(), "ş");
    }

    #[test]
    fn invalid_utf8_message_is_named_and_carries_no_name() {
        let m = invalid_utf8_message();
        assert!(m.contains("UTF-8") && m.contains("DDL"), "{m}");
        assert!(matches!(Error::Invalid(m), Error::Invalid(_)));
    }
}
