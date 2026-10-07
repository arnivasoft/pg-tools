// Nesne diyaloglarının saf modeli: hata eşleme, uygulama kapısı (güvenlik), betik aralıkları, indeks kolon işlemleri.
// İstemci SQL üretmez; yalnızca arka ucun döndürdüğü PlanView'i yorumlar.
import type { ApplyOptions } from "../../ipc/bindings/ApplyOptions";
import type { DdlCreate } from "../../ipc/bindings/DdlCreate";
import type { DdlFocus } from "../../ipc/bindings/DdlFocus";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { IndexColumn } from "../../ipc/bindings/IndexColumn";
import type { IndexDef } from "../../ipc/bindings/IndexDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Statement } from "../../ipc/bindings/Statement";

export type DesignerKind = ObjectKind;

const KIND_LABELS: Record<ObjectKind, string> = { schema: "Schema", sequence: "Sequence", extension: "Extension", index: "Index", table: "Table" };
export function kindLabel(kind: ObjectKind): string {
  return KIND_LABELS[kind];
}

/** Başlıkta/mesajlarda görünen ad; sequence ve indeks şema nitelikli, ad boşsa boş. */
export function defName(def: ObjectDef): string {
  if (!def.name) return "";
  return def.kind === "sequence" || def.kind === "index" || def.kind === "table" ? `${def.schema}.${def.name}` : def.name;
}

export function cloneDef<T extends ObjectDef>(def: T): T {
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
export function defsEqual(a: ObjectDef, b: ObjectDef): boolean {
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
  | { kind: "blocked"; reason: "locked" | "invalid" | "empty" | "stale" | "pending" | "busy" | "disconnected" | "loading" }
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
}

/**
 * Öncelik (ilk eşleşen kazanır): locked > disconnected > loading > busy > pending > stale (view yok dahil) > invalid > empty > risk.
 * Bayat/bekleyen görünümün hata ve boşluk durumu dikkate alınmaz; `invalid` `empty`den önce (geçersiz tanım empty=true da taşır).
 * Risk: safe → ready; destructive → needsConfirm; data_loss → üretimde needsProductionName, aksi halde needsConfirm.
 */
export function applyGate(i: GateInput): ApplyGate {
  if (i.locked) return { kind: "blocked", reason: "locked" };
  if (!i.connected) return { kind: "blocked", reason: "disconnected" };
  if (i.loading) return { kind: "blocked", reason: "loading" };
  if (i.applying) return { kind: "blocked", reason: "busy" };
  if (i.pending) return { kind: "blocked", reason: "pending" };
  const v = i.view;
  if (!v || !i.viewIsCurrent) return { kind: "blocked", reason: "stale" };
  if (v.errors.length > 0) return { kind: "blocked", reason: "invalid" };
  if (v.empty) return { kind: "blocked", reason: "empty" };
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

/** Gönderim seçenekleri; `!canSubmit(gate, c)` ise null (çağıran hiçbir şey göndermez — kapı UI'ya güvenmez). */
export function applyOptionsFor(applyId: string, lockTimeoutMs: number, view: PlanView, gate: ApplyGate, c: ConfirmState): ApplyOptions | null {
  if (!canSubmit(gate, c)) return null;
  return { applyId, lockTimeoutMs, expectedHash: view.hash, ...confirmFlags(gate, c) };
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
export function localBlock(def: ObjectDef | null): LocalBlock | null {
  if (!def || def.kind !== "sequence") return null;
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
export type DesignerTarget = { serverId: string; databaseOid: number } & (
  | { create: DdlCreate; object?: undefined }
  | { object: ObjectRef; create?: undefined }
);

/**
 * Sekme tekilleştirme anahtarı (`DesignerTab.key`). Alter: `alter:<kind>:<sunucu>:<db>:<oid>` — aynı nesne tek sekme.
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

/** Tablo diyaloğu odak alanı: `column:<attnum>` / `constraint:<oid>` (`requestFocus` alanı; `name` gibi düz alanlarla çakışmaz). */
export function focusFieldOf(f: DdlFocus): string {
  return `${f.kind}:${f.id}`;
}
export function parseFocusField(field: string): DdlFocus | null {
  const m = /^(column|constraint):([0-9]+)$/.exec(field);
  return m ? ({ kind: m[1] as DdlFocus["kind"], id: Number(m[2]) }) : null;
}
