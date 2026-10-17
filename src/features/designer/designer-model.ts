// Nesne diyaloglarının saf modeli: hata eşleme, uygulama kapısı (güvenlik), betik aralıkları, indeks kolon işlemleri.
// İstemci SQL üretmez; yalnızca arka ucun döndürdüğü PlanView'i yorumlar.
import type { AclDef } from "../../ipc/bindings/AclDef";
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { DdlCreate } from "../../ipc/bindings/DdlCreate";
import type { DdlFocus } from "../../ipc/bindings/DdlFocus";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Statement } from "../../ipc/bindings/Statement";

export type DesignerKind = ObjectKind;

/**
 * Plan 22: designer sekmesinin türü. `object`: nesne diyaloğu (`ObjectDef`); `acl`: nesne yetkileri (`AclDef`);
 * `default_acl`: varsayılan yetkiler (`DefaultAclDef`).
 */
export type DesignerVariant = "object" | "acl" | "default_acl";
/** Designer'ın düzenlediği tanım (varyanta göre). */
export type DesignerDef = ObjectDef | AclDef | DefaultAclDef;

/**
 * Nesne tanımı mı (yetki tanımı değil). `viewer` yalnız yetki tanımlarında bulunur (`AclDef.kind` nesne türünü
 * taşıdığından `kind` ayırt etmez).
 */
export function isObjectDef(d: DesignerDef): d is ObjectDef {
  return !("viewer" in d);
}
/** Nesne tanımı ise kendisi, aksi halde (yetki tanımı ya da null) null. */
export function objectDefOf(d: DesignerDef | null): ObjectDef | null {
  return d && isObjectDef(d) ? d : null;
}
export function isAclDef(d: DesignerDef): d is AclDef {
  return "viewer" in d && "grants" in d;
}
export function isDefaultAclDef(d: DesignerDef): d is DefaultAclDef {
  return "viewer" in d && "sections" in d;
}

const KIND_LABELS: Record<ObjectKind, string> = {
  schema: "Schema",
  sequence: "Sequence",
  extension: "Extension",
  index: "Index",
  table: "Table",
  view: "View",
  materialized_view: "Materialized View",
  database: "Database",
  role: "Role",
  function: "Function",
  procedure: "Procedure",
  trigger: "Trigger",
  type: "Type",
  domain: "Domain",
};
export function kindLabel(kind: ObjectKind): string {
  return KIND_LABELS[kind];
}

/** Plan 14: sunucu kapsamlı türler (bakım veritabanında; databaseOid her zaman null). */
export function isServerScoped(kind: ObjectKind): boolean {
  return kind === "database" || kind === "role";
}

/** Planda gizlenmiş (parola) ifade var mı → Query Tool'a aktarılmaz/kopyalanmaz, uygulamada parola gerekir. */
export function planHasRedacted(view: PlanView | null): boolean {
  return !!view && view.statements.some((s) => s.redacted);
}

/**
 * SQL önizlemesindeki "Kopyala" ve ApplyBar'daki "Query Tool'da aç" açık mı (tek karar noktası). Görünüm yoksa ya da
 * planda gizlenmiş ifade varsa ikisi de kapalı (gizlenmiş metin bilerek çalıştırılamaz; Plan 14 Karar 6).
 */
export function scriptActionsEnabled(view: PlanView | null): { copy: boolean; open: boolean } {
  const ok = view !== null && !planHasRedacted(view);
  return { copy: ok, open: ok };
}

/**
 * Başlıkta/mesajlarda görünen ad; şema altındaki türler (sequence, indeks, tablo, view, MV, function, procedure, type,
 * domain) şema nitelikli, trigger tablosuyla (`şema.tablo.ad`), ad boşsa boş. Yetki tanımları: acl → nesne adı
 * (`şema.ad`, imza varsa `ad(imza)`); default_acl → rol (şema kapsamında ` / şema`).
 */
