// Yapay zekâ asistanı IPC sarıcıları. Yalnız tembel yüklenen AiPane içe aktarır (ana pakete girmez).
// Anahtar yalnız arka uçta (Kimlik Bilgisi Yöneticisi); `id` ön yüzde üretilir.
// Hata kodları: ai_no_key | ai_auth | ai_rate | ai | network | invalid | busy.
import { Channel } from "@tauri-apps/api/core";
import type { AiEvent } from "../../ipc/bindings/AiEvent";
import type { AiRequest } from "../../ipc/bindings/AiRequest";
import { call } from "../../ipc/commands";

export const aiIpc = {
  keyStatus: () => call<boolean>("ai_key_status"),
  keySet: (key: string) => call<null>("ai_key_set", { key }),
  keyClear: () => call<null>("ai_key_clear"),
  ask: (id: string, request: AiRequest, onEvent: (e: AiEvent) => void) => {
    const ch = new Channel<AiEvent>();
    ch.onmessage = onEvent;
    return call<null>("ai_ask", { id, request, onEvent: ch });
  },
  cancel: (id: string) => call<null>("ai_cancel", { id }),
};
