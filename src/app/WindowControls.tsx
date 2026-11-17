import { createSignal, onCleanup } from "solid-js";
import { commands } from "../ipc/commands";
import { tw } from "./strings-window";

/**
 * Özel başlık çubuğunun pencere düğmeleri (`tauri.conf.json` → `"decorations": false`): çerçeve temayla uyumlu.
 * Simgeler Windows'un kendi simge yazı tipinden gelir (Windows 11: Segoe Fluent Icons, 10: Segoe MDL2 Assets).
 * Sürükleme ve çift tıklama `App.tsx`'teki başlık çubuğu işleyicisindedir.
 */
const base = "inline-flex h-full w-[46px] items-center justify-center text-[10px] [font-family:'Segoe_Fluent_Icons','Segoe_MDL2_Assets']";

/** Pencere çalışma alanını kaplıyor mu: büyütülmüş say (düğme, çift tıklama, Win+↑, sürükleyerek geri alma). */
const isMaximized = () => window.outerWidth >= screen.availWidth && window.outerHeight >= screen.availHeight;

export default function WindowControls() {
  const [maximized, setMaximized] = createSignal(isMaximized());
  const sync = () => setMaximized(isMaximized());
  window.addEventListener("resize", sync);
  onCleanup(() => window.removeEventListener("resize", sync));
  const win = (action: "min" | "max" | "close") =>
    void commands.win(action).then(setMaximized, () => {});
  const max = () => (maximized() ? tw.restore : tw.maximize);
  return (
    <div class="-mr-2 ml-1 flex h-full shrink-0">
      <button type="button" class={`${base} hover:bg-hover`} title={tw.minimize} aria-label={tw.minimize} onClick={() => win("min")}>
        {""}
      </button>
      <button type="button" class={`${base} hover:bg-hover`} title={max()} aria-label={max()} onClick={() => win("max")}>
        {maximized() ? "" : ""}
      </button>
      <button type="button" class={`${base} hover:bg-[#c42b1c] hover:text-white`} title={tw.close} aria-label={tw.close} onClick={() => win("close")}>
        {""}
      </button>
    </div>
  );
}
