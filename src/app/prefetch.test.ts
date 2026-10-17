import { describe, expect, it, vi } from "vitest";
import { prefetchIdle } from "./prefetch";

describe("prefetchIdle", () => {
  it("yükleyiciler zamanlayıcı çalışana dek çağrılmaz, çalışınca hepsi çağrılır", () => {
    const a = vi.fn(() => Promise.resolve());
    const b = vi.fn(() => Promise.resolve());
    let run: () => void = () => {};
    prefetchIdle([a, b], (cb) => (run = cb));
    expect(a).not.toHaveBeenCalled();
    run();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("reddeden yükleyici diğerlerini engellemez ve yakalanmamış ret üretmez", async () => {
    const unhandled = vi.fn();
    const proc = (globalThis as unknown as { process: { on: (e: string, f: () => void) => void; off: (e: string, f: () => void) => void } }).process;
    proc.on("unhandledRejection", unhandled);
    const bad = vi.fn(() => Promise.reject(new Error("ağ")));
    const throws = vi.fn((): Promise<unknown> => {
      throw new Error("eşzamanlı");
    });
    const ok = vi.fn(() => Promise.resolve());
    prefetchIdle([bad, throws, ok], (cb) => cb());
    await new Promise((r) => setTimeout(r, 10));
    proc.off("unhandledRejection", unhandled);
    expect(ok).toHaveBeenCalledTimes(1);
    expect(unhandled).not.toHaveBeenCalled();
  });
});
