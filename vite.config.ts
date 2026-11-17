import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import tauriConf from "./src-tauri/tauri.conf.json" with { type: "json" };

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  // Durum çubuğundaki sürüm: kurulum paketinin sürümü (tauri.conf.json) derleme anında gömülür; IPC gerekmez.
  define: { __APP_VERSION__: JSON.stringify(tauriConf.version) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
  build: {
    target: "chrome120",
    modulePreload: { polyfill: false },
  },
});
