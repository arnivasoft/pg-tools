// Tembel parça yükleyici: `ensure()` parçayı (bir kez) yükler; başarıda `true`, ret durumunda `false` döner.
// Eşzamanlı istekler tek uçuştaki sözü paylaşır (aynı sırayla çözülür); `onFail` çağıran sayısından bağımsız olarak ret
// başına yalnız bir kez çağrılır. Ret önbelleğe alınmaz: sonraki istek yeniden dener. Modülün kendisi gerekiyorsa
// `load` içinde işlenir (`() => import("./X").then((m) => …)`).
export function createLoader<T>(load: () => Promise<T>, onFail: (err: unknown) => void): () => Promise<boolean> {
  let pending: Promise<boolean> | null = null;
  return () =>
    (pending ??= load().then(
      () => true,
      (err) => {
        pending = null;
        onFail(err);
        return false;
      },
    ));
}
