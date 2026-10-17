// Satır listesi düzenleyicilerinin saf işlemleri (tembel parça; Solid içe aktarmaz). Her işlem yeni dizi döndürür,
// girdi değiştirilmez. Kilitli satır (ör. yüklü enum değeri, yüklü composite özniteliği) taşınamaz ve kilitli satırla
// yer değiştirilemez; silme kısıtı çağıranındır (düğme kapatılır).

/** Taşıma yönü: -1 yukarı, +1 aşağı. */
export type MoveDir = -1 | 1;

/** `i` satırı `dir` yönünde taşınabilir mi: hedef liste içinde ve iki satır da kilitsiz. */
export function canMove(i: number, dir: MoveDir, length: number, locked?: (i: number) => boolean): boolean {
  const j = i + dir;
  if (!Number.isInteger(i) || i < 0 || i >= length || j < 0 || j >= length) return false;
  return !(locked?.(i) || locked?.(j));
}

/** `i` satırını komşusuyla yer değiştirir; taşınamazsa değişmemiş kopya döner. */
export function moveRow<T>(list: readonly T[], i: number, dir: MoveDir, locked?: (i: number) => boolean): T[] {
  const out = [...list];
  if (!canMove(i, dir, list.length, locked)) return out;
  const j = i + dir;
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

/** `i` satırını çıkarır; geçersiz indekste değişmemiş kopya döner. */
export function removeRow<T>(list: readonly T[], i: number): T[] {
  if (!Number.isInteger(i) || i < 0 || i >= list.length) return [...list];
  return [...list.slice(0, i), ...list.slice(i + 1)];
}

/** `item`'ı `i` konumuna ekler (`i` [0, length] aralığına sıkıştırılır). */
export function insertRow<T>(list: readonly T[], i: number, item: T): T[] {
  const at = Math.min(Math.max(Number.isFinite(i) ? Math.trunc(i) : list.length, 0), list.length);
  return [...list.slice(0, at), item, ...list.slice(at)];
}
