import { describe, expect, it } from "vitest";
import {
  attributeSizes,
  countMapDeps,
  decodeVlq,
  findEntryScript,
  findForbidden,
  groupSources,
  packageOf,
  parseModulePreloads,
  preloadViolations,
  stripSourceMapComment,
} from "./bundle-check-lib.mjs";

describe("decodeVlq", () => {
  it("bilinen örnekleri çözer", () => {
    expect(decodeVlq("AAAA")).toEqual([0, 0, 0, 0]);
    expect(decodeVlq("C")).toEqual([1]);
    expect(decodeVlq("D")).toEqual([-1]);
    expect(decodeVlq("gB")).toEqual([16]); // devam bitli çok basamaklı
    expect(decodeVlq("2H")).toEqual([123]);
    expect(decodeVlq("hB")).toEqual([-16]);
  });
});

describe("packageOf", () => {
  it("Windows ve POSIX yollarından paket adını çıkarır", () => {
    expect(packageOf(String.raw`..\node_modules\@kobalte\core\dist\index.js`)).toBe("@kobalte/core");
    expect(packageOf("../node_modules/@floating-ui/dom/dist/x.js")).toBe("@floating-ui/dom");
    expect(packageOf("../node_modules/lucide-solid/dist/a.js")).toBe("lucide-solid");
    expect(packageOf("../node_modules/.bun/a@1/node_modules/solid-js/dist/solid.js")).toBe("solid-js");
    expect(packageOf("../src/app/App.tsx")).toBeNull();
  });
});

describe("attributeSizes", () => {
  it("segment başına bayt atar (çok baytlı karakterler dahil)", () => {
    // tek satır: "abcdéf" ; segment 0 -> kaynak 0 (col 0), segment 1 -> kaynak 1 (col 3)
    const code = "abcdéf";
    const map = { sources: ["a.ts", "b.ts"], mappings: "AAAA,GCAA" };
    const sizes = attributeSizes(code, map);
    expect(sizes.get("a.ts")).toBe(3);
    expect(sizes.get("b.ts")).toBe(Buffer.byteLength("déf"));
  });
  it("eşlenmemiş segmenti <unmapped> sayar", () => {
    const sizes = attributeSizes("xxyy", { sources: ["a.ts"], mappings: "AAAA,C,CAAA" });
    expect(sizes.get("<unmapped>")).toBe(1);
  });
});

describe("groupSources", () => {
  it("paket ve kaynak tablosunu ayırır", () => {
    const sizes = new Map([
      [String.raw`..\node_modules\@kobalte\core\a.js`, 10],
      ["../node_modules/@kobalte/core/b.js", 5],
      ["../src/app/App.tsx", 7],
    ]);
    const g = groupSources(sizes);
    expect(g.packages.get("@kobalte/core")).toBe(15);
    expect(g.packages.has("src/app/App.tsx")).toBe(false);
    expect(g.app.get("src/app/App.tsx")).toBe(7);
    expect(g.total).toBe(22);
  });
  it("findForbidden önek eşleşir", () => {
    const pk = new Map([["@kobalte/core", 15], ["@kobalte/utils", 2], ["solid-js", 9]]);
    expect(findForbidden(pk, ["@kobalte/", "solid-presence"])).toEqual([
      ["@kobalte/core", 15],
      ["@kobalte/utils", 2],
    ]);
  });
});

describe("countMapDeps", () => {
  it("dizi girdilerini sayar", () => {
    const code = 'x;const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["assets/A.js","assets/b-1.js","assets/c.css"])))=>i.map(i=>d[i]);y';
    expect(countMapDeps(code)).toBe(3);
  });
  it("yoksa 0 döner", () => {
    expect(countMapDeps("const a=1")).toBe(0);
  });
});

describe("html ayrıştırma", () => {
  const html = `<script type="module" crossorigin src="/assets/index-C0m5wWKA.js"></script>
<link rel="modulepreload" crossorigin href="/assets/solid-ZDASo5p6.js">
<link rel="modulepreload" crossorigin href="/assets/web-BI3UGYjW.js">
<link rel="stylesheet" crossorigin href="/assets/index-BbdNrWWg.css">`;
  it("giriş betiğini bulur", () => {
    expect(findEntryScript(html)).toBe("assets/index-C0m5wWKA.js");
  });
  it("modulepreload adlarını çıkarır", () => {
    expect(parseModulePreloads(html)).toEqual(["solid", "web"]);
  });
  it("preload ihlallerini bildirir", () => {
    expect(preloadViolations(["solid", "web"], ["solid", "web", "Icon"])).toEqual(["eksik: Icon"]);
    expect(preloadViolations(["solid", "web", "Icon", "kobalte"], ["solid", "web", "Icon"])).toEqual(["beklenmeyen: kobalte"]);
    expect(preloadViolations(["solid", "web", "Icon"], ["solid", "web", "Icon"])).toEqual([]);
  });
});

describe("stripSourceMapComment", () => {
  it("izleyen yorumu ve önceki satır sonunu düşer", () => {
    const body = "console.log(1);";
    const withMap = body + "\n//# sourceMappingURL=index-x.js.map";
    expect(stripSourceMapComment(withMap)).toBe(body);
    expect(stripSourceMapComment(body)).toBe(body);
  });
});
