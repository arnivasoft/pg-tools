//! Ortak ayar (GUC) yardımcısı: `ALTER DATABASE|ROLE … SET name = value` / `RESET name` (Database ve Role üreticileri).
//!
//! Ad katı dilbilgisinden geçer ve her parçası `quote_ident` ile yazılır; değer yalnızca `quote_literal` ile yerleşir.
//! Liste ayarlarında (pg_dump `variable_is_guc_list_quote`) değer PG `SplitGUCList` kuralıyla öğelere bölünür ve her öğe
//! ayrı tırnaklanır (`search_path = '$user', 'public'`); aksi halde tek tırnaklı değer tek bir öğe sayılırdı.

use std::collections::{BTreeMap, HashSet};

use super::err;
use crate::model::{ConfigSetting, FieldError};
use crate::quote::{quote_ident, quote_literal};

/// pg_dump `variable_is_guc_list_quote` kümesi (değeri öğe öğe tırnaklanan liste ayarları).
pub const LIST_QUOTE_GUCS: [&str; 6] = [
    "local_preload_libraries",
    "search_path",
    "session_preload_libraries",
    "shared_preload_libraries",
    "temp_tablespaces",
    "unix_socket_directories",
];

/// GUC adı üst sınırı (karakter; dilbilgisi yalnızca ASCII kabul eder).
pub const MAX_GUC_NAME: usize = 128;
/// GUC değeri üst sınırı (bayt).
pub const MAX_GUC_VALUE_BYTES: usize = 4096;

fn is_list_guc(name: &str) -> bool {
    LIST_QUOTE_GUCS.contains(&name)
}

/// `^[a-z_][a-z0-9_$]*(\.[a-z_][a-z0-9_$]*)*$`
fn valid_guc_name(name: &str) -> bool {
    name.split('.').all(|part| {
        let mut chars = part.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$')
    })
}

/// Ad büyük/küçük harf KARARI (inceleme I-1): PG `ALTER … SET` sırasında adı kanonik yazımına çevirip saklar
/// (`timezone` → `TimeZone`, `DateStyle`, `IntervalStyle`). Bu yüzden:
/// - yükleyiciler (Task 6/7) ayar adlarını **küçük harfe çevirerek** döndürür (GUC araması harfe duyarsızdır);
/// - YENİ taraf dilbilgisi yalnızca küçük harftir; büyük harfli ad açık bir "küçük harfle yazın" iletisiyle reddedilir;
/// - ESKİ taraf adları (yalnızca `RESET` hedefi) katı dilbilgisinden geçmez: `validate_old_setting_names` yalnızca
///   boş/NUL/denetim karakteri denetler (yükleyici küçültmeyi unutsa ya da eski bir istemci kanonik adı gönderse bile
///   `RESET "TimeZone"` güvenli ve geçerlidir; ad yalnızca `quote_ident` ile yerleşir).
///
/// `settings[i].name|value` doğrulaması; yinelenen ad → `settings[i].name`.
pub fn validate_settings(list: &[ConfigSetting], errs: &mut Vec<FieldError>) {
    let mut seen = HashSet::new();
    for (i, s) in list.iter().enumerate() {
        let np = format!("settings[{i}].name");
        let vp = format!("settings[{i}].value");
        let name_ok = if s.name.chars().count() > MAX_GUC_NAME {
            errs.push(err(&np, format!("Ayar adı en çok {MAX_GUC_NAME} karakter olabilir.")));
            false
        } else if !valid_guc_name(&s.name) && valid_guc_name(&s.name.to_ascii_lowercase()) {
            errs.push(err(
                &np,
                format!(
                    "Ayar adı küçük harfle yazılmalı: '{}' (PostgreSQL ayar adlarını büyük/küçük harfe duyarsız eşler).",
                    s.name.to_ascii_lowercase()
                ),
            ));
            false
        } else if !valid_guc_name(&s.name) {
            errs.push(err(
                &np,
                "Ayar adı geçersiz: küçük harf, rakam, '_' ve '$' (ör. work_mem, app.mode); parçalar '.' ile ayrılır.",
            ));
            false
        } else if !seen.insert(s.name.as_str()) {
            errs.push(err(&np, "Bu ayar birden çok kez tanımlanmış."));
            false
        } else {
            true
        };
        if s.value.contains('\0') {
            errs.push(err(&vp, "Değer NUL karakteri içeremez."));
        } else if s.value.chars().any(char::is_control) {
            errs.push(err(&vp, "Değer denetim karakteri (satır sonu, sekme vb.) içeremez."));
        } else if s.value.len() > MAX_GUC_VALUE_BYTES {
            errs.push(err(&vp, format!("Değer en çok {MAX_GUC_VALUE_BYTES} bayt olabilir.")));
        } else if name_ok
            && is_list_guc(&s.name)
            && let Err(m) = split_guc_list(&s.value)
        {
            errs.push(err(&vp, m));
        }
    }
}

