import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createCompletionSource, contextAt, matchItems, replaceRange, type CompletionCtxLike } from "./completion";
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";

describe("contextAt", () => {
  const cases: [string, unknown][] = [
    ["select * from app.", { kind: "schemaMember", schema: "app" }],
    ["select * from app.or", { kind: "schemaMember", schema: "app" }],
    ["select o.", { kind: "relationMember", relation: "o" }],
    ['select * from "Order Items".', { kind: "schemaMember", schema: "Order Items" }],
    ['select "Order Items".', { kind: "relationMember", relation: "Order Items" }],
    ["select * from app.orders.", { kind: "relationMember", relation: "orders", schema: "app" }],
    ['select * from "My ""S"""."T".', { kind: "relationMember", relation: "T", schema: 'My "S"' }],
    ["-- from x.", { kind: "none" }],
    ["select 1 -- c\nfrom ", { kind: "relation" }],
    ["'from x.'", { kind: "keyword" }],
    ["select 'from x.", { kind: "none" }],
    ["select 'a' || 'it''s from x.", { kind: "none" }],
    ["/* from x.", { kind: "none" }],
    ["/* /* a */ from x.", { kind: "none" }],
    ["/* a */ select * from ", { kind: "relation" }],
    ["select $$ from x.", { kind: "none" }],
    ["select $q$ x $q$ from ", { kind: "relation" }],
    ["select * from ", { kind: "relation" }],
    ["select * from ord", { kind: "relation" }],
    ["join ", { kind: "relation" }],
    ["update ", { kind: "relation" }],
    ["insert into ", { kind: "relation" }],
    ["select * from orders o", { kind: "keyword" }],
    ["select * from orders where ", { kind: "keyword" }],
    ["select fro", { kind: "keyword" }],
    ["", { kind: "keyword" }],
    ["select * from müşteri.", { kind: "schemaMember", schema: "müşteri" }],
    ["select ğ.", { kind: "relationMember", relation: "ğ" }],
    ["select * from İstanbul.sipariş.", { kind: "relationMember", relation: "sipariş", schema: "İstanbul" }],
    ['select * from "Ünite', { kind: "relation" }],
    ["select $1 from ", { kind: "relation" }],
    ["select * from a, ", { kind: "relation" }],
    ["select * from a x, b y, ", { kind: "relation" }],
    ["select * from a, app.", { kind: "schemaMember", schema: "app" }],
    ["select * from a where b, ", { kind: "keyword" }],
    ["select a, ", { kind: "keyword" }],
    ["truncate ", { kind: "relation" }],
    ["truncate app.", { kind: "schemaMember", schema: "app" }],
    ["alter table ", { kind: "relation" }],
    ["drop table app.", { kind: "schemaMember", schema: "app" }],
  ];
  for (const [input, expected] of cases) {
    it(JSON.stringify(input), () => expect(contextAt(input)).toEqual(expected));
  }
});

describe("replaceRange / matchItems", () => {
  it("açık tırnakla başlayan kısmi ad, imleçten sonraki otomatik kapanış tırnağını kapsar", () => {
    expect(replaceRange(10, '"Ord', '"')).toEqual({ from: 6, to: 11 });
    expect(replaceRange(10, '"Ord', "")).toEqual({ from: 6, to: 10 });
    expect(replaceRange(10, "ord", '"')).toEqual({ from: 7, to: 10 });
  });
  it('`"ord` tırnaksız etiketle (orders) ve tırnaklı adla eşleşir; önek önce gelir', () => {
    const mk = (label: string, quoted = label): CompletionItem => ({ label, quoted, kind: "table", detail: null });
    const items = [mk("my_order"), mk("orders"), mk("Order Items", '"Order Items"'), mk("zzz")];
    expect(matchItems(items, '"ord').map((i) => i.label)).toEqual(["orders", "Order Items", "my_order"]);
    expect(matchItems(items, "")).toHaveLength(4);
  });
});

function ctx(text: string): CompletionCtxLike & { abort(): void } {
  let aborted = false;
  const ls: (() => void)[] = [];
  return {
    pos: text.length,
    explicit: false,
    get aborted() {
      return aborted;
    },
    state: { doc: { sliceString: (f: number, t?: number) => text.slice(f, t) } },
    addEventListener: (_t, l) => {
      ls.push(l);
    },
    abort() {
      aborted = true;
      ls.forEach((l) => l());
    },
  };
}

const item = (quoted: string, kind: CompletionItem["kind"] = "table"): CompletionItem => ({
  label: quoted,
  quoted,
  kind,
  detail: null,
});
const KW = () => ({ from: 0, options: [{ label: "SELECT" }] });

