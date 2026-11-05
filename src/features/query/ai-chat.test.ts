import { describe, expect, it } from "vitest";
import { applyEvent, emptyChat, isCommitted, newAssistant } from "./ai-chat";

describe("applyEvent", () => {
  it("metni biriktirir, yedek modele geçişte metni korur ve bitişi kaydeder", () => {
    let t = newAssistant();
    t = applyEvent(t, { type: "delta", text: "Mer" });
    t = applyEvent(t, { type: "fallback" });
    t = applyEvent(t, { type: "delta", text: "haba" });
    t = applyEvent(t, { type: "done", stopReason: "end_turn", inputTokens: 10, outputTokens: 3 });
    expect(t.text).toBe("Merhaba");
    expect(t.fallback).toBe(true);
    expect(t.finish).toEqual({ stopReason: "end_turn", inputTokens: 10, outputTokens: 3 });
  });
});

describe("isCommitted", () => {
  const done = (stopReason: string) =>
    applyEvent(applyEvent(newAssistant(), { type: "delta", text: "x" }), { type: "done", stopReason, inputTokens: 1, outputTokens: 1 });

  it("başarılı ve kesilen yanıt geçmişe girer", () => {
    expect(isCommitted(done("end_turn"))).toBe(true);
    expect(isCommitted(done("max_tokens"))).toBe(true);
  });

  it("ret, hata, iptal (bitiş yok) ve boş yanıt girmez", () => {
    expect(isCommitted(done("refusal"))).toBe(false);
    expect(isCommitted({ ...done("end_turn"), error: { code: "network", message: "x", detail: null, hint: null } })).toBe(false);
    expect(isCommitted(applyEvent(newAssistant(), { type: "delta", text: "yarım" }))).toBe(false);
    expect(isCommitted(applyEvent(newAssistant(), { type: "done", stopReason: "end_turn", inputTokens: 1, outputTokens: 0 }))).toBe(false);
  });
});

describe("emptyChat", () => {
  it("boş ve boşta başlar", () => {
    expect(emptyChat("c")).toEqual({ conversation: "c", turns: [], committed: 0, requestId: null, status: "idle" });
  });
});
