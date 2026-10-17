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
});
