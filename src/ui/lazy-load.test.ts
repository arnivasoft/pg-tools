import { describe, expect, it, vi } from "vitest";
import { createLoader } from "./lazy-load";

describe("createLoader", () => {
  it("başarıda true döner ve yüklemeyi bir kez çağırır", async () => {
    const load = vi.fn(() => Promise.resolve({ ok: 1 }));
    const onFail = vi.fn();
    const ensure = createLoader(load, onFail);
    expect(await ensure()).toBe(true);
    expect(await ensure()).toBe(true);
    expect(load).toHaveBeenCalledTimes(1);
    expect(onFail).not.toHaveBeenCalled();
  });

  it("reddedilirse false döner, onFail çağrılır, sonraki istek yeniden dener", async () => {
    const err = new Error("ağ");
    const load = vi.fn().mockRejectedValueOnce(err).mockResolvedValueOnce({});
    const onFail = vi.fn();
    const ensure = createLoader(load, onFail);
    expect(await ensure()).toBe(false);
    expect(onFail).toHaveBeenCalledWith(err);
    expect(await ensure()).toBe(true);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("eşzamanlı istekler aynı sırayla çözülür", async () => {
    let release: (v: object) => void = () => {};
    const load = () => new Promise<object>((r) => (release = r));
    const ensure = createLoader(load, vi.fn());
    const order: number[] = [];
    const a = ensure().then(() => order.push(1));
    const b = ensure().then(() => order.push(2));
    release({});
    await Promise.all([a, b]);
    expect(order).toEqual([1, 2]);
  });
  it("eşzamanlı iki istek, tek ret: onFail bir kez; sonraki istek yeniden dener ve başarır", async () => {
    let fail = true;
    const load = vi.fn(async () => {
      if (fail) throw new Error("x");
      return "mod";
    });
    const onFail = vi.fn();
    const ensure = createLoader(load, onFail);
    expect(await Promise.all([ensure(), ensure()])).toEqual([false, false]);
    expect(onFail).toHaveBeenCalledTimes(1);
    expect(load).toHaveBeenCalledTimes(1);
    fail = false;
    expect(await ensure()).toBe(true);
    expect(load).toHaveBeenCalledTimes(2);
  });

  // error-badge-model createCardLoader'dan taşındı (tek yardımcıda birleştirildi): modül yükleyicinin içinde işlenir.
  it("eşzamanlı istekler tek yüklemeyi paylaşır, başarıda bir kez bildirir", async () => {
    const load = vi.fn(async () => "mod");
    const loaded = vi.fn();
    const ensure = createLoader(() => load().then(loaded), vi.fn());
    expect(await Promise.all([ensure(), ensure(), ensure()])).toEqual([true, true, true]);
    await ensure();
    expect(load).toHaveBeenCalledTimes(1);
    expect(loaded).toHaveBeenCalledTimes(1);
    expect(loaded).toHaveBeenCalledWith("mod");
  });

  it("ret: onFail çağıran sayısından bağımsız bir kez; sonraki istek yeniden dener", async () => {
    let fail = true;
    const load = vi.fn(async () => {
      if (fail) throw new Error("x");
      return "mod";
    });
    const onFail = vi.fn();
    const loaded = vi.fn();
    const ensure = createLoader(() => load().then(loaded), onFail);
    expect(await Promise.all([ensure(), ensure(), ensure()])).toEqual([false, false, false]);
    expect(onFail).toHaveBeenCalledTimes(1);
    expect(loaded).not.toHaveBeenCalled();
    fail = false;
    expect(await ensure()).toBe(true);
    expect(loaded).toHaveBeenCalledWith("mod");
  });

  it("load() eşzamanlı fırlatırsa ret gibi işlenir: bir onFail, ensure fırlatmaz, sonraki istek yeniden dener", async () => {
    let fail = true;
    const load = vi.fn((): Promise<string> => {
      if (fail) throw new Error("eşzamanlı");
      return Promise.resolve("mod");
    });
    const onFail = vi.fn();
    const ensure = createLoader(load, onFail);
    expect(await ensure()).toBe(false);
    expect(onFail).toHaveBeenCalledTimes(1);
    fail = false;
    expect(await ensure()).toBe(true);
  });
});
