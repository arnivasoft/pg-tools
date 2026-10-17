// Ana paket bütçe denetimi: `bun run bundle:check`.
// Geçici dizine kaynak haritalı derleme yapar (dist/'e dokunmaz), ana paketi ölçer ve bütçeyle karşılaştırır.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  attributeSizes,
  countMapDeps,
  findEntryScript,
  findForbidden,
  groupSources,
  parseModulePreloads,
  preloadViolations,
  stripSourceMapComment,
} from "./bundle-check-lib.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const budget = JSON.parse(fs.readFileSync(path.join(root, "scripts", "bundle-budget.json"), "utf8"));
const fmt = (n) => n.toLocaleString("en-US").padStart(9);
const fail = [];

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "pgt-bundle-"));
try {
  const vite = path.join(root, "node_modules", "vite", "bin", "vite.js");
  const r = spawnSync(process.execPath, [vite, "build", "--sourcemap", "--outDir", tmp, "--emptyOutDir"], {
    cwd: root,
    encoding: "utf8",
  });
  if (r.status !== 0) {
    console.error(r.stdout, r.stderr);
    throw new Error("vite build başarısız");
  }

  const html = fs.readFileSync(path.join(tmp, "index.html"), "utf8");
  const entry = findEntryScript(html);
  if (!entry) throw new Error("index.html'de giriş betiği (script type=module) bulunamadı");
  const jsPath = path.join(tmp, entry);
  const mapPath = `${jsPath}.map`;
  if (!fs.existsSync(mapPath)) throw new Error(`kaynak haritası yok: ${mapPath}`);

  const raw = fs.readFileSync(jsPath, "utf8");
  const code = stripSourceMapComment(raw);
  const mainBytes = Buffer.byteLength(code);
  const map = JSON.parse(fs.readFileSync(mapPath, "utf8"));
  const sizes = attributeSizes(code, map);
  const { packages, app, total } = groupSources(sizes);
  const unmapped = sizes.get("<unmapped>") ?? 0;
  const mapped = total - unmapped;

  const mapDeps = countMapDeps(code);
  const preloads = parseModulePreloads(html);
  const pre = preloadViolations(preloads, budget.htmlPreloads);
  const forbidden = findForbidden(packages, budget.forbiddenInMain);

  console.log(`Giriş parçası: ${entry}`);
  console.log(`Ana paket:     ${fmt(mainBytes)} B  (tavan ${fmt(budget.mainMaxBytes)} B)`);
  console.log(`Eşlenen:       ${fmt(mapped)} B  (eşlenmeyen ${fmt(unmapped)} B)`);
  console.log(`__vite__mapDeps girdisi: ${mapDeps}`);
  console.log(`modulepreload: ${preloads.join(", ") || "(yok)"}  (beklenen ${budget.htmlPreloads.join(", ")})`);

  console.log("\nEn büyük 25 kaynak ve paket (bayt):");
  const rows = [
    ...[...packages].map(([k, v]) => [`npm:${k}`, v]),
    ...[...app].map(([k, v]) => [k, v]),
  ]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 25);
  for (const [k, v] of rows) console.log(`${fmt(v)}  ${k}`);

  console.log(`\nYasak paketler (${budget.enforceForbidden ? "zorunlu" : "yalnız bilgi"}): ${forbidden.length || "yok"}`);
  for (const [k, v] of forbidden) console.log(`${fmt(v)}  ${k}`);

  if (mainBytes > budget.mainMaxBytes) fail.push(`ana paket tavanı aşıldı: ${mainBytes} > ${budget.mainMaxBytes}`);
  if (budget.enforceForbidden && forbidden.length > 0) fail.push(`ana pakette yasak paket var: ${forbidden.map(([k]) => k).join(", ")}`);
  for (const v of pre) fail.push(`preload ihlali: ${v}`);
  if (mapped < mainBytes * 0.9) fail.push(`kaynak haritası eşlemesi yetersiz: ${mapped} < ${(mainBytes * 0.9).toFixed(0)} (harita ana paketle eşleşmiyor olabilir)`);
} catch (e) {
  fail.push(String(e instanceof Error ? e.message : e));
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}

if (fail.length) {
  console.error("\nBÜTÇE DENETİMİ BAŞARISIZ:");
  for (const f of fail) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("\nBütçe denetimi geçti.");
