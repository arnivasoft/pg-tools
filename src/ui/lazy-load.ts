// Tembel parça yükleyici: `ensure()` parçayı (bir kez) yükler; başarıda `true`, ret durumunda `false` döner.
// Eşzamanlı istekler tek uçuştaki sözü paylaşır (aynı sırayla çözülür); `onFail` çağıran sayısından bağımsız olarak ret
// başına yalnız bir kez çağrılır. Ret önbelleğe alınmaz: sonraki istek yeniden dener. Modülün kendisi gerekiyorsa
// `load` içinde işlenir (`() => import("./X").then((m) => …)`).
// `load()` eşzamanlı fırlatırsa da reddedilmiş söze çevrilir (çağrı eşzamanlı kalır: ilk istek hemen yüklemeyi başlatır).
function safeLoad<T>(load: () => Promise<T>): Promise<T> {
  try {
    return load();
  } catch (e) {
    return Promise.reject(e);
  }
}

export function createLoader<T>(load: () => Promise<T>, onFail: (err: unknown) => void): () => Promise<boolean> {
  let pending: Promise<boolean> | null = null;
  return () =>
    (pending ??= safeLoad(load).then(
      () => true,
      (err) => {
        pending = null;
        onFail(err);
        return false;
      },
    ));
}
