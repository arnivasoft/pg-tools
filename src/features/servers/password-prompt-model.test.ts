import { describe, expect, it, vi } from "vitest";
vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { createPasswordPromptModel } from "./password-prompt-model";

const node = (label: string) => ({ label }) as unknown as TreeNode;

describe("createPasswordPromptModel (tembel yükleme)", () => {
  it("yükleme hatasında istek iptal (null) çözülür", async () => {
    const m = createPasswordPromptModel(() => Promise.resolve(false));
    expect(await m.ask(node("a"), false)).toBeNull();
    expect(m.request()).toBeNull();
  });

  it("ardışık iki istek: ilki iptal, ikincisi yanıtlanır; alanlar sıfırlanır", async () => {
    const m = createPasswordPromptModel(() => Promise.resolve(true));
    const a = m.ask(node("a"), false);
    m.setPassword("x");
    const b = m.ask(node("b"), true);
    expect(await a).toBeNull();
    expect(m.password()).toBe("");
    m.finish({ password: "pw", remember: true });
    expect(await b).toEqual({ password: "pw", remember: true });
    expect(m.request()).toBeNull();
  });

  it("Esc/iptal null verir", async () => {
    const m = createPasswordPromptModel(() => Promise.resolve(true));
    const p = m.ask(node("a"), false);
    await Promise.resolve();
    m.finish(null);
    expect(await p).toBeNull();
  });
});