describe("createCompletionSource", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const run = (src: ReturnType<typeof createCompletionSource>, c: CompletionCtxLike) =>
    src(c as never) as Promise<{ from: number; options: { label: string }[] } | null>;

  it("şema üyesi: 150 ms debounce sonrası relations ister, quoted ekler", async () => {
    const fetch = vi.fn(async () => [item('"Order Items"')]);
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const p = run(src, ctx("select * from app.or"));
    await vi.advanceTimersByTimeAsync(149);
    expect(fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    const r = await p;
    expect(fetch).toHaveBeenCalledWith({ type: "relations", schema: "app" });
    expect(r!.from).toBe("select * from app.".length);
    expect(r!.options.map((o) => o.label)).toEqual(['"Order Items"']);
  });

  it("ilişki üyesi kolon ister; şema yoksa null", async () => {
    const fetch = vi.fn(async () => [item("id", "column")]);
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const p = run(src, ctx("select o."));
    await vi.advanceTimersByTimeAsync(150);
    await p;
    expect(fetch).toHaveBeenCalledWith({ type: "columns", schema: null, relation: "o" });
  });

  it("meşgulken istek yok; ilişki bağlamında anahtar kelimeler, üyede null", async () => {
    const fetch = vi.fn(async () => [item("x")]);
    const src = createCompletionSource({ fetch, isBusy: () => true, keywords: KW });
    expect(await run(src, ctx("select * from "))).toEqual(KW());
    expect(await run(src, ctx("select * from app."))).toBeNull();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("bekleme sırasında meşgul olursa istek atılmaz", async () => {
    let busy = false;
    const fetch = vi.fn(async () => [item("x")]);
    const src = createCompletionSource({ fetch, isBusy: () => busy, keywords: KW });
    const p = run(src, ctx("select * from "));
    busy = true;
    await vi.advanceTimersByTimeAsync(150);
    expect(await p).toEqual(KW());
    expect(fetch).not.toHaveBeenCalled();
  });

  it("debounce sırasında iptal: istek yok, sonuç yok", async () => {
    const fetch = vi.fn(async () => [item("x")]);
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const c = ctx("select * from app.");
    const p = run(src, c);
    c.abort();
    await vi.advanceTimersByTimeAsync(150);
    expect(await p).toBeNull();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("istek sürerken iptal: sonuç yok sayılır", async () => {
    let resolve!: (v: CompletionItem[]) => void;
    const fetch = vi.fn(
      () =>
        new Promise<CompletionItem[]>((r) => {
          resolve = r;
        }),
    );
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const c = ctx("select * from app.");
    const p = run(src, c);
    await vi.advanceTimersByTimeAsync(150);
    c.abort();
    resolve([item("x")]);
    expect(await p).toBeNull();
  });

  it("boş sonuç ilişki bağlamında anahtar kelimelere düşer; hata da boş sayılır", async () => {
    const empty = vi.fn(async () => []);
    const src = createCompletionSource({ fetch: empty, isBusy: () => false, keywords: KW });
    const p = run(src, ctx("select * from "));
    await vi.advanceTimersByTimeAsync(150);
    expect(await p).toEqual(KW());
    const bad = vi.fn(async () => {
      throw new Error("x");
    });
    const src2 = createCompletionSource({ fetch: bad, isBusy: () => false, keywords: KW });
    const p2 = run(src2, ctx("select * from "));
    await vi.advanceTimersByTimeAsync(150);
    expect(await p2).toEqual(KW());
  });

  it("yorum/dize içinde ve keyword bağlamında istek yok", async () => {
    const fetch = vi.fn(async () => [item("x")]);
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    expect(await run(src, ctx("-- from x."))).toBeNull();
    expect(await run(src, ctx("select "))).toEqual(KW());
    expect(fetch).not.toHaveBeenCalled();
  });

  it("kaynak: kabul aralığı kapanış tırnağını kapsar, süzme etikete göre", async () => {
    const fetch = vi.fn(async () => [{ label: "orders", quoted: "orders", kind: "table" as const, detail: null }]);
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const text = 'select * from "ord"';
    const c = ctx(text);
    (c as { pos: number }).pos = text.length - 1;
    const p = src(c as never) as Promise<{ from: number; to: number; options: unknown[] }>;
    await vi.advanceTimersByTimeAsync(150);
    const r = await p;
    expect(r.from).toBe(text.length - 5);
    expect(r.to).toBe(text.length);
    expect(r.options).toHaveLength(1);
  });

  it("select app.: kolon boşsa şema olarak ilişkileri ister", async () => {
    const fetch = vi.fn(async (req: { type: string }) => (req.type === "columns" ? [] : [item("orders")]));
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const p = run(src, ctx("select app."));
    await vi.advanceTimersByTimeAsync(150);
    const r = await p;
    expect(fetch).toHaveBeenNthCalledWith(2, { type: "relations", schema: "app" });
    expect(r!.options).toHaveLength(1);
  });

  it("ilişki bağlamı: istekler art arda; ilk istekten sonra iptal edilirse ikincisi atılmaz", async () => {
    const c = ctx("select * from ");
    const fetch = vi.fn(async () => {
      c.abort();
      return [item("orders")];
    });
    const src = createCompletionSource({ fetch, isBusy: () => false, keywords: KW });
    const p = run(src, c);
    await vi.advanceTimersByTimeAsync(150);
    expect(await p).toBeNull();
    expect(fetch).toHaveBeenCalledTimes(1);
  });
});
