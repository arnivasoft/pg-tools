import { describe, expect, it } from "vitest";
import {
  attributeSizes,
  countMapDeps,
  decodeVlq,
  findEntryScript,
  findForbidden,
  groupSources,
  packageOf,
  parseStaticImports,
  resolveChunk,
  staticClosure,
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

describe("parseStaticImports", () => {
  it("küçültülmüş adlandırılmış, yan etkili ve yeniden dışa aktarma içe aktarmalarını bulur", () => {
    const code = 'import{a as b,c as d}from"./x-1.js";import"./side.js";import e from"./def.js";import*as ns from"./ns.js";export{f}from"./re.js";const z=1;';
    expect(parseStaticImports(code).sort()).toEqual(["./def.js", "./ns.js", "./re.js", "./side.js", "./x-1.js"]);
  });
  it("export* ve export*as biçimlerini bulur", () => {
    expect(parseStaticImports('export*from"./x.js";export*as n from"./y.js";').sort()).toEqual(["./x.js", "./y.js"]);
  });
  it("dize içeriğindeki / ile başlayan yolları içe aktarma saymaz", () => {
    expect(parseStaticImports('fetch("/api/export");const u="/import";const v=a/export;import"./ok.js";')).toEqual(["./ok.js"]);
  });
  it("yorum önekli içe aktarmayı tanır", () => {
    expect(parseStaticImports('/*!x*/import"./c.js";/* a */import{a}from"./d.js";').sort()).toEqual(["./c.js", "./d.js"]);
  });
  it("./ dışı göreli, kök göreli ve çıplak belirteçleri olduğu gibi döner", () => {
    expect(parseStaticImports('import"../x.js";import{a}from"/assets/y.js";import"solid-js";').sort()).toEqual(["../x.js", "/assets/y.js", "solid-js"]);
  });
  it("dinamik import() ve mapDeps girdilerini yok sayar", () => {
    const code = 'import{a}from"./x.js";const l=()=>import("./y.js");m.f=["assets/z.js"];import("./w.js").then(()=>1);';
    expect(parseStaticImports(code)).toEqual(["./x.js"]);
  });
  it("çok satırlı biçimi ve yinelenenleri ele alır", () => {
    const code = `import {
  a,
  b
} from "./m.js";
import { c } from "./m.js";`;
    expect(parseStaticImports(code)).toEqual(["./m.js"]);
  });
});

describe("resolveChunk", () => {
  it("./, ../ ve / belirteçlerini çıktı köküne göre çözer", () => {
    expect(resolveChunk("assets/index.js", "./a.js")).toBe("assets/a.js");
    expect(resolveChunk("assets/index.js", "../x.js")).toBe("x.js");
    expect(resolveChunk("assets/sub/b.js", "../c.js")).toBe("assets/c.js");
    expect(resolveChunk("assets/index.js", "/assets/y.js")).toBe("assets/y.js");
  });
  it("çıplak ya da kökün dışına çıkan belirteçte hata verir", () => {
    expect(() => resolveChunk("assets/index.js", "solid-js")).toThrow(/çözülemeyen/);
    expect(() => resolveChunk("assets/index.js", "../../x.js")).toThrow(/çözülemeyen/);
  });
});

describe("staticClosure", () => {
  it("geçişli statik bağımlılıkları toplar, döngü ve dinamik bağı atlar", () => {
    const files = {
      "assets/e.js": 'import{a}from"./a.js";const d=()=>import("./lazy.js");export*from"./r.js";',
      "assets/a.js": 'import{b}from"/assets/b.js";import"./e.js";',
      "assets/b.js": "export const b=1;",
      "assets/r.js": "export const r=1;",
      "assets/lazy.js": 'import"./never.js";',
    };
    expect(staticClosure("assets/e.js", (n) => files[n] ?? null).sort()).toEqual(["assets/a.js", "assets/b.js", "assets/e.js", "assets/r.js"]);
  });
  it("eksik parça dosyasında hata verir (sessizce atlamaz)", () => {
    const files = { "assets/e.js": 'import"./gone.js";' };
    expect(() => staticClosure("assets/e.js", (n) => files[n] ?? null)).toThrow(/assets\/gone\.js/);
  });
  it("çözülemeyen belirteçte hata verir", () => {
    const files = { "assets/e.js": 'import"solid-js";' };
    expect(() => staticClosure("assets/e.js", (n) => files[n] ?? null)).toThrow(/çözülemeyen/);
  });
});
