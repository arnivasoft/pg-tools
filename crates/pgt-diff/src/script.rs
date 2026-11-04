//! Betik metni yardımcıları (Task 7 bu dosyayı genişletir).

/// Tek satırlık SQL yorumuna güvenle girecek metin: denetim karakterleri (`\n`, `\r`, `\t` dahil) ve
/// U+2028/U+2029 `?` olur; böylece ileti yorumdan kaçıp ifade başlatamaz.
pub fn comment_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                '?'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_safe_denetim_karakterlerini_soru_isaretine_cevirir() {
        assert_eq!(comment_safe("a\nb\r\u{2028}c\td"), "a?b??c?d");
    }
}
