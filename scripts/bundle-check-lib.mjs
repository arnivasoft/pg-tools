// Ana paket bütçe denetiminin saf fonksiyonları (bundle-check.mjs çalıştırıcısı kullanır; testler: bundle-check-lib.test.mjs).

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Kaynak haritası VLQ segmentini sayı dizisine çözer. */
export function decodeVlq(seg) {
  const out = [];
  let value = 0;
  let shift = 0;
  for (const ch of seg) {
    const d = B64.indexOf(ch);
    if (d < 0) throw new Error(`geçersiz VLQ karakteri: ${ch}`);
    value += (d & 31) * 2 ** shift;
    if (d & 32) {
      shift += 5;
    } else {
      out.push(value % 2 === 1 ? -(value - 1) / 2 : value / 2);
      value = 0;
      shift = 0;
    }
  }
  return out;
}

/**
 * Üretilen koddaki her baytı eşlendiği kaynağa yazar (segment, bir sonrakinin başlangıcına kadar sürer).
 * Eşlenmeyen bölgeler "<unmapped>" olur. Sütunlar UTF-16 birimi, boyut UTF-8 baytıdır.
 */
export function attributeSizes(code, map) {
  const lines = code.split("\n");
  const sizes = new Map();
  const add = (key, n) => sizes.set(key, (sizes.get(key) ?? 0) + n);
  const name = (idx) => (idx < 0 ? "<unmapped>" : map.sources[idx]);
  let src = 0;
  map.mappings.split(";").forEach((ln, li) => {
    const text = lines[li] ?? "";
    let col = 0;
    let prev = null;
    for (const seg of ln ? ln.split(",").map(decodeVlq) : []) {
      col += seg[0];
      if (seg.length > 1) src += seg[1];
      if (prev) add(name(prev.src), Buffer.byteLength(text.slice(prev.col, col)));
      prev = { src: seg.length > 1 ? src : -1, col };
    }
    if (prev) add(name(prev.src), Buffer.byteLength(text.slice(prev.col)));
  });
  return sizes;
}

const norm = (p) => p.split("\\").join("/");

/** Yoldan npm paket adını çıkarır (son node_modules'e göre; kapsamlı adlar dahil); uygulama kaynağıysa null. */
export function packageOf(path) {
  const p = norm(path);
  const i = p.lastIndexOf("node_modules/");
  if (i < 0) return null;
  const m = p.slice(i + "node_modules/".length).match(/^(@[^/]+\/[^/]+|[^/]+)/);
  return m ? m[1] : null;
}

/** Kaynak başına baytları pakete (npm) ve uygulama kaynağına ayırır. */
export function groupSources(sizes) {
  const packages = new Map();
  const app = new Map();
  let total = 0;
  for (const [path, n] of sizes) {
    total += n;
    const pkg = packageOf(path);
    const bag = pkg ? packages : app;
    const key = pkg ?? norm(path).replace(/^.*?(?=src\/)/, "");
    bag.set(key, (bag.get(key) ?? 0) + n);
  }
  return { packages, app, total };
}

/** Paket adı bir yasak önekle başlıyorsa [ad, bayt] döner (bayta göre azalan). */
export function findForbidden(packages, prefixes) {
  return [...packages]
    .filter(([name]) => prefixes.some((p) => name.startsWith(p)))
    .sort((a, b) => b[1] - a[1]);
}

/** `__vite__mapDeps` dizisinin girdi sayısı; tanım yoksa 0. */
export function countMapDeps(code) {
  const m = code.match(/m\.f=\[([^\]]*)\]/);
  if (!m) return 0;
  return (m[1].match(/"[^"]*"|'[^']*'/g) ?? []).length;
}

