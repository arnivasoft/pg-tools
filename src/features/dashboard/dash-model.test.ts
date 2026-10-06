import { describe, expect, it } from "vitest";
import type { Sample } from "../../ipc/bindings/Sample";
import { MAX_SAMPLES, SampleRing, formatCount, formatDuration, toRates } from "./dash-model";

const smp = (at: number, p: Partial<Sample> = {}): Sample => ({
  at, xactCommit: 0, xactRollback: 0, tupReturned: 0, tupFetched: 0, tupInserted: 0, tupUpdated: 0, tupDeleted: 0,
  blksRead: 0, blksHit: 0, sessionsActive: 0, sessionsIdle: 0, sessionsIdleInTx: 0, sessionsOther: 0, ...p,
});

describe("toRates", () => {
  it("ilk örnek yalnızca taban çizgisi; oranlar farktan", () => {
    const r = toRates([smp(100, { xactCommit: 1000 }), smp(102, { xactCommit: 1200 })], 2);
    expect(r.commit).toEqual([{ t: 102, v: 100 }]);
  });
  it("boş ve tek örnekte nokta yok", () => {
    expect(toRates([], 2).commit).toEqual([]);
    expect(toRates([smp(1)], 2).hit).toEqual([]);
  });
  it("tüm sayaçlar doğru alana eşlenir", () => {
    const a = smp(0);
    const b = smp(2, {
      xactCommit: 2, xactRollback: 4, tupInserted: 6, tupUpdated: 8, tupDeleted: 10, tupReturned: 12, tupFetched: 14,
      blksRead: 16, blksHit: 18,
    });
    const r = toRates([a, b], 2);
    expect([r.commit, r.rollback, r.ins, r.upd, r.del, r.ret, r.fetched, r.read, r.hit].map((s) => s[0].v)).toEqual([
      1, 2, 3, 4, 5, 6, 7, 8, 9,
    ]);
  });
  it("sayaç geriye giderse null (negatif oran yok); diğer sayaçlar etkilenmez", () => {
    const r = toRates([smp(100, { xactCommit: 5000, xactRollback: 1 }), smp(102, { xactCommit: 40, xactRollback: 5 })], 2);
    expect(r.commit[0].v).toBeNull();
    expect(r.rollback[0].v).toBe(2);
  });
  it("uzun aralık (uyku) boşluk bırakır; sonraki normal aralık yine hesaplanır", () => {
    const r = toRates([smp(100, {}), smp(160, { xactCommit: 9 }), smp(162, { xactCommit: 19 })], 2, 5);
    expect(r.commit.map((p) => p.v)).toEqual([null, 5]);
  });
  it("aralık tam eşikte (gapFactor x aralık) boşluk sayılmaz", () => {
    const r = toRates([smp(0), smp(10, { xactCommit: 10 })], 2, 5);
    expect(r.commit[0].v).toBe(1);
  });
  it("varsayılan gapFactor 5", () => {
    expect(toRates([smp(0), smp(11, { xactCommit: 11 })], 2).commit[0].v).toBeNull();
    expect(toRates([smp(0), smp(10, { xactCommit: 10 })], 2).commit[0].v).toBe(1);
  });
  it("aynı ya da geri giden zaman damgası null", () => {
    const r = toRates([smp(100), smp(100, { xactCommit: 5 }), smp(99, { xactCommit: 9 })], 2);
    expect(r.commit.map((p) => p.v)).toEqual([null, null]);
  });
  it("değişmeyen sayaç 0 oranı verir (null değil)", () => {
    expect(toRates([smp(0), smp(2)], 2).commit[0].v).toBe(0);
  });
});

describe("SampleRing", () => {
  it("halka 300 örnekte en eskiyi düşürür", () => {
    const ring = new SampleRing();
    for (let i = 0; i < 305; i++) ring.push(smp(i));
    expect(MAX_SAMPLES).toBe(300);
    expect(ring.samples()).toHaveLength(300);
    expect(ring.samples()[0].at).toBe(5);
    expect(ring.samples()[299].at).toBe(304);
  });
  it("clear boşaltır ve yeniden doldurulabilir", () => {
    const ring = new SampleRing();
    ring.push(smp(1));
    ring.clear();
    expect(ring.samples()).toHaveLength(0);
    ring.push(smp(2));
    expect(ring.samples().map((s) => s.at)).toEqual([2]);
  });
});

describe("SampleRing anlık görüntü", () => {
  it("push ve clear yeni dizi döndürür; önceki görüntü değişmez", () => {
    const ring = new SampleRing();
    ring.push(smp(1));
    const a = ring.samples();
    expect(ring.samples()).toBe(a); // push olmadan aynı başvuru
    ring.push(smp(2));
    const b = ring.samples();
    expect(b).not.toBe(a);
    expect(a.map((s) => s.at)).toEqual([1]);
    expect(b.map((s) => s.at)).toEqual([1, 2]);
    for (let i = 3; i < 400; i++) ring.push(smp(i));
    expect(b.map((s) => s.at)).toEqual([1, 2]);
    ring.clear();
    expect(ring.samples()).toHaveLength(0);
    expect(b).toHaveLength(2);
  });
});

describe("formatDuration", () => {
  it("süre biçimleri", () => {
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(0)).toBe("0 ms");
    expect(formatDuration(0.85)).toBe("850 ms");
    expect(formatDuration(12)).toBe("12 sn");
    expect(formatDuration(12.7)).toBe("12 sn");
    expect(formatDuration(60)).toBe("1 dk 00 sn");
    expect(formatDuration(185)).toBe("3 dk 05 sn");
    expect(formatDuration(7800)).toBe("2 sa 10 dk");
    expect(formatDuration(3600)).toBe("1 sa 00 dk");
    expect(formatDuration(86400 + 4 * 3600)).toBe("1 g 4 sa");
  });
  it("negatif ve sonlu olmayan değer —", () => {
    expect(formatDuration(-1)).toBe("—");
    expect(formatDuration(NaN)).toBe("—");
  });
});

describe("formatCount", () => {
  it("kısa tr biçimi", () => {
    expect(formatCount(0)).toBe("0");
    expect(formatCount(999)).toBe("999");
    expect(formatCount(1234)).toBe("1,2 B");
    expect(formatCount(1000)).toBe("1 B");
    expect(formatCount(1234567)).toBe("1,2 M");
    expect(formatCount(2_500_000_000)).toBe("2,5 Mr");
  });
  it("yuvarlama bir sonraki birime taşar", () => {
    expect(formatCount(999_999)).toBe("1 M");
  });
  it("negatif sayıda işareti korur", () => {
    expect(formatCount(-1500)).toBe("-1,5 B");
  });
});
