import { render } from "solid-js/web";
import "./app/styles.css";
import App from "./app/App";
import { applyTheme, loadThemePreference } from "./app/theme";
import { commands } from "./ipc/commands";

applyTheme(loadThemePreference());
render(() => <App />, document.getElementById("root")!);
// Ana pencere gizli açılır (beyaz kare olmasın): tema ve ilk DOM hazır, şimdi tema rengiyle göster.
// Tauri dışında (düz tarayıcı önizlemesi) komut başarısız olur; yok sayılır.
void commands.appReady(document.documentElement.dataset.theme === "dark").catch(() => {});
