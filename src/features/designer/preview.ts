// Önizleme zamanlayıcısı (saf, zaman enjekte edilir): alan değişikliğinden sonra debounce, en son istek kazanır.
// Eski/iptal edilmiş çağrıların yanıtı ve hatası durumu asla değiştirmez.

export interface PreviewDeps {
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(h: unknown): void;
}

export interface Previewer<R> {
  /** Her çağrı öncekini geçersiz kılar; 150 ms sonra `run`; yalnızca EN SON çağrının yanıtı `onResult`a gider. */
  schedule(run: () => Promise<R>): void;
  /** Bekleyen zamanlayıcıyı ve uçuştaki yanıtı geçersiz kılar. */
  cancel(): void;
  /** `cancel` + sonraki `schedule` çağrıları etkisiz. */
  dispose(): void;
}

export const PREVIEW_DELAY_MS = 150;

export function createPreviewer<R>(
  deps: PreviewDeps,
  onResult: (r: R) => void,
  onError: (e: unknown) => void,
  delayMs: number = PREVIEW_DELAY_MS,
): Previewer<R> {
  let token = 0;                    // her schedule/cancel'da artar; yalnızca güncel belirteç yanıt iletebilir
  let timer: unknown = null;
  let disposed = false;

  const clear = () => {
    if (timer !== null) { deps.clearTimer(timer); timer = null; }
  };
  const cancel = () => {
    token++;
    clear();
  };

  return {
    schedule(run) {
      if (disposed) return;
      cancel();
      const mine = token;
      timer = deps.setTimer(() => {
        timer = null;
        let p: Promise<R>;
        try { p = run(); } catch (e) { p = Promise.reject(e); }
        p.then(
          (r) => { if (!disposed && mine === token) onResult(r); },
          (e) => { if (!disposed && mine === token) onError(e); },
        );
      }, delayMs);
    },
    cancel,
    dispose() {
      disposed = true;
      cancel();
    },
  };
}
