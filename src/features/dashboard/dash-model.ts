// Dashboard grafik modeli (saf): örnek halkası, sayaçlardan saniyelik oranlar ve kısa biçimler.
import type { Sample } from "../../ipc/bindings/Sample";

export const MAX_SAMPLES = 300;

/** `t`: örnek zamanı (sunucu epoch sn); `v: null` = boşluk (çizgi kesilir). */
export interface RatePoint { t: number; v: number | null }
export interface RateSeries {
  commit: RatePoint[]; rollback: RatePoint[]; ins: RatePoint[]; upd: RatePoint[]; del: RatePoint[];
  ret: RatePoint[]; fetched: RatePoint[]; read: RatePoint[]; hit: RatePoint[];
}

/**
 * Bellekte en çok `MAX_SAMPLES` örnek tutar; dolunca en eski düşer. Her `push`/`clear` yeni bir dizi kurar: `samples()`
 * değişmez bir anlık görüntüdür (Solid tüketicileri başvuru değişimini görür; eski görüntü sonradan değişmez).
 */
export class SampleRing {
  private buf: readonly Sample[] = [];
  push(s: Sample): void {
    const next = [...this.buf, s];
    this.buf = next.length > MAX_SAMPLES ? next.slice(next.length - MAX_SAMPLES) : next;
  }
  clear(): void {
    this.buf = [];
  }
  samples(): readonly Sample[] {
    return this.buf;
  }
}

const COUNTERS: [keyof RateSeries, keyof Sample][] = [
  ["commit", "xactCommit"], ["rollback", "xactRollback"], ["ins", "tupInserted"], ["upd", "tupUpdated"], ["del", "tupDeleted"],
  ["ret", "tupReturned"], ["fetched", "tupFetched"], ["read", "blksRead"], ["hit", "blksHit"],
];

/**
 * Ardışık örneklerden saniyelik oranlar; ilk örnek yalnızca taban çizgisidir (nokta üretmez). Sayaç geriye gittiyse
 * (`pg_stat_reset`, yeniden bağlanma), zaman ilerlemediyse ya da aralık `gapFactor * intervalSec`'ten uzunsa (uyku) o nokta null.
 */
export function toRates(samples: readonly Sample[], intervalSec: number, gapFactor = 5): RateSeries {
  const out: RateSeries = { commit: [], rollback: [], ins: [], upd: [], del: [], ret: [], fetched: [], read: [], hit: [] };
  for (let i = 1; i < samples.length; i++) {
    const prev = samples[i - 1];
    const cur = samples[i];
    const dt = cur.at - prev.at;
    const gap = dt <= 0 || dt > gapFactor * intervalSec;
    for (const [key, field] of COUNTERS) {
      const dv = cur[field] - prev[field];
      out[key].push({ t: cur.at, v: gap || dv < 0 ? null : dv / dt });
    }
  }
  return out;
}

/** "—", "850 ms", "12 sn", "3 dk 05 sn", "2 sa 10 dk", "1 g 4 sa". */
export function formatDuration(secs: number | null): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "—";
  if (secs < 1) {
    const ms = Math.round(secs * 1000);
    return ms >= 1000 ? "1 sn" : `${ms} ms`;
  }
  const s = Math.floor(secs);
  if (s < 60) return `${s} sn`;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (s < 3600) return `${Math.floor(s / 60)} dk ${pad(s % 60)} sn`;
  if (s < 86400) return `${Math.floor(s / 3600)} sa ${pad(Math.floor((s % 3600) / 60))} dk`;
  return `${Math.floor(s / 86400)} g ${Math.floor((s % 86400) / 3600)} sa`;
}

const COUNT_UNITS = ["", "B", "M", "Mr", "Tr"];

/** Kısa tr biçimi: 1 234 → "1,2 B", 1 234 567 → "1,2 M" (B: bin, M: milyon, Mr: milyar, Tr: trilyon). */
export function formatCount(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const sign = n < 0 ? "-" : "";
  let v = Math.abs(n);
  let i = 0;
  while (v >= 1000 && i < COUNT_UNITS.length - 1) {
    v /= 1000;
    i++;
  }
  let r = i === 0 ? Math.round(v) : Math.round(v * 10) / 10;
  if (r >= 1000 && i < COUNT_UNITS.length - 1) {
    r = Math.round(r / 100) / 10;
    i++;
  }
  const text = String(r).replace(".", ",");
  return `${sign}${text}${COUNT_UNITS[i] ? " " + COUNT_UNITS[i] : ""}`;
}
