// Dashboard yenileme zamanlayıcısı (çerçeveden bağımsız): tek uçuşta tur, tek zamanlayıcı; sekme etkin değil, pencere
// gizli ya da duraklatılmışsa hiç tur yok. Zaman ve görünürlük enjekte edilir (testte sahte zamanlayıcı).

export interface PollerDeps {
  now(): number;
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
  /** `document.visibilityState === "visible"`. */
  visible(): boolean;
  /** Görünürlük dinleyicisi; dönüş: dinleyiciyi kaldır. */
  onVisibility(cb: () => void): () => void;
}

export interface Poller {
  /** Sekme etkin mi (ve tur yapılabilir durumda mı). */
  setActive(on: boolean): void;
  setPaused(on: boolean): void;
  setIntervalMs(ms: number): void;
  /** Hemen bir tur; tur sürüyorsa bitince bir tur daha (birleştirilir, aynı anda iki tur yok). */
  refreshNow(): void;
  dispose(): void;
}

export const DEFAULT_INTERVAL_MS = 2000;

/** Tarayıcı varsayılanları; `document` yoksa (test/node) hep görünür sayılır. */
export function defaultPollerDeps(): PollerDeps {
  const doc = typeof document === "undefined" ? null : document;
  return {
    now: () => Date.now(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    visible: () => !doc || doc.visibilityState === "visible",
    onVisibility: (cb) => {
      if (!doc) return () => {};
      doc.addEventListener("visibilitychange", cb);
      return () => doc.removeEventListener("visibilitychange", cb);
    },
  };
}

/**
 * `tick`: bir yenileme turu (söz). Önceki tur bitmeden yenisi başlamaz. Etkin değil/gizli/duraklatılmış → zamanlayıcı yok.
 * Çalışma koşulu yeniden sağlanınca (etkinleşme, görünür olma, duraklatmanın kalkması) hemen bir tur. Reddeden tur döngüyü
 * öldürmez. `dispose` sonrası tüm geri çağrılar no-op.
 */
export function createPoller(deps: PollerDeps, tick: () => Promise<void>): Poller {
  let active = false;
  let paused = false;
  let intervalMs = DEFAULT_INTERVAL_MS;
  let inFlight = false;
  let pending = false;
  let disposed = false;
  let timer: unknown = null;
  let wasRunning = false;

  const running = () => active && !paused && deps.visible();

  const clear = () => {
    if (timer !== null) {
      deps.clearTimer(timer);
      timer = null;
    }
  };

  const schedule = () => {
    clear();
    if (disposed || inFlight || !running()) return;
    timer = deps.setTimer(() => {
      timer = null;
      void run();
    }, intervalMs);
  };

  const run = async (): Promise<void> => {
    if (disposed) return;
    if (inFlight) {
      pending = true;
      return;
    }
    clear();
    inFlight = true;
    try {
      await tick();
    } catch {
      // Turun hatası denetleyicide işlenir; burada yalnızca döngü sürer.
    } finally {
      inFlight = false;
    }
    if (disposed) return;
    if (pending) {
      pending = false;
      return run();
    }
    schedule();
  };

  const reconcile = () => {
    if (disposed) return;
    const now = running();
    const resumed = now && !wasRunning;
    wasRunning = now;
    if (!now) {
      clear();
      pending = false;
    } else if (resumed) void run();
    else if (timer === null && !inFlight) schedule();
  };

  const unlisten = deps.onVisibility(reconcile);

  return {
    setActive(on) {
      active = on;
      reconcile();
    },
    setPaused(on) {
      paused = on;
      reconcile();
    },
    setIntervalMs(ms) {
      if (ms === intervalMs) return;
      intervalMs = ms;
      // Kurulu zamanlayıcı yeni aralıkla yeniden kurulur (tek zamanlayıcı); tur sürüyorsa bitince kurulur.
      if (timer !== null) schedule();
    },
    refreshNow() {
      void run();
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      clear();
      unlisten();
    },
  };
}