/// ESKİ (yüklü) taraf ayar adları: yalnızca `RESET <ad>` hedefi olarak yazılır; boş/NUL/denetim karakteri denetlenir,
/// katı dilbilgisi uygulanmaz (bkz. `validate_settings` üstündeki karar). Yol: `<prefix>settings[i].name`.
pub fn validate_old_setting_names(prefix: &str, list: &[ConfigSetting], errs: &mut Vec<FieldError>) {
    for (i, s) in list.iter().enumerate() {
        let np = format!("{prefix}settings[{i}].name");
        if s.name.is_empty() {
            errs.push(err(&np, "Ayar adı boş olamaz."));
        } else if s.name.contains('\0') {
            errs.push(err(&np, "Ayar adı NUL karakteri içeremez."));
        } else if s.name.chars().any(char::is_control) {
            errs.push(err(&np, "Ayar adı denetim karakteri (satır sonu, sekme vb.) içeremez."));
        }
    }
}

/// PG `scanner_isspace`.
fn is_guc_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c' | '\x0b')
}

/// PG `SplitGUCList`: virgülle ayrılmış, `"…"` öğeler (`""` kaçış), baş/son boşluk kırpılır. Kapanmamış tırnak → Err.
/// Boş metin boş listedir; tırnaksız boş öğe (`a,,b`, `a,`) ve öğeden sonra ayırıcı dışı karakter (`a b`) de Err.
pub fn split_guc_list(value: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    let skip_space = |i: &mut usize| {
        while *i < chars.len() && is_guc_space(chars[*i]) {
            *i += 1;
        }
    };
    let mut out = vec![];
    skip_space(&mut i);
    if i == chars.len() {
        return Ok(out);
    }
    loop {
        let mut item = String::new();
        if chars.get(i) == Some(&'"') {
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("Liste değerinde kapanmamış çift tırnak (\") var.".into()),
                    Some('"') if chars.get(i + 1) == Some(&'"') => {
                        item.push('"');
                        i += 2;
                    }
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some(&c) => {
                        item.push(c);
                        i += 1;
                    }
                }
            }
        } else {
            while i < chars.len() && chars[i] != ',' && !is_guc_space(chars[i]) {
                item.push(chars[i]);
                i += 1;
            }
            if item.is_empty() {
                return Err("Liste değerinde boş öğe var (art arda ya da sonda virgül).".into());
            }
        }
        skip_space(&mut i);
        out.push(item);
        match chars.get(i) {
            None => return Ok(out),
            Some(',') => {
                i += 1;
                skip_space(&mut i);
            }
            Some(_) => {
                return Err("Liste değeri geçersiz: öğeler virgülle ayrılmalı; boşluk içeren öğeyi \"…\" içine alın.".into());
            }
        }
    }
}

/// `<parça>.<parça>` → her parça quote_ident.
pub fn guc_name_sql(name: &str) -> String {
    name.split('.').map(quote_ident).collect::<Vec<_>>().join(".")
}

/// `name = <değer>`: liste ayarında öğe başına quote_literal (', ' ile), diğerlerinde tek quote_literal.
/// Doğrulanmış girdi varsayılır; bölünemeyen liste değeri (doğrulamayı atlayan çağıran) tek değer olarak yazılır.
pub fn guc_assignment_sql(s: &ConfigSetting) -> String {
    let value = match is_list_guc(&s.name).then(|| split_guc_list(&s.value)) {
        Some(Ok(items)) if items.is_empty() => "''".to_string(),
        Some(Ok(items)) => items.iter().map(|v| quote_literal(v)).collect::<Vec<_>>().join(", "),
        _ => quote_literal(&s.value),
    };
    format!("{} = {value}", guc_name_sql(&s.name))
}

