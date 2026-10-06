//! Açılışta beyaz kare (white flash) önleme.
//!
//! Ana pencere gizli oluşturulur (`tauri.conf.json` → `"visible": false`). Ön yüz temayı uygulayıp ilk çizimi
//! kurduktan sonra `app_ready` komutunu çağırır: pencere ve WebView2 arka planı temanın `--bg` rengine
//! ayarlanır, sonra pencere gösterilir. Böylece pencere hiçbir zaman WebView2'nin varsayılan beyaz arka planıyla
//! görünmez. Ön yüz hiç çağırmazsa (betik hatası vb.) yedek zamanlayıcı pencereyi yine de gösterir.

use std::time::Duration;

use tauri::window::Color;
use tauri::{Manager, Runtime, WebviewWindow};

/// Ön yüz `app_ready` çağırmazsa pencerenin gösterileceği süre.
const FALLBACK_SHOW: Duration = Duration::from_secs(3);

/// `src/app/styles.css` içindeki `--bg` değerleri (açık / koyu); test eşleşmeyi denetler.
const LIGHT_BG: Color = Color(0xfb, 0xfb, 0xfc, 0xff);
const DARK_BG: Color = Color(0x16, 0x18, 0x1d, 0xff);

fn background(dark: bool) -> Color {
    if dark { DARK_BG } else { LIGHT_BG }
}

/// Pencere ve WebView arka planını temaya ayarlar ve (gizliyse) gösterir.
fn reveal<R: Runtime>(window: &WebviewWindow<R>, dark: bool) {
    if let Err(e) = window.set_background_color(Some(background(dark))) {
        tracing::warn!(error = %e, "pencere arka planı ayarlanamadı");
    }
    if !window.is_visible().unwrap_or(false)
        && let Err(e) = window.show()
    {
        tracing::warn!(error = %e, "pencere gösterilemedi");
    }
}

/// `setup` içinde çağrılır: gizli pencerenin arka planını sistem temasına göre önceden ayarlar ve yedek
/// gösterme zamanlayıcısını kurar.
pub fn init<R: Runtime>(app: &tauri::App<R>) {
    let Some(window) = app.get_webview_window("main") else { return };
    let dark = matches!(window.theme(), Ok(tauri::Theme::Dark));
    if let Err(e) = window.set_background_color(Some(background(dark))) {
        tracing::warn!(error = %e, "pencere arka planı ayarlanamadı");
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FALLBACK_SHOW).await;
        if !window.is_visible().unwrap_or(true) {
            // smoke-release.ps1 logda "fallback-show" arar: ön yüz pencereyi göstermediyse başarısız sayılır.
            tracing::warn!("fallback-show: pencere yedek zamanlayıcıyla gösterildi (ön yüz app_ready çağırmadı)");
            reveal(&window, dark);
        }
    });
}

/// Ön yüz ilk çizimden hemen sonra çağırır. Yenilemede (pencere zaten görünür) yalnızca arka planı günceller.
#[tauri::command]
pub fn app_ready(window: WebviewWindow, dark: bool) {
    reveal(&window, dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color) -> String {
        format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
    }

    #[test]
    fn backgrounds_match_theme_css() {
        let css = include_str!("../../src/app/styles.css");
        let (light, dark) = css.split_once("
[data-theme=\"dark\"] {").expect("koyu tema bloğu");
        assert!(light.contains(&format!("--bg: {};", hex(LIGHT_BG))), "açık --bg değişti");
        assert!(dark.contains(&format!("--bg: {};", hex(DARK_BG))), "koyu --bg değişti");
    }
}
