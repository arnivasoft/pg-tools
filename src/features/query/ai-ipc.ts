// Yapay zekâ asistanı IPC sarıcıları. Yalnız tembel yüklenen AiPane içe aktarır (ana pakete girmez).
// Anahtarlar yalnız arka uçta (Kimlik Bilgisi Yöneticisi, sağlayıcı başına); arayüz yalnız "kayıtlı mı" bilgisini alır.
// `id` ve `conversation` ön yüzde üretilir.
// Hata kodları: ai_no_key | ai_auth | ai_billing | ai_rate | ai_stale | ai_long | ai | network | invalid | busy.
import { Channel } from "@tauri-apps/api/core";
import type { AiEvent } from "../../ipc/bindings/AiEvent";
import type { AiProvider } from "../../ipc/bindings/AiProvider";
import type { AiRequest } from "../../ipc/bindings/AiRequest";
import type { AiSettings } from "../../ipc/bindings/AiSettings";
import type { AiSettingsView } from "../../ipc/bindings/AiSettingsView";
import { call } from "../../ipc/commands";

export const aiIpc = {
  settingsGet: () => call<AiSettingsView>("ai_settings_get"),
  settingsSet: (settings: AiSettings) => call<AiSettingsView>("ai_settings_set", { settings }),
  keySet: (provider: AiProvider, key: string) => call<AiSettingsView>("ai_key_set", { provider, key }),
  keyClear: (provider: AiProvider) => call<AiSettingsView>("ai_key_clear", { provider }),
  /**
   * `turns`: arayüzün bildiği tamamlanmış tur sayısı (arka uç geçmişiyle uyuşmazsa `ai_stale`).
   * `target`: sekmenin veritabanı (katalog araçları, katalog bağlantı havuzunda) ve sorgu oturumu (plan aracı).
   * Sağlayıcı/model/effort arka uçtaki ayarlardan alınır.
   */
  ask: (
    id: string,
    conversation: string,
    turns: number,
    request: AiRequest,
    target: { serverId: string | null; databaseOid: number | null; session: string | null },
    onEvent: (e: AiEvent) => void,
  ) => {
    const ch = new Channel<AiEvent>();
    ch.onmessage = onEvent;
    return call<null>("ai_ask", {
      id,
      conversation,
      turns,
      request,
      serverId: target.serverId,
      databaseOid: target.databaseOid,
      session: target.session,
      onEvent: ch,
    });
  },
  cancel: (id: string) => call<null>("ai_cancel", { id }),
  forget: (conversation: string) => call<null>("ai_forget", { conversation }),
};

/** Etkin sağlayıcının anahtarı kayıtlı mı. */
export function activeHasKey(v: AiSettingsView): boolean {
  return v.settings.provider === "anthropic" ? v.hasAnthropicKey : v.hasOpenrouterKey;
}
