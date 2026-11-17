import { describe, expect, it, vi } from "vitest";
import { ts } from "../../app/strings-search";
import type { SearchHit } from "../../ipc/bindings/SearchHit";
import type { SearchResult } from "../../ipc/bindings/SearchResult";
import { ALL_KINDS, createSearch, SEARCH_DELAY_MS, type SearchDeps } from "./search-model";

const hit = (name: string): SearchHit => ({ id: name, path: [], kind: "table", name, schema: "public", owner: null, system: false });
const res = (...names: string[]): SearchResult => ({ hits: names.map(hit), truncated: false });
const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};
const notConn = () => Object.assign(new Error("bağlı değil"), { code: "not_connected", message: "bağlı değil", detail: null, hint: null });
const calls = (d: SearchDeps) => (d.search as unknown as { mock: { calls: unknown[][] } }).mock.calls;

function setup(over: Partial<SearchDeps> = {}) {
  const timers: { fn: () => void; ms: number; live: boolean }[] = [];
  const deps: SearchDeps = {
    search: vi.fn(async () => res("a")),
    connect: vi.fn(async () => true),
    setTimer: (fn, ms) => {
      const t = { fn, ms, live: true };
      timers.push(t);
      return t;
    },
    clearTimer: (h) => {
      (h as { live: boolean }).live = false;
    },
    ...over,
  };
  const fire = () => {
    for (const t of timers) {
      if (t.live) {
        t.live = false;
        t.fn();
      }
    }
  };
  return { deps, timers, fire };
}

// Her test kendi veritabanı oid'ini kullanır (modül düzeyi hafıza testler arası sızmasın).
let n = 0;
const target = () => ({ serverId: "S", databaseOid: ++n });

describe("search-model", () => {
  it("250 ms içinde üç setQuery tek arama yapar", async () => {
    const { deps, fire, timers } = setup();
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.setQuery("ab");
    s.setQuery("abc");
    expect(timers.every((t) => t.ms === SEARCH_DELAY_MS)).toBe(true);
    fire();
    await flush();
    expect(deps.search).toHaveBeenCalledTimes(1);
    expect(calls(deps)[0][2]).toBe("abc");
    expect(s.state().status).toBe("done");
    expect(s.state().active).toBe(0);
  });

  it("searchNow beklemeden arar", async () => {
    const { deps } = setup();
    const s = createSearch(deps, target());
    s.setQuery("x");
    s.searchNow();
    await flush();
    expect(deps.search).toHaveBeenCalledTimes(1);
  });

  it("eski yanıt yenisinden sonra gelirse yok sayılır", async () => {
    const resolvers: ((r: SearchResult) => void)[] = [];
    const { deps } = setup({ search: vi.fn(() => new Promise<SearchResult>((r) => resolvers.push(r))) });
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.searchNow();
    s.setQuery("ab");
    s.searchNow();
    resolvers[1](res("yeni"));
    await flush();
    resolvers[0](res("eski"));
    await flush();
    expect(s.state().hits.map((h) => h.name)).toEqual(["yeni"]);
  });

  it("toggleKind ve setAllKinds", async () => {
    const { deps } = setup();
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.toggleKind("column");
    await flush();
    expect(s.state().kinds).toEqual(["column"]);
    expect(calls(deps).at(-1)![3]).toEqual(["column"]);
    s.toggleKind("table");
    expect(s.state().kinds).toEqual(["table", "column"]);
    s.toggleKind("table");
    s.toggleKind("column");
    expect(s.state().kinds).toEqual([]);
    s.toggleKind("index");
    s.setAllKinds();
    expect(s.state().kinds).toEqual([]);
    expect(ALL_KINDS).toHaveLength(17);
  });

  it("boş sorgu idle, arama yok", async () => {
    const { deps, fire } = setup();
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.searchNow();
    await flush();
    s.setQuery("  ");
    fire();
    await flush();
    expect(s.state().status).toBe("idle");
    expect(s.state().hits).toEqual([]);
    expect(deps.search).toHaveBeenCalledTimes(1);
  });

  it("201 karakter engellenir", () => {
    const { deps } = setup();
    const s = createSearch(deps, target());
    s.setQuery("a".repeat(201));
    s.searchNow();
    expect(s.state().error).toBe(ts.tooLong);
    expect(s.state().status).toBe("error");
    expect(deps.search).not.toHaveBeenCalled();
  });

  it("not_connected: connect sonrası bir kez yeniden dener", async () => {
    const search = vi.fn().mockRejectedValueOnce(notConn()).mockResolvedValueOnce(res("a"));
    const { deps } = setup({ search });
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.searchNow();
    await flush();
    expect(deps.connect).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledTimes(2);
    expect(s.state().status).toBe("done");
  });

  it("ikinci not_connected hata metni olur, döngü yok", async () => {
    const search = vi.fn().mockRejectedValue(notConn());
    const { deps } = setup({ search });
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.searchNow();
    await flush();
    expect(search).toHaveBeenCalledTimes(2);
    expect(s.state().status).toBe("error");
    expect(s.state().error).toBe("bağlı değil");
  });

  it("connect false ise hata metni", async () => {
    const search = vi.fn().mockRejectedValue(notConn());
    const { deps } = setup({ search, connect: vi.fn(async () => false) });
    const s = createSearch(deps, target());
    s.setQuery("a");
    s.searchNow();
    await flush();
    expect(search).toHaveBeenCalledTimes(1);
    expect(s.state().error).toBe("bağlı değil");
  });

  it("move sınırlarda durur; activeHit", async () => {
    const { deps } = setup({ search: vi.fn(async () => res("a", "b")) });
    const s = createSearch(deps, target());
    expect(s.activeHit()).toBeNull();
    s.setQuery("a");
    s.searchNow();
    await flush();
    s.move(-1);
    expect(s.state().active).toBe(0);
    s.move(1);
    s.move(1);
    expect(s.state().active).toBe(1);
    expect(s.activeHit()?.name).toBe("b");
  });

  it("hafıza: aynı hedef geri yüklenir ve arar; farklı veritabanı boş başlar", async () => {
    const { deps } = setup();
    const t = target();
    const a = createSearch(deps, t);
    a.setQuery("abc");
    a.toggleKind("view");
    await flush();
    a.dispose();
    (deps.search as unknown as { mockClear(): void }).mockClear();
    const b = createSearch(deps, t);
    expect(b.state().query).toBe("abc");
    expect(b.state().kinds).toEqual(["view"]);
    await flush();
    expect(deps.search).toHaveBeenCalledTimes(1);
    const c = createSearch(deps, target());
    expect(c.state().query).toBe("");
    expect(c.state().status).toBe("idle");
  });
});
