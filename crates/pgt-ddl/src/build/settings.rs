//! Ortak ayar (GUC) yardımcısı: `ALTER DATABASE|ROLE … SET name = value` / `RESET name` (Database ve Role üreticileri).
//!
//! Ad katı dilbilgisinden geçer ve her parçası `quote_ident` ile yazılır; değer yalnızca `quote_literal` ile yerleşir.
//! Liste ayarlarında (pg_dump `variable_is_guc_list_quote`) değer PG `SplitGUCList` kuralıyla öğelere bölünür ve her öğe
//! ayrı tırnaklanır (`search_path = '$user', 'public'`); aksi halde tek tırnaklı değer tek bir öğe sayılırdı.

use std::collections::{BTreeMap, HashSet};

use super::err;
use super::type_syntax::is_spoofing_format;
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
/// Ad parçası üst sınırı (bayt; PG tanımlayıcı sınırı NAMEDATALEN-1 — aşan parçayı sunucu sessizce keser).
pub const MAX_GUC_PART_BYTES: usize = 63;
const DUPLICATE_MSG: &str = "Bu ayar birden çok kez tanımlanmış.";
/// GUC değeri üst sınırı (bayt).
pub const MAX_GUC_VALUE_BYTES: usize = 4096;

fn is_list_guc(name: &str) -> bool {
    LIST_QUOTE_GUCS.contains(&name)
}

/// Noktasız (yerleşik) ad: `^[a-z_][a-z0-9_$]*$`; noktalı (özel/eklenti) ad: her parça `[A-Za-z_][A-Za-z0-9_$]*`
/// (PG özel adların yazımını korur: `"App.Mode"` öyle saklanır). Parça ≤ 63 bayt ayrıca denetlenir.
fn valid_guc_name(name: &str) -> bool {
    let dotted = name.contains('.');
    name.split('.').all(|part| {
        let lower_ok = |c: char| c.is_ascii_lowercase() || (dotted && c.is_ascii_uppercase());
        let mut chars = part.chars();
        matches!(chars.next(), Some(c) if lower_ok(c) || c == '_')
            && chars.all(|c| lower_ok(c) || c.is_ascii_digit() || c == '_' || c == '$')
    })
}

/// Ad karşılaştırma anahtarı: PG `guc_name_compare` gibi yalnızca ASCII harf katlanır.
fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Bu yeni girdi alter'da YAZILIR mı (eskiyle aynı değilse)? `diff_settings` ile aynı ölçüt (son yinelenen kazanır):
/// doğrulama tam olarak yazılan girdileri kapsar (inceleme G-1).
fn written(old_map: &BTreeMap<&str, &str>, s: &ConfigSetting) -> bool {
    old_map.get(s.name.as_str()) != Some(&s.value.as_str())
}

fn old_values(old: &[ConfigSetting]) -> BTreeMap<&str, &str> {
    old.iter().map(|s| (s.name.as_str(), s.value.as_str())).collect()
}

/// Ad büyük/küçük harf KARARI (inceleme I-1, G-2): PG `ALTER … SET` sırasında yerleşik adı kanonik yazımına çevirip
/// saklar (`timezone` → `TimeZone`, `DateStyle`, `IntervalStyle`); özel (noktalı) adları ise YAZILDIĞI gibi saklar
/// (`"App.Mode"`) ve başka oturumda yer tutucu olmadığından RESET/SET harfe duyarlı eşleşir. Bu yüzden:
/// - yükleyiciler yalnızca NOKTASIZ adları ASCII küçük harfe çevirir; noktalı adlar saklandığı gibi döner;
/// - YENİ taraf dilbilgisi noktasız adlarda yalnızca küçük harftir (büyük harfli ad açık bir "küçük harfle yazın"
///   iletisiyle reddedilir); noktalı adlarda büyük harf serbesttir (yazım korunur, parçalar `quote_ident` ile);
/// - yinelenen ad denetimi ASCII harfe duyarsızdır (`App.Mode` ile `app.mode` aynı listede olamaz);
/// - ESKİ taraf adları (yalnızca `RESET` hedefi) katı dilbilgisinden geçmez: `validate_old_setting_names` yalnızca
///   boş/NUL/denetim karakteri denetler (yükleyici küçültmeyi unutsa ya da eski bir istemci kanonik adı gönderse bile
///   `RESET "TimeZone"` güvenli ve geçerlidir; ad yalnızca `quote_ident` ile yerleşir).
///
/// `settings[i].name|value` doğrulaması; yinelenen ad → `settings[i].name`.
pub fn validate_settings(list: &[ConfigSetting], errs: &mut Vec<FieldError>) {
    validate_settings_where(list, |_| true, errs);
}

