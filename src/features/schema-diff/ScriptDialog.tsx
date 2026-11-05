// Betik önizleme diyaloğu (tembel parça). Geçici yer tutucu: Task 4 diyaloğun gerçeğini yazar. Şimdilik yalnızca
// denetleyicinin kaydetme seçicisini (`picker`) verir; bileşen hiçbir şey çizmez.
import { save } from "@tauri-apps/plugin-dialog";
import type { Picker } from "./diff-controller";

export const picker: Picker = {
  save: async (o) => {
    const path: unknown = await save(o);
    return typeof path === "string" ? path : null;
  },
};

export default function ScriptDialog() {
  return null;
}
