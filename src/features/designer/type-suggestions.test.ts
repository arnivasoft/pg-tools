// Tip önerileri: 200 ms debounce, son istek kazanır, hata → boş liste, busy bayrağı. Sinyaller gerçek olsun diye solid-js
// tarayıcı yapısına yönlendirilir.
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

describe("createTypeSuggestions", () => {
  it("200 ms debounce; yalnızca son sorgu gönderilir ve sorgu typeQueryOf'tur", async () => {
    const solid = await import("solid-js");
    const { createTypeSuggestions, TYPE_QUERY_DELAY_MS } = await import("./type-suggestions");
    const t = fakeTimers();
    const calls: Array<string | null> = [];
    await solid.createRoot(async (dispose) => {
      const s = createTypeSuggestions(async (q) => (calls.push(q), [item(`${q}`)]), t.deps);
      s.query("num");
      s.query("numeric(10");
      expect(s.busy()).toBe(true);
      t.advance(TYPE_QUERY_DELAY_MS - 1);
      expect(calls).toEqual([]);
      t.advance(1);
      expect(calls).toEqual(["numeric"]);
      await flush();
      expect([s.items(), s.busy()]).toEqual([[item("numeric")], false]);
      s.query("  ");
      t.advance(TYPE_QUERY_DELAY_MS);
      expect(calls).toEqual(["numeric", null]);
      dispose();
    });
  });

  it("uçuştaki eski yanıt yeni sorgudan sonra gelirse yok sayılır; hata → boş liste", async () => {
    const solid = await import("solid-js");
    const { createTypeSuggestions, TYPE_QUERY_DELAY_MS } = await import("./type-suggestions");
    const t = fakeTimers();
    const resolvers = new Map<string | null, (v: LookupItem[]) => void>();
    const rejecters = new Map<string | null, (e: unknown) => void>();
    await solid.createRoot(async (dispose) => {
      const s = createTypeSuggestions((q) => new Promise((res, rej) => { resolvers.set(q, res); rejecters.set(q, rej); }), t.deps);
      s.query("int");
      t.advance(TYPE_QUERY_DELAY_MS);
      s.query("text");
      t.advance(TYPE_QUERY_DELAY_MS);
      resolvers.get("int")!([item("int4")]);
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], true]);
      resolvers.get("text")!([item("text")]);
      await flush();
      expect(s.items()).toEqual([item("text")]);
      s.query("bad");
      t.advance(TYPE_QUERY_DELAY_MS);
      rejecters.get("bad")!(new Error("x"));
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], false]);
      // dispose sonrası yanıt durumu değiştirmez.
      s.query("late");
      t.advance(TYPE_QUERY_DELAY_MS);
      s.dispose();
      resolvers.get("late")!([item("late")]);
      await flush();
      expect([s.items(), s.busy()]).toEqual([[], false]);
      dispose();
    });
  });

  it("m-2: sorgu metni değişmezse yeniden sorgulanmaz (busy titremez); hata sonrası aynı sorgu yeniden denenir", async () => {
    const solid = await import("solid-js");
    const { createTypeSuggestions, TYPE_QUERY_DELAY_MS } = await import("./type-suggestions");
    const t = fakeTimers();
    const calls: Array<string | null> = [];
    let fail = false;
    await solid.createRoot(async (dispose) => {
      const s = createTypeSuggestions(async (q) => {
        calls.push(q);
        if (fail) throw new Error("x");
        return [item(`${q}`)];
      }, t.deps);
      s.query("numeric(");
      t.advance(TYPE_QUERY_DELAY_MS);
      await flush();
      expect(s.busy()).toBe(false);
      s.query("numeric(1");
      s.query("numeric(10,2)");
      expect(s.busy()).toBe(false);
      t.advance(TYPE_QUERY_DELAY_MS);
      expect(calls).toEqual(["numeric"]);
      fail = true;
      s.query("text");
      t.advance(TYPE_QUERY_DELAY_MS);
      await flush();
      fail = false;
      s.query("text");
      t.advance(TYPE_QUERY_DELAY_MS);
      await flush();
      expect(calls).toEqual(["numeric", "text", "text"]);
      expect(s.items()).toEqual([item("text")]);
      dispose();
    });
  });
});