export function defName(def: DesignerDef): string {
  if (!isObjectDef(def)) {
    if (!isAclDef(def)) return def.schema === null ? def.role : `${def.role} / ${def.schema}`;
    const base = def.schema === null ? def.name : `${def.schema}.${def.name}`;
    return def.signature === null ? base : `${base}(${def.signature})`;
  }
  if (!def.name) return "";
  switch (def.kind) {
    case "sequence":
    case "index":
    case "table":
    case "view":
    case "materialized_view":
    case "function":
    case "procedure":
    case "type":
    case "domain":
      return `${def.schema}.${def.name}`;
    case "trigger":
      return `${def.schema}.${def.table}.${def.name}`;
    default:
      return def.name;
  }
}

export function cloneDef<T extends DesignerDef>(def: T): T {
  return structuredClone(def);
}

/** Alan sırasından bağımsız kurallı JSON (nesne anahtarları sıralı; dizi sırası korunur). */
function canonical(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(canonical).join(",")}]`;
  if (v !== null && typeof v === "object") {
    const o = v as Record<string, unknown>;
    return `{${Object.keys(o).filter((k) => o[k] !== undefined).sort().map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`).join(",")}}`;
  }
  return JSON.stringify(v) ?? "null";
}
/** Taslak == yüklenen mi (kirli bayrağı). */
export function defsEqual(a: DesignerDef, b: DesignerDef): boolean {
  return canonical(a) === canonical(b);
}

// ---- Hata eşleme ----

/** `generalPaths[i]`: `general[i]` iletisinin özgün yolu (ör. üst düzey `columns`; bölüm rozeti için). */
export interface FieldIssues { byPath: Map<string, string[]>; general: string[]; generalPaths: string[] }

/** `path` bu kökün kendisi ya da altı mı (`columns` ⊃ `columns[1].key`; `name` ⊅ `nameX`). */
function under(path: string, root: string): boolean {
  return path === root || path.startsWith(root + ".") || path.startsWith(root + "[");
}

/** FieldError[] → alan yoluna göre gruplar; `old.` önekliler, üst-düzey `columns` ve (verilmişse) bilinmeyen yollar general'a düşer. */
export function groupIssues(errors: readonly FieldError[], knownPaths?: ReadonlySet<string>): FieldIssues {
  const byPath = new Map<string, string[]>();
  const general: string[] = [];
  const generalPaths: string[] = [];
  for (const e of errors) {
    const known = !knownPaths || [...knownPaths].some((k) => under(e.path, k));
    if (e.path.startsWith("old.") || e.path === "columns" || !known) {
      general.push(e.message);
      generalPaths.push(e.path);
      continue;
    }
    const list = byPath.get(e.path);
    if (list) list.push(e.message); else byPath.set(e.path, [e.message]);
  }
  return { byPath, general, generalPaths };
}

/** Tam yol + `path.`/`path[` altı (ör. `columns` → `columns[1].key` dahil); girdi sırası korunur. */
export function issuesFor(issues: FieldIssues, path: string): string[] {
  const out: string[] = [];
  for (const [p, msgs] of issues.byPath) if (under(p, path)) out.push(...msgs);
  return out;
}

// ---- Uygulama kapısı ----

export type ApplyGate =
  | { kind: "ready" }
  | { kind: "blocked"; reason: "locked" | "partial" | "invalid" | "empty" | "password" | "stale" | "pending" | "busy" | "disconnected" | "loading" }
  | { kind: "needsConfirm" }                                  // onay kutusu gerekli
  | { kind: "needsProductionName"; expected: string };        // onay kutusu + sunucu adı gerekli

export interface GateInput {
  locked: boolean;
  production: boolean;
  serverName: string;
  view: PlanView | null;
  /** `view` mevcut taslağın en son önizlemesi mi (hash/sürüm eşleşir). */
  viewIsCurrent: boolean;
  pending: boolean;
  applying: boolean;
  connected: boolean;
  /** Tanım yükleniyor ya da ilk yükleme hatayla bitti (sekme henüz düzenlenebilir değil; bağlantı kesik değil). */
  loading: boolean;
  /** Planda gizlenmiş (parola) ifade var ve denetleyicide geçerli parola yok (`planHasRedacted(view) && !secretReady`). */
  secretMissing: boolean;
  /** Kısmi create (`failed.created`): nesne oluştu, aynı create yeniden gönderilmez ("Düzenle olarak aç"). */
  partial?: boolean;
}

