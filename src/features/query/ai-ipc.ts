// Yapay zekâ asistanı IPC sarıcıları. Yalnız tembel yüklenen AiPane içe aktarır (ana pakete girmez).
// Anahtar yalnız arka uçta (Kimlik Bilgisi Yöneticisi); `id` ve `conversation` ön yüzde üretilir.
// Hata kodları: ai_no_key | ai_auth | ai_rate | ai_stale | ai_long | ai | network | invalid | busy.
import { Channel } from "@tauri-apps/api/core";
import type { AiEvent } from "../../ipc/bindings/AiEvent";
import type { AiRequest } from "../../ipc/bindings/AiRequest";
import { call } from "../../ipc/commands";

export const aiIpc = {
  keyStatus: () => call<boolean>("ai_key_status"),
  keySet: (key: string) => call<null>("ai_key_set", { key }),
  keyClear: () => call<null>("ai_key_clear"),
  /**
   * `turns`: arayüzün bildiği tamamlanmış tur sayısı (arka uç geçmişiyle uyuşmazsa `ai_stale`).
   * `target`: sekmenin veritabanı; asistanın katalog araçları burada (katalog bağlantı havuzunda) koşar.
   */
  ask: (
    id: string,
    conversation: string,
    turns: number,
    request: AiRequest,
    target: { serverId: string | null; databaseOid: number | null },
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
      onEvent: ch,
    });
  },
  cancel: (id: string) => call<null>("ai_cancel", { id }),
  forget: (conversation: string) => call<null>("ai_forget", { conversation }),
};
