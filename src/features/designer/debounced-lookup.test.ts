// Anahtarlı debounce'lu lookup (FK hedef tablo/kolon önerileri): debounce, aynı anahtar tekrar sorgulanmaz, son istek
// kazanır, anahtar değişince liste boşalır, null → temizle, hata → boş liste ve yeniden deneme. Sinyaller gerçek olsun
// diye solid-js tarayıcı yapısına yönlendirilir.
import { describe, expect, it, vi } from "vitest";
import type { LookupItem } from "../../ipc/bindings/LookupItem";

vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));

function fakeTimers() {
  const pending = new Map<number, { fn: () => void; at: number }>();
  let now = 0;
  let next = 1;
  return {
    deps: {
      setTimer: (fn: () => void, ms: number) => {
        const h = next++;
        pending.set(h, { fn, at: now + ms });
        return h;
      },
      clearTimer: (h: unknown) => void pending.delete(h as number),
    },
    advance(ms: number) {
      now += ms;
      for (const [h, t] of [...pending]) if (t.at <= now) { pending.delete(h); t.fn(); }
    },
  };
}
const flush = () => new Promise((r) => setTimeout(r, 0));
const item = (value: string): LookupItem => ({ value, detail: null });

describe("createDebouncedLookup", () => {
  it("debounce; aynı anahtar yeniden sorgulanmaz; anahtar değişince liste hemen boşalır; null temizler", async () => {
    const solid = await import("solid-js");
    const { createDebouncedLookup, LOOKUP_DELAY_MS } = await import("./debounced-lookup");
    const t = fakeTimers();
    const calls: string[] = [];
    await solid.createRoot(async (dispose) => {
      const s = createDebouncedLookup<string>(async (k) => (calls.push(k), [item(k)]), (k) => k, t.deps);
      s.query("a");
      s.query("ap");
      t.advance(LOOKUP_DELAY_MS - 1);
      expect(calls).toEqual([]);
      t.advance(1);
      await flush();
      expect(calls).toEqual(["ap"]);
      expect([s.items(), s.busy()]).toEqual([[item("ap")], false]);
      s.query("ap");
      t.advance(LOOKUP_DELAY_MS);
      expect(calls).toEqual(["ap"]);
      expect(s.items()).toEqual([item("ap")]);
      s.query("app");
      expect([s.items(), s.busy()]).toEqual([[], true]);
      t.advance(LOOKUP_DELAY_MS);
      await flush();
      expect(s.items()).toEqual([item("app")]);
      s.query(null);
      expect([s.items(), s.busy()]).toEqual([[], false]);
      t.advance(LOOKUP_DELAY_MS);
      expect(calls).toEqual(["ap", "app"]);
      // null sonrası aynı anahtar yeniden sorgulanır.
      s.query("app");
      t.advance(LOOKUP_DELAY_MS);
      expect(calls).toEqual(["ap", "app", "app"]);
      dispose();
    });
  });

  it("uçuştaki eski yanıt yok sayılır; hata → boş liste ve aynı anahtar yeniden denenir; dispose sonrası değişmez", async () => {
    const solid = await import("solid-js");
    const { createDebouncedLookup, LOOKUP_DELAY_MS } = await import("./debounced-lookup");
    const t = fakeTimers();
    const resolvers = new Map<string, (v: LookupItem[]) => void>();
    const rejecters = new Map<string, (e: unknown) => void>();
    const calls: string[] = [];
    await solid.createRoot(async (dispose) => {
      const s = createDebouncedLookup<{ schema: string; table: string }>(
        (k) => new Promise((res, rej) => { calls.push(k.table); resolvers.set(k.table, res); rejecters.set(k.table, rej); }),
        (k) => JSON.stringify([k.schema, k.table]),
        t.deps,
      );
      s.query({ schema: "public", table: "a" });
      t.advance(LOOKUP_DELAY_MS);
      s.query({ schema: "public", table: "b" });
      t.advance(LOOKUP_DELAY_MS);
      resolvers.get("a")!([item("a1")]);
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], true]);
      rejecters.get("b")!(new Error("x"));
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], false]);
      s.query({ schema: "public", table: "b" });
      t.advance(LOOKUP_DELAY_MS);
      expect(calls).toEqual(["a", "b", "b"]);
      s.dispose();
      resolvers.get("b")!([item("late")]);
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], false]);
      dispose();
    });
  });
});