/**
 * Öncelik (ilk eşleşen kazanır): locked > disconnected > partial (kısmi create) > loading > busy > pending > stale (view yok dahil) > invalid > empty >
 * password (gizlenmiş planda parola yok) > risk.
 * Bayat/bekleyen görünümün hata ve boşluk durumu dikkate alınmaz; `invalid` `empty`den önce (geçersiz tanım empty=true da taşır).
 * Risk: safe → ready; destructive → needsConfirm; data_loss → üretimde needsProductionName, aksi halde needsConfirm.
 */
export function applyGate(i: GateInput): ApplyGate {
  if (i.locked) return { kind: "blocked", reason: "locked" };
  if (!i.connected) return { kind: "blocked", reason: "disconnected" };
  if (i.partial) return { kind: "blocked", reason: "partial" };
  if (i.loading) return { kind: "blocked", reason: "loading" };
  if (i.applying) return { kind: "blocked", reason: "busy" };
  if (i.pending) return { kind: "blocked", reason: "pending" };
  const v = i.view;
  if (!v || !i.viewIsCurrent) return { kind: "blocked", reason: "stale" };
  if (v.errors.length > 0) return { kind: "blocked", reason: "invalid" };
  if (v.empty) return { kind: "blocked", reason: "empty" };
  if (i.secretMissing) return { kind: "blocked", reason: "password" };
  if (v.risk === "data_loss" && i.production) return { kind: "needsProductionName", expected: i.serverName };
  if (v.risk === "destructive" || v.risk === "data_loss") return { kind: "needsConfirm" };
  return { kind: "ready" };
}

export interface ConfirmState { checked: boolean; typedName: string }

/** Yazılan ad: büyük/küçük harf duyarlı, kenar boşlukları kırpılmış tam eşleşme; boş beklenen ad asla eşleşmez. */
function nameMatches(expected: string, typed: string): boolean {
  return expected !== "" && typed.trim() === expected;
}

export function canSubmit(g: ApplyGate, c: ConfirmState): boolean {
  switch (g.kind) {
    case "ready": return true;
    case "needsConfirm": return c.checked;
    case "needsProductionName": return c.checked && nameMatches(g.expected, c.typedName);
    case "blocked": return false;
  }
}

/** `confirmedProduction` yalnızca needsProductionName kapısında, kutu işaretli ve ad eşleşirse true (canSubmit ile tutarlı). */
export function confirmFlags(g: ApplyGate, c: ConfirmState): { confirmed: boolean; confirmedProduction: boolean } {
  if (g.kind === "needsConfirm") return { confirmed: c.checked, confirmedProduction: false };
  if (g.kind === "needsProductionName") return { confirmed: c.checked, confirmedProduction: c.checked && nameMatches(g.expected, c.typedName) };
  return { confirmed: false, confirmedProduction: false };
}

/**
 * Gönderim seçenekleri; `!canSubmit(gate, c)` ise null (çağıran hiçbir şey göndermez — kapı UI'ya güvenmez). Parola
 * yalnızca planda gizlenmiş ifade varsa gider; aksi halde verilmiş olsa bile `null` (arka uç zaten reddeder).
 */
export function applyOptionsFor(
  applyId: string, lockTimeoutMs: number, view: PlanView, gate: ApplyGate, c: ConfirmState, password: string | null = null,
): ApplyOptions | null {
  if (!canSubmit(gate, c)) return null;
  return { applyId, lockTimeoutMs, expectedHash: view.hash, ...confirmFlags(gate, c), password: planHasRedacted(view) ? password : null };
}

// ---- Betik ----

/** Birleşik betik + her ifadenin [başlangıç, bitiş) aralığı (UTF-16 indeksi; `;` ve boş satır aralık dışı). */
export function scriptWithRanges(statements: readonly Statement[]): { text: string; ranges: Array<[number, number]> } {
  let text = "";
  const ranges: Array<[number, number]> = [];
  statements.forEach((s, i) => {
    if (i > 0) text += "\n\n";
    const start = text.length;
    text += s.sql;
    ranges.push([start, text.length]);
    text += ";";
  });
  return { text, ranges };
}

