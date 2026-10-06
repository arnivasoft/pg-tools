import { describe, expect, it } from "vitest";
import { buildPaths, nearestIndex, niceMax, seriesMax, xDomain, yTicks, type Series, type Viewport } from "./chart-model";

const vp: Viewport = { width: 110, height: 110, padLeft: 10, padBottom: 10, padTop: 0, padRight: 0 }; // çizim alanı 100x100
const dom: [number, number] = [0, 100];

describe("niceMax", () => {
  it("güzel adımlara yuvarlar", () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(-5)).toBe(1);
    expect(niceMax(NaN)).toBe(1);
    expect(niceMax(87)).toBe(100);
    expect(niceMax(1234)).toBe(2000);
    expect(niceMax(2.3)).toBe(2.5);
    expect(niceMax(100)).toBe(100);
    expect(niceMax(0.03)).toBeCloseTo(0.05);
  });
});

describe("yTicks", () => {
  it("0'dan niceMax'a eşit aralık", () => {
    expect(yTicks(87)).toEqual([0, 25, 50, 75, 100]);
    expect(yTicks(0, 2)).toEqual([0, 0.5, 1]);
  });
});

describe("xDomain", () => {
  it("son noktadan geriye pencere", () => {
    const s: Series = { name: "a", color: "--accent", points: [{ t: 50, v: 1 }, { t: 1000, v: 2 }] };
    expect(xDomain([s], 600)).toEqual([400, 1000]);
  });
  it("boş seride güvenli", () => {
    expect(xDomain([], 600)).toEqual([0, 600]);
    expect(xDomain([{ name: "a", color: "--x", points: [] }])).toEqual([0, 600]);
  });
});

describe("buildPaths", () => {
  it("çizgi: null çizgiyi keser", () => {
    const s: Series = { name: "a", color: "--a", points: [{ t: 0, v: 0 }, { t: 50, v: 50 }, { t: 60, v: null }, { t: 80, v: 100 }, { t: 100, v: 100 }] };
    const p = buildPaths([s], vp, dom, 100);
    expect(p.map((c) => c.op)).toEqual(["M", "L", "M", "L"]);
    expect(p[0]).toMatchObject({ x: 10, y: 100 });
    expect(p[1]).toMatchObject({ x: 60, y: 50 });
    expect(p[2]).toMatchObject({ x: 90, y: 0 });
  });
  it("yığılı: kümülatif toplam, kapalı alan", () => {
    const a: Series = { name: "a", color: "--a", stacked: true, points: [{ t: 0, v: 10 }, { t: 100, v: 10 }] };
    const b: Series = { name: "b", color: "--b", stacked: true, points: [{ t: 0, v: 20 }, { t: 100, v: 20 }] };
    const p = buildPaths([a, b], vp, dom, 100);
    const bp = p.filter((c) => c.s === 1);
    expect(bp.map((c) => c.op)).toEqual(["M", "L", "L", "L", "Z"]);
    expect(bp[0]).toMatchObject({ y: 70 }); // 10+20 = 30 → 100-30
    expect(bp[2]).toMatchObject({ y: 90 }); // alt kenar = a'nın üstü (10)
    expect(p.filter((c) => c.s === 0)[2]).toMatchObject({ y: 100 }); // ilk serinin tabanı 0
  });
  it("yığılıda null kesinti yapar, toplama 0 katar", () => {
    const a: Series = { name: "a", color: "--a", stacked: true, points: [{ t: 0, v: 10 }, { t: 50, v: null }, { t: 100, v: 10 }] };
    const p = buildPaths([a], vp, dom, 100);
    expect(p.filter((c) => c.op === "Z")).toHaveLength(2);
  });
  it("boş seri komut üretmez", () => {
    expect(buildPaths([], vp, dom, 1)).toEqual([]);
  });
});

describe("nearestIndex", () => {
  const pts = [{ t: 0 }, { t: 50 }, { t: 100 }];
  it("en yakın zamanı bulur", () => {
    expect(nearestIndex(pts, 10, vp, dom)).toBe(0);
    expect(nearestIndex(pts, 62, vp, dom)).toBe(1);
    expect(nearestIndex(pts, 110, vp, dom)).toBe(2);
  });
  it("kenar durumlar", () => {
    expect(nearestIndex([], 50, vp, dom)).toBeNull();
    expect(nearestIndex(pts, 5, vp, dom)).toBeNull();
    expect(nearestIndex(pts, 200, vp, dom)).toBeNull();
  });
});

describe("seriesMax", () => {
  it("yığılıda toplam, değilse tek değer", () => {
    const a: Series = { name: "a", color: "--a", stacked: true, points: [{ t: 0, v: 3 }, { t: 1, v: null }] };
    const b: Series = { name: "b", color: "--b", stacked: true, points: [{ t: 0, v: 4 }, { t: 1, v: 1 }] };
    expect(seriesMax([a, b])).toBe(7);
    expect(seriesMax([{ ...a, stacked: false }, { ...b, stacked: false }])).toBe(4);
    expect(seriesMax([])).toBe(0);
  });
});
