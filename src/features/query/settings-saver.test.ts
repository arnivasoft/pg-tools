import { describe, expect, it, vi } from "vitest";
import { createSerialSaver } from "./settings-saver";

type S = { check: boolean; indent: number };

function setup(save: (v: S) => Promise<unknown>) {
  let cur: S = { check: true, indent: 2 };
  const failed = vi.fn();
  const saver = createSerialSaver<S>({ get: () => cur, set: (v) => (cur = v), save, failed });
  return { saver, get: () => cur, failed };
}

describe("createSerialSaver", () => {
  it("değer hemen uygulanır; art arda iki değişiklik birbirini ezmez, yazımlar sırayla", async () => {
    const order: S[] = [];
    let release!: () => void;
    const first = new Promise<void>((r) => (release = r));
    const save = vi.fn(async (v: S) => {
      if (order.length === 0) await first;
      order.push(v);
    });
    const { saver, get } = setup(save);
    const a = saver.save({ ...get(), check: false });
    expect(get()).toEqual({ check: false, indent: 2 });
    const b = saver.save({ ...get(), indent: 4 }); // ilki bitmeden; önceki değişiklik korunur
    expect(get()).toEqual({ check: false, indent: 4 });
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(save).toHaveBeenCalledTimes(1); // ikinci yazım ilki bitene kadar bekler
    release();
    await Promise.all([a, b]);
    expect(order).toEqual([
      { check: false, indent: 2 },
      { check: false, indent: 4 },
    ]);
  });

  it("başarısız yazım önceki değere döner ve bildirir", async () => {
    const { saver, get, failed } = setup(async () => {
      throw new Error("x");
    });
    await saver.save({ check: false, indent: 2 });
    expect(get()).toEqual({ check: true, indent: 2 });
    expect(failed).toHaveBeenCalledTimes(1);
  });

  it("başarısız yazımdan sonra daha yeni değişiklik varsa geri alınmaz", async () => {
    let n = 0;
    const { saver, get } = setup(async () => {
      if (n++ === 0) throw new Error("x");
    });
    const a = saver.save({ check: false, indent: 2 });
    const b = saver.save({ check: false, indent: 4 });
    await Promise.all([a, b]);
    expect(get()).toEqual({ check: false, indent: 4 });
  });
});