const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;
/** Sequence sayı alanları: /^[+-]?\d+$/ ve i64 aralığı (BigInt ile). */
export function isSafeIntegerString(s: string): boolean {
  if (!/^[+-]?[0-9]+$/.test(s)) return false;
  const n = BigInt(s);
  return n >= I64_MIN && n <= I64_MAX;
}

/**
 * İstemci tarafı yerel engel: tanım arka uca gönderilemez durumdaysa nedeni (aksi halde null). Sequence sayıları JSON'da
 * dizgedir ve arka uç çözümlemede (alan doğrulamasından önce) biçimsiz dizgeyi reddeder; bu durumda önizleme/uygulama
 * isteği hiç gönderilmez. Zorunlu sayılar (increment/start/cache) ve null olmayan min/max `isSafeIntegerString`
 * olmalıdır (boş dizge de biçimsiz; boş sınır formda `null`'dır). Diğer türler için engel yok.
 */
export type LocalBlock = "sequenceNumbers";
export function localBlock(def: DesignerDef | null): LocalBlock | null {
  if (!def || !isObjectDef(def) || def.kind !== "sequence") return null;
  const ok = [def.increment, def.start, def.cache].every(isSafeIntegerString)
    && [def.minValue, def.maxValue].every((v) => v === null || isSafeIntegerString(v));
  return ok ? null : "sequenceNumbers";
}

// ---- İndeks kolon listesi işlemleri (değişmez; sınır dışı indeks → aynı nesne) ----

const inRange = (i: number, len: number) => Number.isInteger(i) && i >= 0 && i < len;

export function addColumn(d: IndexDef, column: IndexColumn): IndexDef {
  return { ...d, columns: [...d.columns, column] };
}
export function removeColumn(d: IndexDef, i: number): IndexDef {
  if (!inRange(i, d.columns.length)) return d;
  return { ...d, columns: d.columns.filter((_, k) => k !== i) };
}
export function moveColumn(d: IndexDef, from: number, to: number): IndexDef {
  if (!inRange(from, d.columns.length) || !inRange(to, d.columns.length) || from === to) return d;
  const cols = [...d.columns];
  const [c] = cols.splice(from, 1);
  cols.splice(to, 0, c!);
  return { ...d, columns: cols };
}
export function updateColumn(d: IndexDef, i: number, patch: Partial<IndexColumn>): IndexDef {
  if (!inRange(i, d.columns.length)) return d;
  return { ...d, columns: d.columns.map((c, k) => (k === i ? { ...c, ...patch } : c)) };
}
export function addInclude(d: IndexDef, name: string): IndexDef {
  return { ...d, include: [...d.include, name] };
}
export function removeInclude(d: IndexDef, i: number): IndexDef {
  if (!inRange(i, d.include.length)) return d;
  return { ...d, include: d.include.filter((_, k) => k !== i) };
}

// ---- Önizleme görünümü (SqlPreview) ----

/**
 * Her ifadenin satır aralığı (1 tabanlı, kapsayıcı) — `scriptWithRanges` metni ve aralıkları üzerinden. Satır
 * dekorasyonu (başarısız ifade vurgusu) için; aralık metin dışındaysa metnin son satırına kırpılır.
 */
export function statementRangesToLineRanges(text: string, ranges: ReadonlyArray<readonly [number, number]>): Array<{ from: number; to: number }> {
  // Satır başlangıç ofsetleri: lineOf(i) = i'den küçük-eşit son başlangıcın sırası.
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) starts.push(i + 1);
  const lineOf = (offset: number) => {
    const o = Math.max(0, Math.min(offset, text.length));
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid]! <= o) lo = mid; else hi = mid - 1;
    }
    return lo + 1;
  };
  return ranges.map(([a, b]) => {
    const from = lineOf(a);
    // Bitiş dışlayıcı: son karakterin satırı (boş ifadede başlangıç satırı).
    return { from, to: Math.max(from, lineOf(b > a ? b - 1 : a)) };
  });
}

/**
 * Önizleme alanının boş durumu (null: betik gösterilir). Bekleyen önizlemede eski görünüm (varsa) gösterilmeye devam
 * eder; yalnızca hiç görünüm yokken "hazırlanıyor". Görünüm yokken: create → "formu tamamlayın", alter → "değişiklik
 * yok" (yüklenen tanım = taslak). Geçersiz tanım `empty = true` da taşır: önce `errors` bakılır.
 */
