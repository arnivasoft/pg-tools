// Ayar kaydı (saf): değer hemen uygulanır (arayüz gecikmesiz ve art arda değişiklikler birbirini ezmez), yazımlar
// sırayla yapılır (son yazım kazanır), yazım başarısızsa — o arada daha yeni bir değişiklik yoksa — önceki değere
// dönülür ve hata bildirilir.

export interface SerialSaverDeps<T> {
  get(): T;
  set(value: T): void;
  save(value: T): Promise<unknown>;
  failed(e: unknown): void;
}

export function createSerialSaver<T>(deps: SerialSaverDeps<T>): { save(value: T): Promise<void> } {
  let chain: Promise<void> = Promise.resolve();
  return {
    save(value) {
      const prev = deps.get();
      deps.set(value);
      chain = chain.then(async () => {
        try {
          await deps.save(value);
        } catch (e) {
          if (deps.get() === value) deps.set(prev);
          deps.failed(e);
        }
      });
      return chain;
    },
  };
}
