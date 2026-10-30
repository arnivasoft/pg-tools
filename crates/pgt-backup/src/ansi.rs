//! Windows ANSI kod sayfası gidiş-dönüş denetimi (spec §5 "Bilinen sınırlar" (b)).
//!
//! PostgreSQL istemci araçları ANSI `main` kullanır: CRT argv'yi `GetCommandLineA` ile, yani UTF-16 komut satırını
//! sistem ANSI kod sayfasına (`CP_ACP`) **en yakın eşleme (best-fit)** ile çevirip yeniden ayrıştırır. Bu
//! çeviride örneğin U+FF02 (tam genişlikte tırnak `＂`) düz `"` olur, U+FF0D (`－`) `-` olur. Böyle karakter
//! içeren bir yol, tek argüman olarak verilse bile araçta tırnağı kapatıp yeni argümanlar (`-h kötü.örnek`:
//! `PGPASSWORD` başka sunucuya gider; `--clean`: kapı atlatma) üretebilir. Ortam değerleri de (sertifika yolları,
//! `PGHOST`) aynı çeviriyle okunur ve başka bir dosyayı/adresi gösterebilir.
//!
//! Kural: sabit ASCII olmayan her argv öğesi ve ortam değeri kod sayfasında **birebir** temsil edilmelidir
//! (best-fit yok, varsayılan karakter yok, geri çözüm aynı metni verir). Aksi plan hatasıdır.

/// Değer sistem ANSI kod sayfasında birebir gidiş-dönüş yapıyor mu. Windows dışında her zaman `true`
/// (araçlar argv'yi bayt olarak alır).
pub fn ensure_ansi_roundtrip(s: &str) -> bool {
    #[cfg(windows)]
    {
        // SAFETY: argümansız, yan etkisiz Win32 çağrısı.
        let cp = unsafe { windows_sys::Win32::Globalization::GetACP() };
        ensure_roundtrip_cp(s, cp)
    }
    #[cfg(not(windows))]
    {
        let _ = s;
        true
    }
}

/// Değer `cp` kod sayfasında birebir gidiş-dönüş yapıyor mu (testler için kod sayfası açıkça verilir).
/// ASCII her ANSI kod sayfasında değişmezdir (çok baytlı sayfaların öncü baytları ≥ 0x80). UTF-8 (65001) her
/// geçerli metni birebir taşır. Çeviri başarısızsa (ör. bayrakları desteklemeyen kod sayfası) `false`: kapalı
/// kalınır.
#[cfg(windows)]
pub fn ensure_roundtrip_cp(s: &str, cp: u32) -> bool {
    if s.is_ascii() || cp == 65001 {
        return true;
    }
    let wide: Vec<u16> = s.encode_utf16().collect();
    let Some(bytes) = wide_to_multibyte(cp, &wide) else { return false };
    multibyte_to_wide(cp, &bytes).is_some_and(|back| back == wide)
}

/// `WideCharToMultiByte(cp, WC_NO_BEST_FIT_CHARS, …)`; varsayılan karakter kullanıldıysa ya da çeviri
/// başarısızsa `None`.
#[cfg(windows)]
fn wide_to_multibyte(cp: u32, wide: &[u16]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Globalization::{WC_NO_BEST_FIT_CHARS, WideCharToMultiByte};
    let len = i32::try_from(wide.len()).ok()?;
    let mut used_default: windows_sys::core::BOOL = 0;
    // SAFETY: `wide` `len` öğe geçerlidir; ilk çağrı yalnız boyut sorar (çıktı NULL, boyut 0).
    // `used_default` çağrı boyunca yaşayan yerel değişkendir.
    let need = unsafe {
        WideCharToMultiByte(cp, WC_NO_BEST_FIT_CHARS, wide.as_ptr(), len, std::ptr::null_mut(), 0, std::ptr::null(), &mut used_default)
    };
    if need <= 0 || used_default != 0 {
        return None;
    }
    let mut out = vec![0u8; usize::try_from(need).ok()?];
    // SAFETY: `out` tam `need` bayttır; girdi ilk çağrıdakiyle aynıdır.
    let written = unsafe {
        WideCharToMultiByte(cp, WC_NO_BEST_FIT_CHARS, wide.as_ptr(), len, out.as_mut_ptr(), need, std::ptr::null(), &mut used_default)
    };
    (written == need && used_default == 0).then_some(out)
}

/// `MultiByteToWideChar(cp, MB_ERR_INVALID_CHARS, …)`; geçersiz dizide `None`.
#[cfg(windows)]
fn multibyte_to_wide(cp: u32, bytes: &[u8]) -> Option<Vec<u16>> {
    use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: `bytes` `len` bayt geçerlidir; ilk çağrı yalnız boyut sorar.
    let need = unsafe { MultiByteToWideChar(cp, MB_ERR_INVALID_CHARS, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return None;
    }
    let mut out = vec![0u16; usize::try_from(need).ok()?];
    // SAFETY: `out` tam `need` öğedir.
    let written = unsafe { MultiByteToWideChar(cp, MB_ERR_INVALID_CHARS, bytes.as_ptr(), len, out.as_mut_ptr(), need) };
    (written == need).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_always_round_trips() {
        assert!(ensure_ansi_roundtrip(r#"C:\Yedek\a b"c.dump"#));
        assert!(ensure_ansi_roundtrip(""));
    }

    #[cfg(windows)]
    #[test]
    fn best_fit_characters_are_rejected_in_explicit_code_pages() {
        for cp in [1252, 1254] {
            // Tam genişlikte tırnak ve tire best-fit ile `"` ve `-` olur: argv enjeksiyonu.
            assert!(!ensure_roundtrip_cp(r"C:\x＂ -h evil.example ＂.dump", cp), "{cp}");
            assert!(!ensure_roundtrip_cp(r"C:\a\－－clean.dump", cp), "{cp}");
            // Kod sayfasında olmayan karakter (varsayılan karakter `?`).
            assert!(!ensure_roundtrip_cp(r"C:\日本\a.dump", cp), "{cp}");
            assert!(ensure_roundtrip_cp(r"C:\Yedek Klasörü\a.dump", cp), "{cp}");
        }
        // Türkçe harfler 1254'te birebir, 1252'de (ğ, ş, ı yok) değil.
        assert!(ensure_roundtrip_cp(r"C:\ğüşıöç\ĞÜŞİÖÇ.dump", 1254));
        assert!(!ensure_roundtrip_cp(r"C:\ğüşıöç.dump", 1252));
        // UTF-8 kod sayfası her metni taşır.
        assert!(ensure_roundtrip_cp(r"C:\x＂.dump", 65001));
    }

    #[cfg(windows)]
    #[test]
    fn system_code_page_matches_explicit_check() {
        // Sonuç sistem ACP'sine bağlıdır; yardımcı aynı kod sayfasıyla aynı yanıtı vermeli.
        let cp = unsafe { windows_sys::Win32::Globalization::GetACP() };
        for s in [r"C:\x＂.dump", r"C:\ğüşıöç.dump", r"C:\Yedek Klasörü\a.dump"] {
            assert_eq!(ensure_ansi_roundtrip(s), ensure_roundtrip_cp(s, cp), "{cp} {s}");
        }
        if cp != 65001 {
            assert!(!ensure_ansi_roundtrip("＂"));
            assert!(!ensure_ansi_roundtrip("－"));
        }
        if cp == 1254 {
            assert!(ensure_ansi_roundtrip("ğüşıöç"));
        }
    }
}