/** index.html'deki `<script type="module" src>` giriş parçasının yolu (başındaki / olmadan). */
export function findEntryScript(html) {
  for (const tag of html.match(/<script\b[^>]*>/g) ?? []) {
    if (!/type=["']module["']/.test(tag)) continue;
    const src = tag.match(/\bsrc=["']([^"']+)["']/);
    if (src) return src[1].replace(/^\//, "");
  }
  return null;
}

/** `modulepreload` bağlantılarının parça adları (`assets/solid-ZDASo5p6.js` -> `solid`). */
export function parseModulePreloads(html) {
  const names = [];
  for (const tag of html.match(/<link\b[^>]*>/g) ?? []) {
    if (!/rel=["']modulepreload["']/.test(tag)) continue;
    const href = tag.match(/\bhref=["']([^"']+)["']/);
    if (!href) continue;
    const file = href[1].split("/").pop() ?? "";
    names.push(file.replace(/\.js$/, "").replace(/-[A-Za-z0-9_-]{8}$/, ""));
  }
  return names;
}

/** Beklenen preload kümesinden sapmalar (eksik / beklenmeyen). */
export function preloadViolations(actual, expected) {
  const out = [];
  for (const e of expected) if (!actual.includes(e)) out.push(`eksik: ${e}`);
  for (const a of actual) if (!expected.includes(a)) out.push(`beklenmeyen: ${a}`);
  return out;
}

/** Kaynak haritası yorumunu ve öncesindeki satır sonunu düşer (`vite build` çıktısıyla aynı bayt). */
export function stripSourceMapComment(code) {
  return code.replace(/\r?\n\/\/# sourceMappingURL=[^\n]*\s*$/, "");
}

/**
 * Koddaki statik içe aktarma belirteçleri, olduğu gibi (`import{a as b}from"./x.js"`, `import"./x.js"`, `export{a}from"./x.js"`,
 * `export*from"./x.js"`, yorum önekli `/*!x*\/import"./x.js"`); dinamik `import()` ve mapDeps girdileri sayılmaz.
 * Belirteçler çözülmez: `resolveChunk` çözer, çözülemeyende hata verir (yanlış "temiz" yok).
 * Önek yalnız deyim sınırı (`;`, `{`, `}`, boşluk, satır başı) ya da blok yorum sonu olabilir; bu yüzden
 * `fetch("/api/export")` gibi dize içerikleri eşleşmez. Bilinen kalan yanlış alarm: dize içinde boşluk + import/export +
 * tırnak (ör. `"a import 'x'"`). Sessiz geçmez: denetim "çözülemeyen statik içe aktarma" hatasıyla açıkça düşer.
 */
export function parseStaticImports(code) {
  const out = [];
  const re = /(?:^|[;{}\s]|\*\/)(?:import|export)\s*(?:[\w$*{][^"'`;]*?\s*from\s*)?(["'])([^"'\n]+)\1/g;
  for (const m of code.matchAll(re)) out.push(m[2]);
  return [...new Set(out)];
}

/**
 * Belirteci, içe aktaran parçanın yoluna göre çıktı köküne göreli yola çözer (`assets/index.js` + `./a.js` -> `assets/a.js`,
 * `/assets/a.js` -> `assets/a.js`). Çıplak belirteç ya da kökün dışına çıkan yol hata verir.
 */
export function resolveChunk(from, spec) {
  const fail = () => {
    throw new Error(`çözülemeyen statik içe aktarma: "${spec}" (${from})`);
  };
  let parts;
  if (spec.startsWith("/")) parts = [];
  else if (spec.startsWith("./") || spec.startsWith("../")) parts = from.split("/").slice(0, -1);
  else fail();
  for (const seg of spec.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      if (parts.length === 0) fail();
      parts.pop();
    } else parts.push(seg);
  }
  return parts.join("/");
}

/**
 * Giriş parçasının statik içe aktarma kapanışı (kendisi dahil). Adlar çıktı köküne göreli yollardır (`assets/index-x.js`);
 * `read(name)` parça kodunu döner, yoksa null — eksik parça ya da çözülemeyen belirteç hata verir.
 */
export function staticClosure(entryName, read) {
  const seen = new Set();
  const stack = [[entryName, null]];
  while (stack.length) {
    const [name, by] = stack.pop();
    if (seen.has(name)) continue;
    seen.add(name);
    const code = read(name);
    if (code == null) throw new Error(`parça bulunamadı: ${name}${by ? ` (${by} içe aktarıyor)` : ""}`);
    for (const spec of parseStaticImports(code)) stack.push([resolveChunk(name, spec), name]);
  }
  return [...seen];
}
