import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPoller, type PollerDeps } from "./poller";

/** Sahte görünürlük + sahte zamanlayıcı (vi.useFakeTimers); kurulu zamanlayıcılar sayılır. */
function fakeDeps() {
  let visible = true;
  const listeners = new Set<() => void>();
  const live = new Set<unknown>();
  const deps: PollerDeps = {
    now: () => Date.now(),
    setTimer: (fn, ms) => {
      const h = setTimeout(() => {
        live.delete(h);
        fn();
      }, ms);
      live.add(h);
      return h;
    },
    clearTimer: (h) => {
      live.delete(h);
      clearTimeout(h as ReturnType<typeof setTimeout>);
    },
    visible: () => visible,
    onVisibility: (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  return {
    deps,
    timers: () => live.size,
    listeners: () => listeners.size,
    setVisible(v: boolean) {
      visible = v;
      for (const l of [...listeners]) l();
    },
  };
}

/** Elle bitirilen turlar. */
function controlledTick() {
  const pending: { resolve: () => void; reject: (e: unknown) => void }[] = [];
  const tick = vi.fn(
    () =>
      new Promise<void>((resolve, reject) => {
        pending.push({ resolve, reject });
      }),
  );
  return {
    tick,
    finish: async () => {
      pending.shift()?.resolve();
      await vi.advanceTimersByTimeAsync(0);
    },
    fail: async () => {
      pending.shift()?.reject(new Error("x"));
      await vi.advanceTimersByTimeAsync(0);
    },
    inFlight: () => pending.length,
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("poller", () => {
  it("etkin değilken ya da gizliyken ya da duraklatılmışken hiç tick yok", async () => {
    const f = fakeDeps();
    const tick = vi.fn(async () => {});
    const p = createPoller(f.deps, tick);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(tick).not.toHaveBeenCalled(); // etkin değil

    f.setVisible(false);
    p.setActive(true);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(tick).not.toHaveBeenCalled(); // gizli
    expect(f.timers()).toBe(0);

    p.setPaused(true);
    f.setVisible(true);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(tick).not.toHaveBeenCalled(); // duraklatılmış
    expect(f.timers()).toBe(0);

    p.setPaused(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(1); // dönüşte hemen
    p.setActive(false);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(tick).toHaveBeenCalledTimes(1);
    expect(f.timers()).toBe(0);
    p.dispose();
  });

  it("tick bitmeden yenisi başlamaz (tek uçuşta); bitince bir sonraki aralıkta başlar", async () => {
    const f = fakeDeps();
    const c = controlledTick();
    const p = createPoller(f.deps, c.tick);
    p.setIntervalMs(1000);
    p.setActive(true);
    expect(c.tick).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(c.tick).toHaveBeenCalledTimes(1); // uçuşta: zamanlayıcı yok
    expect(f.timers()).toBe(0);
    p.refreshNow(); // uçuşta: aynı anda ikinci tur yok
    expect(c.tick).toHaveBeenCalledTimes(1);
    await c.finish(); // birleştirilmiş elle yenileme hemen
    expect(c.tick).toHaveBeenCalledTimes(2);
    await c.finish();
    expect(c.tick).toHaveBeenCalledTimes(2);
    expect(f.timers()).toBe(1);
    await vi.advanceTimersByTimeAsync(999);
    expect(c.tick).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(c.tick).toHaveBeenCalledTimes(3);
    expect(c.inFlight()).toBe(1);
    p.dispose();
  });

  it("aralık değişince tek zamanlayıcı kalır (çift zamanlayıcı yok)", async () => {
    const f = fakeDeps();
    const tick = vi.fn(async () => {});
    const p = createPoller(f.deps, tick);
    p.setActive(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(1);
    expect(f.timers()).toBe(1);
    p.setIntervalMs(5000);
    p.setIntervalMs(1000);
    p.setIntervalMs(10_000);
    expect(f.timers()).toBe(1);
    await vi.advanceTimersByTimeAsync(9_999);
    expect(tick).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(tick).toHaveBeenCalledTimes(2);
    expect(f.timers()).toBe(1);
    // etkinleştirme/duraklatma tekrarları da zamanlayıcıyı çoğaltmaz
    p.setActive(true);
    p.setPaused(false);
    f.setVisible(true);
    expect(f.timers()).toBe(1);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(tick).toHaveBeenCalledTimes(3);
    p.dispose();
  });

  it("görünür olunca / etkinleşince hemen bir tick çalışır", async () => {
    const f = fakeDeps();
    const tick = vi.fn(async () => {});
    const p = createPoller(f.deps, tick);
    p.setIntervalMs(30_000);
    p.setActive(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(1);
    f.setVisible(false);
    expect(f.timers()).toBe(0);
    await vi.advanceTimersByTimeAsync(5_000);
    f.setVisible(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(2); // aralığı beklemeden
    p.setActive(false);
    p.setActive(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(3);
    expect(f.timers()).toBe(1);
    p.dispose();
  });

  it("dispose zamanlayıcıyı ve dinleyiciyi kaldırır; uçuştaki tick sonucu yoksayılır", async () => {
    const f = fakeDeps();
    const c = controlledTick();
    const p = createPoller(f.deps, c.tick);
    expect(f.listeners()).toBe(1);
    p.setActive(true);
    expect(c.tick).toHaveBeenCalledTimes(1);
    p.refreshNow(); // bekleyen yenileme de düşer
    p.dispose();
    expect(f.listeners()).toBe(0);
    await c.finish();
    expect(f.timers()).toBe(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(c.tick).toHaveBeenCalledTimes(1);
    // dispose sonrası çağrılar no-op
    p.setActive(true);
    p.refreshNow();
    p.setIntervalMs(1000);
    f.setVisible(true);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(c.tick).toHaveBeenCalledTimes(1);
    expect(f.timers()).toBe(0);
  });

  it("dispose bekleyen zamanlayıcıyı kaldırır", async () => {
    const f = fakeDeps();
    const tick = vi.fn(async () => {});
    const p = createPoller(f.deps, tick);
    p.setActive(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.timers()).toBe(1);
    p.dispose();
    expect(f.timers()).toBe(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tick).toHaveBeenCalledTimes(1);
  });

  it("tick reddederse zamanlayıcı yine kurulur (hata döngüyü öldürmez)", async () => {
    const f = fakeDeps();
    const c = controlledTick();
    const p = createPoller(f.deps, c.tick);
    p.setIntervalMs(1000);
    p.setActive(true);
    await c.fail();
    expect(f.timers()).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(c.tick).toHaveBeenCalledTimes(2);
    await c.fail();
    await vi.advanceTimersByTimeAsync(1000);
    expect(c.tick).toHaveBeenCalledTimes(3);
    p.dispose();
  });

  it("duraklatılmışken refreshNow tek tur yapar ama zamanlayıcı kurmaz", async () => {
    const f = fakeDeps();
    const tick = vi.fn(async () => {});
    const p = createPoller(f.deps, tick);
    p.setPaused(true);
    p.setActive(true);
    p.refreshNow();
    await vi.advanceTimersByTimeAsync(0);
    expect(tick).toHaveBeenCalledTimes(1);
    expect(f.timers()).toBe(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tick).toHaveBeenCalledTimes(1);
    p.dispose();
  });
});