export type PreviewEmptyState = "loading" | "pending" | "incomplete" | "noChanges";
/**
 * Sekme durumu ve önizleme hatası görünüm yokken dikkate alınır: yüklenirken "yükleniyor"; hazır olmayan sekmede
 * (hata/bağlantı kesik/nesne yok) ya da önizleme hatası varken boş-durum metni yok (şeritler nedeni gösterir;
 * "Değişiklik yok" yanıltıcı olurdu).
 */
export function previewEmptyState(
  view: PlanView | null,
  pending: boolean,
  mode: "create" | "alter",
  status: "loading" | "ready" | "error" | "disconnected" | "gone" = "ready",
  previewError: string | null = null,
): PreviewEmptyState | null {
  if (!view) {
    if (status === "loading") return "loading";
    if (status !== "ready") return null;
    if (pending) return "pending";
    if (previewError !== null) return null;
    return mode === "create" ? "incomplete" : "noChanges";
  }
  if (view.errors.length > 0) return "incomplete";
  if (view.empty || view.statements.length === 0) return "noChanges";
  return null;
}

// ---- Sekme anahtarı ----

/** Designer sekmesinin hedefi: yeni nesne (`create`) ya da var olan nesne (`object`). */
export type DesignerTarget = { serverId: string; databaseOid: number | null } & (
  | { create: DdlCreate; object?: undefined }
  | { object: ObjectRef; create?: undefined }
);

/**
 * Sekme tekilleştirme anahtarı (`DesignerTab.key`). Alter: `alter:<kind>:<sunucu>:<db>:<oid>` — aynı nesne tek sekme
 * (sunucu kapsamlı türlerde `<db>` = `null`, ör. `alter:role:<sunucu>:null:<oid>`).
 * Create: `create:<kind>:<sunucu>:<db>:<şema>:<tablo>:<n>` — aynı bağlamda birden çok "yeni" sekmesi serbest; `n` açık
 * create sekmelerinin en büyüğünden bir fazladır (anahtar her açılışta benzersiz).
 */
export function designerTabKey(open: ReadonlyArray<{ key: string }>, t: DesignerTarget): string {
  if (t.object) return `alter:${t.object.kind}:${t.serverId}:${t.databaseOid}:${t.object.oid}`;
  const c = t.create;
  const prefix = `create:${c.kind}:${t.serverId}:${t.databaseOid}:${c.context.schemaOid ?? ""}:${c.context.tableOid ?? ""}:`;
  let n = 1;
  for (const d of open) {
    if (!d.key.startsWith(prefix)) continue;
    const k = Number(d.key.slice(prefix.length));
    if (Number.isInteger(k) && k >= n) n = k + 1;
  }
  return `${prefix}${n}`;
}

/** Plan 22: nesne yetkileri sekmesi anahtarı `acl:<kind>:<sunucu>:<db>:<oid>` (aynı nesne tek sekme). */
export function aclTabKey(kind: ObjectKind, serverId: string, databaseOid: number | null, oid: number): string {
  return `acl:${kind}:${serverId}:${databaseOid}:${oid}`;
}
/** Plan 22: varsayılan yetkiler sekmesi anahtarı `defacl:<sunucu>:<db>:<şemaOid|0>` (veritabanı geneli = 0). */
export function defaultAclTabKey(serverId: string, databaseOid: number, schemaOid: number | null): string {
  return `defacl:${serverId}:${databaseOid}:${schemaOid ?? 0}`;
}

/** Tablo diyaloğu odak alanı: `column:<attnum>` / `constraint:<oid>` (`requestFocus` alanı; `name` gibi düz alanlarla çakışmaz). */
export function focusFieldOf(f: DdlFocus): string {
  return `${f.kind}:${f.id}`;
}
export function parseFocusField(field: string): DdlFocus | null {
  const m = /^(column|constraint):([0-9]+)$/.exec(field);
  return m ? ({ kind: m[1] as DdlFocus["kind"], id: Number(m[2]) }) : null;
}
