//! Tırnaklama yardımcılarının gerçek sunucuyla çapraz doğrulaması (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Ölçüt (sunucunun `quote_ident` çıktısıyla bayt eşitliği DEĞİL; bizim `$` içeren adları tırnaksız bırakmamız
//! geçerlidir): (1) bizim çıktımız tanımlayıcı olarak çalışır ve sütun adı geri gelir; (2) bizim çıktımız
//! gerekenden DAHA AZ tırnaklı değildir: tırnaksız bıraktığımız her ad sunucuda da tırnaksızdır ya da yalnız `$`
//! yüzünden tırnaklanır.

mod common;

use common::{servers, version_num};
use pgt_core::testing::admin_client;
use pgt_ddl::quote::KEYWORDS;
use pgt_ddl::{quote_ident, quote_literal};

/// Sabit tohumlu xorshift (yeniden üretilebilir).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Ad parçaları: sıradan, boşluk, tırnaklar, noktalı virgül, ters bölü, yorum, `$`, çok baytlı ve büyük harf.
const ATOMS: &[&str] = &[
    "a", "b", "z", "_", "0", "7", "x1", "name", "A", "Zed", " ", "  ", "\"", "\"\"", ";", "'", "\\", "--", "/*", "*/", "$", "$$", ".", ",", "(", ")",
    "ş", "ğ", "ü", "€", "日本", "😀", "é", "select", "table", "order", "user", "end",
];

/// En çok 63 bayt (PG tanımlayıcı sınırı), denetim karakteri/NUL yok.
fn random_name(rng: &mut Rng) -> String {
    let target = 1 + rng.below(63);
    let mut s = String::new();
    while s.len() < target {
        let atom = ATOMS[rng.below(ATOMS.len())];
        if s.len() + atom.len() > 63 {
            break;
        }
        s.push_str(atom);
    }
    if s.is_empty() { "a".to_string() } else { s }
}

fn fixed_names() -> Vec<String> {
    let mut v: Vec<String> = [
        "a", "A", "a b", "\"", "\"\"", "a\"b", ";", "'", "\\", "--", "/*", " ", "1a", "_x", "$x", "a$b", "日本語", "select", "Select", "user", "plain_name_1",
        "x; DROP TABLE t; --", "it's", "a\\b", "ş", "pg_catalog", "information_schema",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push("a".repeat(63));
    v.push("ş".repeat(31)); // 62 bayt
    v.push(format!("{}a", "ş".repeat(31))); // 63 bayt
    v.push("€".repeat(21)); // 63 bayt
    // Uzun adın çok baytlı karakter ortasında 63 bayta kesilmiş hali (karakter sınırına geri çekilir).
    let long = "ş".repeat(60);
    let mut cut = 63;
    while !long.is_char_boundary(cut) {
        cut -= 1;
    }
    v.push(long[..cut].to_string());
    v
}

#[tokio::test]
async fn keywords_cover_the_server_list() {
    for ts in servers() {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let rows = admin.query("SELECT lower(word) FROM pg_get_keywords()", &[]).await.unwrap();
        let missing: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).filter(|w| KEYWORDS.binary_search(&w.as_str()).is_err()).collect();
        assert!(missing.is_empty(), "PG {} sunucusunda olup KEYWORDS'te olmayan sözcükler: {missing:?}", version_num(&ts).await);
    }
}

#[tokio::test]
async fn keywords_list_is_sorted_and_lowercase() {
    assert!(KEYWORDS.windows(2).all(|w| w[0] < w[1]), "KEYWORDS sıralı ve tekrarsız olmalı");
    assert!(KEYWORDS.iter().all(|w| *w == w.to_lowercase()));
}

#[tokio::test]
async fn quote_ident_works_and_is_never_under_quoted() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut names: Vec<String> = KEYWORDS.iter().map(|k| k.to_string()).collect();
    names.extend(fixed_names());
    names.extend((0..300).map(|_| random_name(&mut rng)));
    for ts in servers() {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        for name in &names {
            let ours = quote_ident(name);
            // (1) tanımlayıcı olarak çalışır: sütun etiketi ve (ayrılmış sözcükleri de yakalayan) tablo takma adı.
            let row = admin.query_one(&format!("SELECT 1 AS {ours}"), &[]).await.unwrap_or_else(|e| panic!("{name:?} → {ours}: {e}"));
            assert_eq!(row.columns()[0].name(), name, "sütun adı geri gelmedi: {ours}");
            admin
                .query_one(&format!("SELECT {ours}.a FROM (SELECT 1 AS a) AS {ours}"), &[])
                .await
                .unwrap_or_else(|e| panic!("{name:?} takma ad olarak çalışmadı ({ours}): {e}"));
            // (2) gerekenden az tırnaklı değil.
            if !ours.starts_with('"') {
                let server: String = admin.query_one("SELECT quote_ident($1::text)", &[name]).await.unwrap().get(0);
                assert!(server == *name || name.contains('$'), "{name:?} tırnaksız bırakıldı ama sunucu {server} yazıyor");
            }
        }
    }
}

#[tokio::test]
async fn quote_literal_round_trips_in_both_string_modes() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    const PARTS: &[&str] = &["a", "b", " ", "'", "''", "\\", "\\\\", "\\n", "\n", "\t", "\"", ";", "--", "ş", "€", "😀", "%", "$$", "E'", "x"];
    let mut values: Vec<String> = vec![String::new(), "it's".into(), "a\\b".into(), "\\".into(), "'".into(), "'; DROP TABLE t; --".into(), "\\'".into(), "\r\n".into()];
    for _ in 0..300 {
        let n = 1 + rng.below(12);
        values.push((0..n).map(|_| PARTS[rng.below(PARTS.len())]).collect());
    }
    for ts in servers() {
        for scs in ["on", "off"] {
            let admin = admin_client(&ts, &ts.config.maintenance_db).await;
            admin.batch_execute(&format!("SET standard_conforming_strings = {scs}; SET escape_string_warning = off")).await.unwrap();
            for v in &values {
                let lit = quote_literal(v);
                let got: String = admin.query_one(&format!("SELECT {lit}::text"), &[]).await.unwrap_or_else(|e| panic!("{v:?} → {lit} (scs={scs}): {e}")).get(0);
                assert_eq!(&got, v, "scs={scs}: {lit}");
            }
        }
    }
}
