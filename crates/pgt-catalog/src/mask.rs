//! Gizli değerlerin maskelenmesi (Plan 38 Karar 2): subscription bağlantı dizgesi ve `password` seçenekleri.
//!
//! Saf işlevlerdir; ham değer yalnız çağıranın belleğinde, maskelemeden hemen önce bulunur. Çözümlemede herhangi
//! bir belirsizlik (bozuk tırnak, `=` yok, tanınmayan anahtar, iç içe bağlantı dizgesi …) olursa **tüm dizge**
//! `MASK` olur: yanlışlıkla gösterilen bir parça parola, gizlenen fazladan bir ayrıntıdan pahalıdır.

/// Gizli değerin yerine yazılan metin.
pub const MASK: &str = "********";

/// Değeri gizli sayılan anahtar: `password`, `sslpassword` … (büyük/küçük harf duyarsız, `password` ile biten).
pub fn is_secret_key(key: &str) -> bool {
    key.trim().to_ascii_lowercase().ends_with("password")
}

/// `anahtar=değer` seçenek öğelerinde gizli anahtarların değerini maskeler (`pg_*.…options` dizileri).
/// PostgreSQL öğeyi ilk `=`'den böler; `=` yoksa öğe olduğu gibi kalır.
pub fn mask_options(options: &[String]) -> Vec<String> {
    options
        .iter()
        .map(|o| match o.split_once('=') {
            Some((k, _)) if is_secret_key(k) => format!("{k}={MASK}"),
            _ => o.clone(),
        })
        .collect()
}

/// libpq bağlantı dizgesindeki parolaları maskeler: `anahtar=değer` biçimi ve `postgres(ql)://` URI biçimi.
/// Diğer anahtarlar ve boşluklar aynen kalır; belirsizlikte tüm dizge `MASK` olur.
pub fn mask_conninfo(conninfo: &str) -> String {
    let rest = conninfo.trim_start();
    let lower = rest.get(..13).unwrap_or(rest).to_ascii_lowercase();
    let masked = if lower.starts_with("postgresql://") || lower.starts_with("postgres://") {
        mask_uri(conninfo, conninfo.len() - rest.len())
    } else {
        mask_keyword_value(conninfo)
    };
    masked.unwrap_or_else(|| MASK.to_string())
}

/// libpq `isspace` kümesi (dikey sekme dahil).
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
}

/// libpq anahtarları yalnız harf, rakam ve `_` içerir; başka bir şey belirsizliktir.
fn plain_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Gizli olmayan bir değer iç içe bağlantı dizgesi ya da parola içeriyor gibi görünüyorsa belirsizdir.
fn suspicious_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("password") || lower.contains("://")
}

/// `anahtar = değer` biçimi (libpq `conninfo_parse` kuralları: tırnaklı değerde `\'`/`\\` kaçışları).
fn mask_keyword_value(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    let mut i = 0;
    loop {
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let key_start = i;
        while i < b.len() && b[i] != b'=' && !is_space(b[i]) {
            i += 1;
        }
        let key = &s[key_start..i];
        if !plain_key(key) {
            return None;
        }
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            return None;
        }
        i += 1;
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        let mut value = Vec::new();
        if i < b.len() && b[i] == b'\'' {
            i += 1;
            loop {
                match b.get(i) {
                    None => return None, // kapanmamış tırnak
                    Some(b'\\') => {
                        value.push(*b.get(i + 1)?);
                        i += 2;
                    }
                    Some(b'\'') => {
                        i += 1;
                        break;
                    }
                    Some(&c) => {
                        value.push(c);
                        i += 1;
                    }
                }
            }
        } else {
            while i < b.len() && !is_space(b[i]) {
                if b[i] == b'\\' {
                    // Sonda tek ters bölü: libpq sessizce atar; belirsiz say.
                    value.push(*b.get(i + 1)?);
                    i += 2;
                } else {
                    value.push(b[i]);
                    i += 1;
                }
            }
        }
        if is_secret_key(key) {
            out.push_str(&s[copied..key_start]);
            out.push_str(key);
            out.push('=');
            out.push_str(MASK);
            copied = i;
        } else if suspicious_value(&String::from_utf8_lossy(&value)) {
            return None;
        }
    }
    out.push_str(&s[copied..]);
    Some(out)
}

