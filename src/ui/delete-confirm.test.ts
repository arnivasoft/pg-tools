import { describe, expect, it } from "vitest";
import { vi } from "vitest";
vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));
import { createDeleteConfirmModel } from "./delete-confirm";

describe("createDeleteConfirmModel (tembel yükleme)", () => {
  it("yükleme hatasında istek iptal (false) çözülür ve açılmaz", async () => {
    const m = createDeleteConfirmModel(() => Promise.resolve(false));
    expect(await m.ask("t1")).toBe(false);
    expect(m.request()).toBeNull();
    expect(m.loaded()).toBe(false);
  });

  it("ardışık iki istek: ilki ikincisiyle iptal edilir, ikincisi yanıt bekler", async () => {
    const m = createDeleteConfirmModel(() => Promise.resolve(true));
    const a = m.ask("a");
    const b = m.ask("b");
    expect(await a).toBe(false);
    expect(m.request()?.name).toBe("b");
    m.finish(true);
    expect(await b).toBe(true);
    expect(m.request()).toBeNull();
  });

  it("görünüm yüklenmeden/istek yokken finish etkisiz; Esc (false) iptal sonucu verir", async () => {
    const m = createDeleteConfirmModel(() => Promise.resolve(true));
    m.finish(false);
    const p = m.ask("x");
    expect(await Promise.race([p, Promise.resolve("bekliyor")])).toBe("bekliyor");
    m.finish(false);
    expect(await p).toBe(false);
    expect(m.loaded()).toBe(true);
  });
});