/// Alter: yalnızca YAZILACAK (eklenen ya da değişen; eskiyle birebir aynı olmayan) girdilerin adı/değeri doğrulanır —
/// yüklü bir değer kurala takılsa bile dokunulmayan ayar düzenlemeyi engellemez. Yinelenen ad denetimi ise **her
/// zaman** tüm yeni listede çalışır (inceleme M-1): `[a=2, a=1]` gibi bir listede ikinci girdi yüklü değerle aynı olsa
/// bile hata verir; aksi halde `diff_settings` son girdiyi seçer ve kullanıcının `a=2`'si sessizce kaybolurdu.
pub fn validate_written_settings(old: &[ConfigSetting], new: &[ConfigSetting], errs: &mut Vec<FieldError>) {
    let old_map = old_values(old);
    validate_settings_where(new, |s| written(&old_map, s), errs);
}

/// `check(s)` false ise girdinin ad dilbilgisi ve değeri atlanır; yinelenen ad denetimi yine yapılır.
fn validate_settings_where(list: &[ConfigSetting], check: impl Fn(&ConfigSetting) -> bool, errs: &mut Vec<FieldError>) {
    let mut seen = HashSet::new();
    for (i, s) in list.iter().enumerate() {
        let np = format!("settings[{i}].name");
        let vp = format!("settings[{i}].value");
        if !check(s) {
            if !seen.insert(name_key(&s.name)) {
                errs.push(err(&np, DUPLICATE_MSG));
            }
            continue;
        }
        let name_ok = if s.name.chars().count() > MAX_GUC_NAME {
            errs.push(err(&np, format!("Ayar adı en çok {MAX_GUC_NAME} karakter olabilir.")));
            false
        } else if !s.name.contains('.') && !valid_guc_name(&s.name) && valid_guc_name(&s.name.to_ascii_lowercase()) {
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
                "Ayar adı geçersiz: küçük harf, rakam, '_' ve '$' (ör. work_mem, app.mode); parçalar '.' ile ayrılır \
                 (yalnızca noktalı özel adlarda büyük harf korunur).",
            ));
            false
        } else if s.name.split('.').any(|p| p.len() > MAX_GUC_PART_BYTES) {
            errs.push(err(&np, format!("Ayar adının her parçası en çok {MAX_GUC_PART_BYTES} bayt olabilir.")));
            false
        } else if !seen.insert(name_key(&s.name)) {
            errs.push(err(&np, DUPLICATE_MSG));
            false
        } else {
            true
        };
        if s.value.contains('\0') {
            errs.push(err(&vp, "Değer NUL karakteri içeremez."));
        } else if s.value.chars().any(char::is_control) {
            errs.push(err(&vp, "Değer denetim karakteri (satır sonu, sekme vb.) içeremez."));
        } else if s.value.chars().any(is_spoofing_format) {
            errs.push(err(&vp, "Değer görünmez biçim/yön karakteri (ör. U+202E, U+200B) içeremez."));
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
        } else if s.name.split('.').any(str::is_empty) {
            // PG boş parçalı ad saklamaz; `RESET "".x` yalnızca sözdizimi hatası olurdu (inceleme G-4).
            errs.push(err(&np, "Ayar adında boş parça olamaz ('.x', 'a..b', 'a.')."));
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
/// Yazılan her girdi `validate_written_settings`'in `written` ölçütüyle doğrulanmıştır (aynı işlev).
pub fn diff_settings<'a>(old: &'a [ConfigSetting], new: &'a [ConfigSetting]) -> (Vec<&'a str>, Vec<&'a ConfigSetting>) {
    let old_map = old_values(old);
    let new_map: BTreeMap<&str, &ConfigSetting> = new.iter().map(|s| (s.name.as_str(), s)).collect();
    let removed = old_map.keys().filter(|n| !new_map.contains_key(*n)).copied().collect();
    let set = new_map.values().filter(|s| written(&old_map, s)).copied().collect();
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
        // 128 karakter (parça başına ≤ 63 bayt) sınırda kabul.
        let max = format!("{}.{}.c", "a".repeat(63), "b".repeat(62));
        assert_eq!(max.len(), 128);
        assert!(paths(&[s(&max, "1")]).is_empty());
        let long = "a".repeat(129);
        for bad in ["", "Work_Mem", "a-b", "a..b", ".a", "a.", "a b", "a;b", "1a", long.as_str(), "a\0", "a\"b", "ş", "a.1b", "App.ş"] {
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
    fn written_settings_keep_duplicate_errors() {
        let w = |old: &[ConfigSetting], new: &[ConfigSetting]| {
            let mut e = vec![];
            validate_written_settings(old, new, &mut e);
            e.into_iter().map(|e| (e.path, e.message)).collect::<Vec<_>>()
        };
        let dup = |i: usize| vec![(format!("settings[{i}].name"), DUPLICATE_MSG.to_string())];
        // İnceleme M-1: yinelenen girdi yüklü değerle aynı olsa da hata (her iki sıra).
        let old = [s("a", "1")];
        assert_eq!(w(&old, &[s("a", "2"), s("a", "1")]), dup(1));
        assert_eq!(w(&old, &[s("a", "1"), s("a", "2")]), dup(1));
        assert_eq!(w(&old, &[s("a", "1"), s("a", "1")]), dup(1));
        // Değişmeyen yüklü girdi (büyük harfli ad, denetim karakterli değer) doğrulanmaz; yazılan girdi katı.
        let old = [s("TimeZone", "UTC"), s("work_mem", "a\nb")];
        assert!(w(&old, &old).is_empty());
        assert_eq!(w(&old, &[s("TimeZone", "UTC"), s("work_mem", "b\nc")])[0].0, "settings[1].value");
        // Yinelenen değişmeyen girdi de yakalanır; tam doğrulamayla aynı yol.
        assert_eq!(w(&old, &[s("TimeZone", "UTC"), s("TimeZone", "UTC")]), dup(1));
        assert_eq!(paths(&[s("a", "2"), s("a", "1")]), ["settings[1].name"]);
    }

    #[test]
    fn written_settings_validate_every_emitted_entry_when_old_has_duplicate_names() {
        let nul = format!("x{}X", char::from(0));
        for bad in [nul.as_str(), "x\nX", "a\u{202e}b"] {
            // Eski tarafta aynı ad iki kez (son kazanır: `1`); yeni girdi ilk (son olmayan) eski değere eşit → yazılır.
            let old = [s("app.a", bad), s("app.a", "1")];
            let new = [s("app.a", bad)];
            let (_, set) = diff_settings(&old, &new);
            assert_eq!(set.len(), 1, "yazılıyor");
            let mut e = vec![];
            validate_written_settings(&old, &new, &mut e);
            assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["settings[0].value"], "{bad:?}");
        }
        // Ad tarafı: büyük harfli noktasız ad aynı yolla yazılırdı.
        let old = [s("Work_Mem", "1"), s("Work_Mem", "2")];
        let new = [s("Work_Mem", "1")];
        let mut e = vec![];
        validate_written_settings(&old, &new, &mut e);
        assert_eq!(e[0].path, "settings[0].name");
        // Üreticiler doğrulamadan önce yazmaz: rol ve veritabanı alter'ı panik yerine alan hatası döndürür.
        use crate::build::{BuildCtx, alter};
        use crate::model::ObjectDef;
        let ctx = BuildCtx { version_num: 160000, concurrently: false };
        let role = |settings: Vec<ConfigSetting>| {
            ObjectDef::Role(crate::model::RoleDef {
                name: "r".into(),
                login: false,
                superuser: false,
                createdb: false,
                createrole: false,
                inherit: true,
                replication: false,
                bypassrls: false,
                connection_limit: -1,
                valid_until: None,
                password: crate::model::PasswordChange::Keep,
                member_of: vec![],
                settings,
                comment: None,
                unsupported: vec![],
            })
        };
        let r = alter(&role(vec![s("app.a", &nul), s("app.a", "1")]), &role(vec![s("app.a", &nul)]), &ctx);
        assert!(r.is_err_and(|e| e.iter().any(|e| e.path == "settings[0].value")));
        let db = |settings: Vec<ConfigSetting>| {
            ObjectDef::Database(crate::model::DatabaseDef {
                name: "d".into(),
                owner: Some("o".into()),
                template: None,
                encoding: Some("UTF8".into()),
                locale_provider: None,
                lc_collate: Some("C".into()),
                lc_ctype: Some("C".into()),
                provider_locale: None,
                tablespace: Some("pg_default".into()),
                connection_limit: -1,
                is_template: false,
                allow_connections: true,
                settings,
                comment: None,
            })
        };
        let r = alter(&db(vec![s("app.a", &nul), s("app.a", "1")]), &db(vec![s("app.a", &nul)]), &ctx);
        assert!(r.is_err_and(|e| e.iter().any(|e| e.path == "settings[0].value")));
    }

    #[test]
    fn guc_name_parts_over_63_bytes_are_rejected() {
        let ok = format!("app.{}", "a".repeat(63));
        assert!(paths(&[s(&ok, "1")]).is_empty());
        let bad = format!("app.{}", "a".repeat(64));
        assert_eq!(paths(&[s(&bad, "1")]), ["settings[0].name"]);
        let mut e = vec![];
        validate_settings(&[s(&"b".repeat(64), "1")], &mut e);
        assert!(e[0].message.contains("63"), "{e:?}");
    }

    #[test]
    fn old_setting_names_reject_empty_segments() {
        let mut e = vec![];
        validate_old_setting_names("old.", &[s(".x", ""), s("a..b", ""), s("a.", ""), s("App.Mode", ""), s("TimeZone", "")], &mut e);
        let p: Vec<_> = e.into_iter().map(|e| e.path).collect();
        assert_eq!(p, ["old.settings[0].name", "old.settings[1].name", "old.settings[2].name"]);
    }

    #[test]
    fn setting_values_reject_spoofing_format_chars() {
        for bad in ["a\u{202e}b", "\u{200b}", "x\u{2066}y", "\u{feff}"] {
            assert_eq!(paths(&[s("work_mem", bad)]), ["settings[0].value"], "{bad:?}");
        }
        assert!(paths(&[s("app.note", "çağrı ünlü")]).is_empty());
    }

    #[test]
    fn dotted_custom_names_keep_case_and_duplicates_fold_ascii() {
        assert!(paths(&[s("App.Mode", "x")]).is_empty());
        assert_eq!(guc_name_sql("App.Mode"), "\"App\".\"Mode\"");
        // Noktasız ad yine küçük harf ister.
        let mut e = vec![];
        validate_settings(&[s("Work_Mem", "1")], &mut e);
        assert!(e[0].message.contains("küçük harfle yazılmalı"), "{e:?}");
        // ASCII harfe duyarsız yinelenen.
        assert_eq!(paths(&[s("App.Mode", "x"), s("app.mode", "y")]), ["settings[1].name"]);
    }

    #[test]
    fn guc_names_quote_each_part() {
        assert_eq!(guc_name_sql("work_mem"), "work_mem");
        assert_eq!(guc_name_sql("app.user"), "app.\"user\"");
        assert_eq!(guc_name_sql("select.from"), "\"select\".\"from\"");
        assert_eq!(guc_name_sql("a_b.c$d"), "a_b.c$d");
    }
}