/// `%XX` kodlamasını çözer; bozuk kodlama `None`.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// URI biçimi: `postgres(ql)://[kullanıcı[:parola]@][konak][/vt][?anahtar=değer&…]`. `start` önekin başladığı bayt.
fn mask_uri(s: &str, start: usize) -> Option<String> {
    let after = start + s[start..].find("://")? + 3;
    // libpq kullanıcı bilgisini ilk `/`'ye kadar arar; birden çok `@` ya da `/`'den sonra `@` belirsizdir.
    let authority_end = s[after..].find('/').map_or(s.len(), |p| after + p);
    let ats = s[after..].matches('@').count();
    let at = s[after..authority_end].find('@').map(|p| after + p);
    if ats > 1 || (ats == 1 && at.is_none()) {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let host_start = match at {
        Some(at) => {
            let userinfo = &s[after..at];
            match userinfo.find(':') {
                Some(colon) => {
                    out.push_str(&s[..after + colon + 1]);
                    out.push_str(MASK);
                    out.push('@');
                }
                None => out.push_str(&s[..=at]),
            }
            at + 1
        }
        None => {
            out.push_str(&s[..after]);
            after
        }
    };
    let Some(q) = s[host_start..].find('?').map(|p| host_start + p) else {
        out.push_str(&s[host_start..]);
        return Some(out);
    };
    out.push_str(&s[host_start..=q]);
    let params: Vec<String> = s[q + 1..]
        .split('&')
        .map(|param| {
            let (key, value) = param.split_once('=')?;
            if value.contains('=') {
                return None;
            }
            let decoded_key = percent_decode(key)?;
            if !plain_key(&decoded_key) {
                return None;
            }
            if is_secret_key(&decoded_key) {
                return Some(format!("{key}={MASK}"));
            }
            if suspicious_value(&percent_decode(value)?) {
                return None;
            }
            Some(param.to_string())
        })
        .collect::<Option<_>>()?;
    out.push_str(&params.join("&"));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "pgt_gizli_parola_7f3";

    #[test]
    fn conninfo_formats_are_masked_and_other_keys_kept() {
        let cases: &[(&str, &str)] = &[
            // anahtar=değer
            ("host=h password=SECRET dbname=d", "host=h password=******** dbname=d"),
            ("host=h password = SECRET  port=5432", "host=h password=********  port=5432"),
            ("password='SE CR\\'ET' host=h", "password=******** host=h"),
            ("password='SECRET\\\\' host=h", "password=******** host=h"),
            ("password=SE\\ CRET host=h", "password=******** host=h"),
            ("PASSWORD=SECRET host=h", "PASSWORD=******** host=h"),
            ("Password='SECRET'", "Password=********"),
            ("password=SECRET host=h password='SECRET'", "password=******** host=h password=********"),
            ("sslpassword=SECRET sslmode=require", "sslpassword=******** sslmode=require"),
            ("host=h port=5432 user=u", "host=h port=5432 user=u"),
            ("application_name='a b' host=h", "application_name='a b' host=h"),
            ("", ""),
            ("   ", "   "),
            // URI
            ("postgresql://user:SECRET@host:5432/db", "postgresql://user:********@host:5432/db"),
            ("postgres://user:SECRET@host/db?sslmode=require&password=SECRET", "postgres://user:********@host/db?sslmode=require&password=********"),
            ("postgres://host/db?PASSWORD=SECRET", "postgres://host/db?PASSWORD=********"),
            ("postgres://host/db?pass%77ord=SECRET", "postgres://host/db?pass%77ord=********"),
            ("POSTGRESQL://u:SECRET@h", "POSTGRESQL://u:********@h"),
            ("  postgres://u:SECRET@h/db", "  postgres://u:********@h/db"),
            ("postgres://u@h/db", "postgres://u@h/db"),
            ("postgres://h1:5432,h2:5433/db?target_session_attrs=any", "postgres://h1:5432,h2:5433/db?target_session_attrs=any"),
            ("postgres://u:SE?CRET@h/db", "postgres://u:********@h/db"),
            ("postgres://u:@h", "postgres://u:********@h"),
        ];
        for (input, expected) in cases {
            let input = input.replace("SECRET", SECRET);
            let got = mask_conninfo(&input);
            assert_eq!(got, *expected, "girdi: {input}");
            assert!(!got.contains(SECRET), "sır kaldı: {got}");
        }
    }

    #[test]
    fn ambiguous_conninfo_is_masked_entirely() {
        for input in [
            "password='SECRET host=h",           // kapanmamış tırnak
            "password='SECRET\\",                // kaçışla biten kapanmamış tırnak
            "host=h password",                   // `=` yok
            "host=h SECRET",                     // `=` yok
            "password=SECRET\\",                 // sonda tek ters bölü
            "=SECRET",                           // boş anahtar
            "pass-word=SECRET",                  // tanınmayan anahtar biçimi
            "dbname='postgresql://u:SECRET@h'",  // iç içe URI
            "options='-c password=SECRET'",      // değerde parola
            "postgres://u:SE@CRET@h/db",         // birden çok @
            "postgres://u:SE/CRET@h/db",         // / sonrasında @
            "postgres://h/db?password",          // sorguda `=` yok
            "postgres://h/db?password=a=SECRET", // fazladan `=`
            "postgres://h/db?pass%7=SECRET",     // bozuk yüzde kodlaması
            "postgres://h/db?x-y=SECRET",        // tanınmayan anahtar
            "postgres://h/db?options=password%3DSECRET",
        ] {
            let input = input.replace("SECRET", SECRET);
            assert_eq!(mask_conninfo(&input), MASK, "girdi: {input}");
        }
    }

    #[test]
    fn options_mask_password_keys_only() {
        let input: Vec<String> =
            ["user=u", "password=SECRET", "PASSWORD=SECRET", "sslpassword=SECRET", "flag", "password_required=false", "host=a=b"]
                .iter()
                .map(|s| s.replace("SECRET", SECRET))
                .collect();
        assert_eq!(
            mask_options(&input),
            ["user=u", "password=********", "PASSWORD=********", "sslpassword=********", "flag", "password_required=false", "host=a=b"]
        );
        assert!(mask_options(&[]).is_empty());
    }

    #[test]
    fn secret_keys() {
        for k in ["password", "PASSWORD", " Password ", "sslpassword"] {
            assert!(is_secret_key(k), "{k}");
        }
        for k in ["passfile", "user", "password_required", ""] {
            assert!(!is_secret_key(k), "{k}");
        }
    }
}
