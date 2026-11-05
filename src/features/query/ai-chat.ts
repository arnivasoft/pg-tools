// Asistan sohbet durumu (saf). Sekme başına bir sohbet; arka uç geçmişi yalnız başarılı turlarla büyür,
// `committed` arayüzün bildiği tamamlanmış tur sayısıdır (arka uca `turns` olarak gider).
import type { AiEvent } from "../../ipc/bindings/AiEvent";
import type { AiMode } from "../../ipc/bindings/AiMode";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";

export type Finish = { stopReason: string; inputTokens: number; cacheReadTokens: number; outputTokens: number };
export type ToolStep = { name: string; detail: string };

export type UserTurn = { role: "user"; mode: AiMode; text: string; sql: string };
export type AssistantTurn = {
  role: "assistant";
  text: string;
  finish: Finish | null;
  error: ErrorPayload | null;
  /** Yanıt sırasında sunucu yedek modele geçti. */
  fallback: boolean;
  /** Asistanın çalıştırdığı katalog araçları (sırayla). */
  tools: ToolStep[];
  /** Tur arka uç geçmişine girdi (sonraki sorular bu yanıtı bilir). */
  committed: boolean;
};
export type ChatTurn = UserTurn | AssistantTurn;

export type ChatStatus = "idle" | "gathering" | "streaming";

export interface ChatState {
  conversation: string;
  turns: ChatTurn[];
  committed: number;
  requestId: string | null;
  status: ChatStatus;
}

export function emptyChat(conversation: string): ChatState {
  return { conversation, turns: [], committed: 0, requestId: null, status: "idle" };
}

export function newAssistant(): AssistantTurn {
  return { role: "assistant", text: "", finish: null, error: null, fallback: false, tools: [], committed: false };
}

/** Akış olayını son asistan turuna uygular (yeni nesne döner). */
export function applyEvent(turn: AssistantTurn, ev: AiEvent): AssistantTurn {
  switch (ev.type) {
    case "delta":
      return { ...turn, text: turn.text + ev.text };
    case "fallback":
      return { ...turn, fallback: true };
    case "tool":
      return { ...turn, tools: [...turn.tools, { name: ev.name, detail: ev.detail }] };
    case "done":
      return {
        ...turn,
        finish: {
          stopReason: ev.stopReason,
          inputTokens: ev.inputTokens,
          cacheReadTokens: ev.cacheReadTokens,
          outputTokens: ev.outputTokens,
        },
      };
  }
}

/** Arka uç turu geçmişe ekledi mi: istek başarıyla bitti, yanıt var ve ret değil. */
export function isCommitted(turn: AssistantTurn): boolean {
  return turn.error === null && turn.finish !== null && turn.finish.stopReason !== "refusal" && turn.text.trim() !== "";
}