/// Fark: (kaldırılan adlar, eklenen/değişen ayarlar), her ikisi ad sırasıyla (deterministik).
pub fn diff_settings<'a>(old: &'a [ConfigSetting], new: &'a [ConfigSetting]) -> (Vec<&'a str>, Vec<&'a ConfigSetting>) {
    let old_map: BTreeMap<&str, &str> = old.iter().map(|s| (s.name.as_str(), s.value.as_str())).collect();
    let new_map: BTreeMap<&str, &ConfigSetting> = new.iter().map(|s| (s.name.as_str(), s)).collect();
    let removed = old_map.keys().filter(|n| !new_map.contains_key(*n)).copied().collect();
    let set = new_map
        .values()
        .filter(|s| old_map.get(s.name.as_str()) != Some(&s.value.as_str()))
        .copied()
        .collect();
    (removed, set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: &str, v: &str) -> ConfigSetting {
        ConfigSetting { name: n.into(), value: v.into() }
    }

    #[test]
    fn guc_lists_are_split_and_quoted_per_element() {
        let s = |n: &str, v: &str| ConfigSetting { name: n.into(), value: v.into() };
        assert_eq!(guc_assignment_sql(&s("search_path", "\"$user\", public")), "search_path = '$user', 'public'");
        assert_eq!(guc_assignment_sql(&s("search_path", "\"My \"\"S\"\"\", app")), "search_path = 'My \"S\"', 'app'");
        assert_eq!(guc_assignment_sql(&s("work_mem", "64MB")), "work_mem = '64MB'");
        // Plan metni `app.mode` bekliyordu; `mode` PG anahtar sözcüğüdür ve `quote_ident` tüm anahtar sözcükleri tırnaklar.
        assert_eq!(guc_assignment_sql(&s("app.mode", "it's")), "app.\"mode\" = 'it''s'");
        assert_eq!(guc_assignment_sql(&s("app.kind", "it's")), "app.kind = 'it''s'");
        assert_eq!(guc_assignment_sql(&s("app.user", "x")), "app.\"user\" = 'x'"); // anahtar sözcük parçası tırnaklı
        assert_eq!(guc_assignment_sql(&s("app.p", "a\\b")), "app.p = E'a\\\\b'");
        assert!(split_guc_list("\"unterminated, x").is_err());
        assert_eq!(split_guc_list(" a ,b,  \"c d\" ").unwrap(), vec!["a", "b", "c d"]);
    }

    #[test]
    fn split_guc_list_follows_pg_rules() {
        assert_eq!(split_guc_list("").unwrap(), Vec::<String>::new());
        assert_eq!(split_guc_list("  ").unwrap(), Vec::<String>::new());
        assert_eq!(split_guc_list("\"\"").unwrap(), vec![""]);
        assert_eq!(split_guc_list("\"a,b\" , c").unwrap(), vec!["a,b", "c"]);
        // Tırnaksız öğeler küçük harfe çevrilmez (SplitGUCList, SplitIdentifierString değil).
        assert_eq!(split_guc_list("Public").unwrap(), vec!["Public"]);
        for bad in ["a,,b", "a,", ",a", "a b", "\"a\"b", "\"a", "\"a\"\""] {
            assert!(split_guc_list(bad).is_err(), "{bad:?}");
        }
        // Boş liste değeri `''` yazılır; hostile öğeler tek literal kalır.
        assert_eq!(guc_assignment_sql(&s("search_path", "")), "search_path = ''");
        assert_eq!(
            guc_assignment_sql(&s("search_path", "\"x'; DROP ROLE r; --\", \"a\\b\"")),
            "search_path = 'x''; DROP ROLE r; --', E'a\\\\b'"
        );
        assert_eq!(guc_assignment_sql(&s("temp_tablespaces", "ts1,ts2")), "temp_tablespaces = 'ts1', 'ts2'");
        // Liste dışı ayarda virgül/tırnak olduğu gibi tek değer.
        assert_eq!(guc_assignment_sql(&s("app.list", "\"a\", b")), "app.list = '\"a\", b'");
    }

    fn paths(list: &[ConfigSetting]) -> Vec<String> {
        let mut e = vec![];
        validate_settings(list, &mut e);
        e.into_iter().map(|e| e.path).collect()
    }

    #[test]
    fn setting_names_and_values_are_validated() {
        for ok in ["work_mem", "app.mode", "a_b.c$d", "_x", "a1.b2"] {
            assert!(paths(&[s(ok, "1")]).is_empty(), "{ok:?}");
        }
        assert!(paths(&[s(&"a".repeat(128), "1")]).is_empty());
        let long = "a".repeat(129);
        for bad in ["", "Work_Mem", "a-b", "a..b", ".a", "a.", "a b", "a;b", "1a", long.as_str(), "a\0", "a\"b", "ş", "a.1b"] {
            assert_eq!(paths(&[s(bad, "1")]), ["settings[0].name"], "{bad:?}");
        }
        let big = "x".repeat(4097);
        for bad in ["a\0b", "a\nb", "a\rb", "a\tb", "a\u{85}b", big.as_str()] {
            assert_eq!(paths(&[s("work_mem", bad)]), ["settings[0].value"], "{bad:?}");
        }
        assert!(paths(&[s("work_mem", &"x".repeat(4096))]).is_empty());
        assert!(paths(&[s("work_mem", "")]).is_empty());
        // Yinelenen ad: ikinci girdinin adı.
        assert_eq!(paths(&[s("work_mem", "1"), s("work_mem", "2")]), ["settings[1].name"]);
        // Liste ayarında bölünemeyen değer → değer hatası; ad ve değer hataları birlikte raporlanır.
        assert_eq!(paths(&[s("search_path", "\"unterminated, x")]), ["settings[0].value"]);
        assert_eq!(paths(&[s("A", "a\nb"), s("search_path", "a, b")]), ["settings[0].name", "settings[0].value"]);
    }

    #[test]
    fn settings_diff_is_sorted_reset_then_set() {
        let old = [s("a", "1"), s("b", "2"), s("c", "3")];
        let new = [s("c", "3"), s("b", "9"), s("d", "4")];
        let (removed, set) = diff_settings(&old, &new);
        assert_eq!(removed, ["a"]);
        let (b9, d4) = (s("b", "9"), s("d", "4"));
        assert_eq!(set, [&b9, &d4]);
        let (removed, set) = diff_settings(&old, &old);
        assert!(removed.is_empty() && set.is_empty());
        // Sıra girdi sırasından bağımsız (ad sırası).
        let (old, new) = ([s("z", "1"), s("m", "1")], [s("y", "1"), s("b", "1")]);
        let (removed, set) = diff_settings(&old, &new);
        assert_eq!(removed, ["m", "z"]);
        assert_eq!(set.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["b", "y"]);
    }

    #[test]
    fn canonical_mixed_case_names_old_side_lenient_new_side_lowercase() {
        // Yeni taraf: PG'nin kanonik yazımı (TimeZone) reddedilir, ileti küçük harfli adı söyler.
        let mut e = vec![];
        validate_settings(&[s("TimeZone", "UTC")], &mut e);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].path, "settings[0].name");
        assert!(e[0].message.contains("küçük harf") && e[0].message.contains("'timezone'"), "{e:?}");
        // Dilbilgisine küçültmeyle de uymayan ad genel iletiyi alır.
        let mut e = vec![];
        validate_settings(&[s("Time-Zone", "UTC")], &mut e);
        assert!(!e[0].message.contains("küçük harfle yazılmalı"), "{e:?}");
        // Eski taraf (RESET hedefi): kanonik ad kabul, `RESET "TimeZone"` güvenli biçimde yazılır.
        let mut e = vec![];
        validate_old_setting_names("old.", &[s("TimeZone", "UTC"), s("DateStyle", "ISO, MDY"), s("x\"; --", "")], &mut e);
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(guc_name_sql("TimeZone"), "\"TimeZone\"");
        assert_eq!(guc_name_sql("x\"; --"), "\"x\"\"; --\"");
        // Eski tarafta yalnızca boş/NUL/denetim karakteri reddedilir.
        let mut e = vec![];
        validate_old_setting_names("old.", &[s("", ""), s("a\0", ""), s("a\nb", ""), s("ok", "")], &mut e);
        let p: Vec<_> = e.into_iter().map(|e| e.path).collect();
        assert_eq!(p, ["old.settings[0].name", "old.settings[1].name", "old.settings[2].name"]);
    }

    #[test]
    fn guc_names_quote_each_part() {
        assert_eq!(guc_name_sql("work_mem"), "work_mem");
        assert_eq!(guc_name_sql("app.user"), "app.\"user\"");
        assert_eq!(guc_name_sql("select.from"), "\"select\".\"from\"");
        assert_eq!(guc_name_sql("a_b.c$d"), "a_b.c$d");
    }
}
