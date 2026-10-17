// Tembel parça yükleyici: `ensure()` parçayı (bir kez) yükler; başarıda `true`, ret durumunda `onFail` çağrılıp `false`
// döner (ret önbelleğe alınmaz, sonraki istek yeniden dener). Aynı söz paylaşıldığı için eşzamanlı istekler sırayla çözülür.
export function createLoader<T>(load: () => Promise<T>, onFail: (err: unknown) => void): () => Promise<boolean> {
  let pending: Promise<T> | null = null;
  return async () => {
    try {
      pending ??= load();
      await pending;
      return true;
    } catch (err) {
      pending = null;
      onFail(err);
      return false;
    }
  };
}
