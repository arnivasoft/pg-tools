// Dashboard grafiğinin saf modeli: eksen ölçekleri ve çizim komutları (Canvas'tan bağımsız, test edilebilir).

export interface SeriesPoint { t: number; v: number | null }
export interface Series {
  name: string;
  /** CSS değişken adı, ör. "--accent". */
  color: string;
  points: SeriesPoint[];
  /** Yığılı seriler sırayla üst üste çizilir (kümülatif toplam); noktalar dizin olarak hizalı varsayılır. */
  stacked?: boolean;
}
export interface Viewport { width: number; height: number; padLeft: number; padBottom: number; padTop: number; padRight: number }

/** `s`: seri dizini; `area`: yığılı seride kapalı alan (üst kenar ileri, alt kenar geri), değilse çizgi. */
export type PathCmd =
  | { s: number; area: boolean; op: "M" | "L"; x: number; y: number }
  | { s: number; area: boolean; op: "Z" };

/** Seçilen adım kümesi: 1/2/2.5/5/10 × 10^n. */
const NICE_STEPS = [1, 2, 2.5, 5, 10];

/** `max`'ı kapsayan "güzel" üst sınır; 0/geçersiz → 1; 87 → 100; 1234 → 2000. */
export function niceMax(max: number): number {
  if (!Number.isFinite(max) || max <= 0) return 1;
  const exp = Math.floor(Math.log10(max));
  const pow = 10 ** exp;
  const m = max / pow;
  const step = NICE_STEPS.find((s) => s >= m - 1e-9) ?? 10;
  return step * pow;
}

/** 0'dan `niceMax(max)`'a eşit aralıklı `count` bölüm: `count + 1` değer. */
export function yTicks(max: number, count = 4): number[] {
  const top = niceMax(max);
  const n = Math.max(1, Math.floor(count));
  return Array.from({ length: n + 1 }, (_, i) => (top * i) / n);
}

/** Son noktadan geriye `windowSec`; hiç nokta yoksa [0, windowSec]. */
export function xDomain(series: Series[], windowSec = 600): [number, number] {
  let last = -Infinity;
  for (const s of series) for (const p of s.points) if (p.t > last) last = p.t;
  if (!Number.isFinite(last)) return [0, windowSec];
  return [last - windowSec, last];
}

const xOf = (t: number, vp: Viewport, d: [number, number]) => {
  const w = vp.width - vp.padLeft - vp.padRight;
  const span = d[1] - d[0] || 1;
  return vp.padLeft + ((t - d[0]) / span) * w;
};
const yOf = (v: number, vp: Viewport, yMax: number) => {
  const h = vp.height - vp.padTop - vp.padBottom;
  return vp.padTop + h - (v / (yMax || 1)) * h;
};

/**
 * Çizim komutları. Çizgi serisinde `null` çizgiyi keser (yeni `M`). Yığılı seride her seri kümülatif toplamın üst kenarıdır;
 * alt kenar bir önceki yığılı serinin toplamı (ilkinde 0). Boşluklu (`null`) yığılı nokta o seride kesinti yapar,
 * üstündeki serilerin toplamına 0 katar. Pencere dışındaki noktalar da üretilir (Canvas kırpar).
 */
export function buildPaths(series: Series[], vp: Viewport, domain: [number, number], yMax: number): PathCmd[] {
  const out: PathCmd[] = [];
  const base: number[] = []; // yığılı serilerin önceki kümülatif toplamı (dizine göre)
  series.forEach((s, si) => {
    if (s.stacked) {
      const tops = s.points.map((p, i) => (p.v == null ? null : (base[i] ?? 0) + p.v));
      let run: number[] = [];
      const flush = () => {
        if (run.length === 0) return;
        run.forEach((i, k) => {
          out.push({ s: si, area: true, op: k === 0 ? "M" : "L", x: xOf(s.points[i].t, vp, domain), y: yOf(tops[i] as number, vp, yMax) });
        });
        for (let k = run.length - 1; k >= 0; k--) {
          const i = run[k];
          out.push({ s: si, area: true, op: "L", x: xOf(s.points[i].t, vp, domain), y: yOf(base[i] ?? 0, vp, yMax) });
        }
        out.push({ s: si, area: true, op: "Z" });
        run = [];
      };
      s.points.forEach((_, i) => (tops[i] == null ? flush() : run.push(i)));
      flush();
      s.points.forEach((p, i) => {
        base[i] = (base[i] ?? 0) + (p.v ?? 0);
      });
      return;
    }
    let pen = false;
    for (const p of s.points) {
      if (p.v == null) {
        pen = false;
        continue;
      }
      out.push({ s: si, area: false, op: pen ? "L" : "M", x: xOf(p.t, vp, domain), y: yOf(p.v, vp, yMax) });
      pen = true;
    }
  });
  return out;
}

/** `x` (piksel) konumuna zamanca en yakın noktanın dizini; boş dizi ya da çizim alanı dışı → null. */
export function nearestIndex(points: { t: number }[], x: number, vp: Viewport, domain: [number, number]): number | null {
  if (points.length === 0) return null;
  if (x < vp.padLeft || x > vp.width - vp.padRight) return null;
  const w = vp.width - vp.padLeft - vp.padRight;
  if (w <= 0) return null;
  const t = domain[0] + ((x - vp.padLeft) / w) * (domain[1] - domain[0]);
  let best = 0;
  let bestD = Math.abs(points[0].t - t);
  for (let i = 1; i < points.length; i++) {
    const d = Math.abs(points[i].t - t);
    if (d < bestD) {
      best = i;
      bestD = d;
    }
  }
  return best;
}

/** Serilerdeki en büyük değer (yığılıda kümülatif toplam); boş/hepsi null → 0. */
export function seriesMax(series: Series[]): number {
  let max = 0;
  const sums: number[] = [];
  for (const s of series) {
    s.points.forEach((p, i) => {
      if (p.v == null) return;
      if (s.stacked) {
        sums[i] = (sums[i] ?? 0) + p.v;
        if (sums[i] > max) max = sums[i];
      } else if (p.v > max) max = p.v;
    });
  }
  return max;
}
