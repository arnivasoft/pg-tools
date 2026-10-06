import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPreviewer, type PreviewDeps } from "./preview";

/** Sahte zamanlayıcı: `advance(ms)` ile ilerletilir; tanıtıcılar sayıdır. */
function fakeTimers() {
  let now = 0;
  let next = 1;
  const pending = new Map<number, { at: number; fn: () => void }>();
  const deps: PreviewDeps = {
    setTimer: (fn, ms) => { const h = next++; pending.set(h, { at: now + ms, fn }); return h; },
    clearTimer: (h) => { pending.delete(h as number); },
  };
  const advance = (ms: number) => {
    now += ms;
    for (const [h, t] of [...pending]) {
      if (t.at <= now) { pending.delete(h); t.fn(); }
    }
  };
  return { deps, advance, count: () => pending.size };
}

/** Elle çözülebilen söz. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("createPreviewer", () => {
  let t: ReturnType<typeof fakeTimers>;
  let onResult: ReturnType<typeof vi.fn<(r: string) => void>>;
  let onError: ReturnType<typeof vi.fn<(e: unknown) => void>>;
  beforeEach(() => {
    t = fakeTimers();
    onResult = vi.fn<(r: string) => void>();
    onError = vi.fn<(e: unknown) => void>();
  });

  it("150 ms debounce: üç hızlı schedule → tek run (sonuncusu)", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const a = vi.fn(async () => "a"), b = vi.fn(async () => "b"), c = vi.fn(async () => "c");
    p.schedule(a); t.advance(50);
    p.schedule(b); t.advance(50);
    p.schedule(c);
    expect(t.count()).toBe(1);
    t.advance(149);
    expect(c).not.toHaveBeenCalled();
    t.advance(1);
    expect(a).not.toHaveBeenCalled();
    expect(b).not.toHaveBeenCalled();
    expect(c).toHaveBeenCalledTimes(1);
    await flush();
    expect(onResult).toHaveBeenCalledExactlyOnceWith("c");
    expect(onError).not.toHaveBeenCalled();
  });

  it("run gecikmeden önce çağrılmaz; özel gecikme kullanılır", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError, 40);
    const run = vi.fn(async () => "x");
    p.schedule(run);
    t.advance(39);
    expect(run).not.toHaveBeenCalled();
    t.advance(1);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("eski yanıt geç gelirse (yeni schedule sonrası) onResult çağrılmaz", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const d1 = deferred<string>(), d2 = deferred<string>();
    p.schedule(() => d1.promise); t.advance(150);   // uçuşta
    p.schedule(() => d2.promise);                   // yenisi bekliyor
    d1.resolve("eski"); await flush();
    expect(onResult).not.toHaveBeenCalled();
    t.advance(150);
    d2.resolve("yeni"); await flush();
    expect(onResult).toHaveBeenCalledExactlyOnceWith("yeni");
  });

  it("eski yanıt yeni yanıttan SONRA gelse de yok sayılır", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const d1 = deferred<string>(), d2 = deferred<string>();
    p.schedule(() => d1.promise); t.advance(150);
    p.schedule(() => d2.promise); t.advance(150);
    d2.resolve("yeni"); await flush();
    d1.resolve("eski"); await flush();
    expect(onResult.mock.calls).toEqual([["yeni"]]);
  });

  it("cancel bekleyen zamanlayıcıyı iptal eder", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const run = vi.fn(async () => "x");
    p.schedule(run);
    p.cancel();
    expect(t.count()).toBe(0);
    t.advance(1000);
    expect(run).not.toHaveBeenCalled();
  });

  it("cancel uçuştaki yanıtı ve hatayı yok sayar; sonra schedule çalışır", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const d = deferred<string>(), e = deferred<string>();
    p.schedule(() => d.promise); t.advance(150);
    p.cancel();
    d.resolve("x"); await flush();
    p.schedule(() => e.promise); t.advance(150);
    p.cancel();
    e.reject(new Error("y")); await flush();
    expect(onResult).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
    p.schedule(async () => "ok"); t.advance(150); await flush();
    expect(onResult).toHaveBeenCalledExactlyOnceWith("ok");
  });

  it("dispose: bekleyen ve uçuştaki yok sayılır, sonraki schedule hiçbir şey yapmaz", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const d = deferred<string>();
    p.schedule(() => d.promise); t.advance(150);
    p.dispose();
    d.resolve("x"); await flush();
    const run = vi.fn(async () => "y");
    p.schedule(run);
    expect(t.count()).toBe(0);
    t.advance(1000); await flush();
    expect(run).not.toHaveBeenCalled();
    expect(onResult).not.toHaveBeenCalled();
    p.dispose();                                     // ikinci çağrı zararsız
  });

  it("run reddederse onError (yalnızca son çağrı için) ve sonraki schedule çalışır", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const d1 = deferred<string>();
    p.schedule(() => d1.promise); t.advance(150);
    p.schedule(async () => "iyi"); // d1'i geçersiz kıldı
    d1.reject(new Error("eski hata")); await flush();
    expect(onError).not.toHaveBeenCalled();
    t.advance(150); await flush();
    expect(onResult).toHaveBeenLastCalledWith("iyi");

    const boom = new Error("boom");
    p.schedule(async () => { throw boom; }); t.advance(150); await flush();
    expect(onError).toHaveBeenCalledExactlyOnceWith(boom);
    p.schedule(async () => "sonra"); t.advance(150); await flush();
    expect(onResult).toHaveBeenLastCalledWith("sonra");
    expect(onResult).toHaveBeenCalledTimes(2);
  });

  it("run eşzamanlı fırlatırsa onError'a gider", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const boom = new Error("sync");
    p.schedule(() => { throw boom; }); t.advance(150); await flush();
    expect(onError).toHaveBeenCalledExactlyOnceWith(boom);
  });

  it("yanıt geldikten sonra aynı çağrı bir daha raporlanmaz; her schedule bağımsızdır", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    p.schedule(async () => "1"); t.advance(150); await flush();
    p.schedule(async () => "2"); t.advance(150); await flush();
    expect(onResult.mock.calls).toEqual([["1"], ["2"]]);
  });

  it("onResult/onError fırlatırsa yutulur (işlenmemiş ret yok), sonraki schedule çalışır", async () => {
    const unhandled = vi.fn();
    const proc = (globalThis as unknown as { process: { on(ev: string, f: unknown): void; off(ev: string, f: unknown): void } }).process;
    proc.on("unhandledRejection", unhandled);
    try {
      const p = createPreviewer<string>(
        t.deps,
        (r) => { onResult(r); throw new Error("onResult patladı"); },
        (e) => { onError(e); throw new Error("onError patladı"); },
      );
      p.schedule(async () => "a"); t.advance(150); await flush();
      p.schedule(async () => { throw new Error("x"); }); t.advance(150); await flush();
      p.schedule(async () => "b"); t.advance(150); await flush();
      await flush();
      expect(onResult.mock.calls).toEqual([["a"], ["b"]]);
      expect(onError).toHaveBeenCalledTimes(1);
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      proc.off("unhandledRejection", unhandled);
    }
  });

  it("runNow: bekleyen zamanlayıcıyı iptal eder, hemen çalıştırır; en son kazanır kuralı aynı", async () => {
    const p = createPreviewer<string>(t.deps, onResult, onError);
    const later = vi.fn(async () => "zamanlı");
    p.schedule(later);
    const d = deferred<string>();
    p.runNow(() => d.promise);
    expect(t.count()).toBe(0);
    t.advance(1000);
    expect(later).not.toHaveBeenCalled();
    d.resolve("hemen"); await flush();
    expect(onResult).toHaveBeenCalledExactlyOnceWith("hemen");
    const d2 = deferred<string>();
    p.runNow(() => d2.promise);
    p.schedule(async () => "yeni"); // d2'yi geçersiz kılar
    d2.resolve("eski"); await flush();
    t.advance(150); await flush();
    expect(onResult).toHaveBeenLastCalledWith("yeni");
    expect(onResult).toHaveBeenCalledTimes(2);
    p.dispose();
    const run = vi.fn(async () => "z");
    p.runNow(run);
    expect(run).not.toHaveBeenCalled();
  });
});
