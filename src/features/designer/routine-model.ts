// Function / Procedure formunun saf modeli (tembel parça): tür daraltma, alan güncelleyicileri ve form kuralları
// (yeniden oluşturma gereksinimi — spec §8.2 `needs_recreate` aynası —, dönüş geçişleri, uyarılar). SQL üretmez; gövde,
// varsayılan ve tip metinleri olduğu gibi gider.
import { trout } from "../../app/strings-routine";
import type { ArgMode } from "../../ipc/bindings/ArgMode";
import type { FunctionArg } from "../../ipc/bindings/FunctionArg";
import type { FunctionDef } from "../../ipc/bindings/FunctionDef";
import type { FunctionReturn } from "../../ipc/bindings/FunctionReturn";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ReturnColumn } from "../../ipc/bindings/ReturnColumn";
import type { FieldIssues } from "./designer-model";

export type RoutineKind = "function" | "procedure";
export type RoutineDraft = FunctionDef & { kind: RoutineKind };
/** Formdaki dönüş seçimi (`type` + `setof` iki ayrı seçenektir). */
export type ReturnChoice = "type" | "setof" | "table" | "trigger" | "void";

export function asRoutine(d: ObjectDef | null): RoutineDraft | null {
  return d && (d.kind === "function" || d.kind === "procedure") ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind` korunur). Tür uyuşmazlığında aynı nesne. */
export function routinePatch(d: ObjectDef, patch: Partial<FunctionDef>): ObjectDef {
  return d.kind === "function" || d.kind === "procedure" ? { ...d, ...patch, kind: d.kind } : d;
}

const isInput = (a: FunctionArg) => a.mode !== "out";
const isOutput = (a: FunctionArg) => a.mode === "out" || a.mode === "inout";
/** Karar 2: istemcide yalnız metin karşılaştırması (`int` ↔ `int4` eşdeğerliği arka ucun işidir). */
const sameType = (a: string, b: string) => a === b;

function sameReturn(a: FunctionReturn | null, b: FunctionReturn | null): boolean {
  if (a === null || b === null) return a === b;
  if (a.kind === "type") return b.kind === "type" && a.setof === b.setof && sameType(a.typeName, b.typeName);
  if (a.kind === "table") {
    return (
      b.kind === "table" &&
      a.columns.length === b.columns.length &&
      a.columns.every((c, i) => c.name === b.columns[i]!.name && sameType(c.typeName, b.columns[i]!.typeName))
    );
  }
  return a.kind === b.kind;
}

/**
 * `OR REPLACE`'in yapamadığı değişikliklerin alan yolları (spec §8.2 / Rust `needs_recreate` aynası; sıra ve tekrar
 * kuralları aynı). Create (`old = null`) → []. Yol indeksleri taslaktaki (`d.args`) sıradır.
 */
export function recreateNeeds(old: FunctionDef | null, d: FunctionDef): string[] {
  if (old === null) return [];
  const e: string[] = [];
  const oin = old.args.filter(isInput);
  const nin = d.args.map((a, i) => [i, a] as const).filter(([, a]) => isInput(a));
  if (oin.length !== nin.length) {
    e.push("args");
  } else {
    oin.forEach((o, k) => {
      const [i, n] = nin[k]!;
      if (!sameType(o.typeName, n.typeName)) e.push(`args[${i}].typeName`);
      else if (o.name !== null && o.name !== n.name) e.push(`args[${i}].name`);
      if (o.default !== null && n.default === null) e.push(`args[${i}].default`);
    });
  }
  // Çıkış listesi (OUT/INOUT): sayı, mod, ad, tip. İki tarafta INOUT olup girişte zaten raporlanan satır `args` üretmez.
  const oo = old.args.filter(isOutput);
  const no = d.args.map((a, i) => [i, a] as const).filter(([, a]) => isOutput(a));
  const reported = (i: number) => e.includes(`args[${i}].name`) || e.includes(`args[${i}].typeName`);
  const outsChanged =
    oo.length !== no.length ||
    oo.some((o, k) => {
      const [i, n] = no[k]!;
      const same = o.mode === n.mode && o.name === n.name && sameType(o.typeName, n.typeName);
      return !same && !(o.mode === "inout" && n.mode === "inout" && reported(i));
    });
  if (outsChanged && !e.includes("args")) e.push("args");
  if (!sameReturn(old.returns, d.returns)) e.push("returns");
  return e;
}

/** Gereksinim kalmadıysa `recreate` false'a çekilir (kutu gizlenir, bayrak sızmaz); aksi halde aynı nesne. */
export function syncRecreate<T extends FunctionDef>(old: FunctionDef | null, d: T): T {
  return d.recreate && recreateNeeds(old, d).length === 0 ? { ...d, recreate: false } : d;
}

export function returnKindOf(r: FunctionReturn | null): ReturnChoice | null {
  if (r === null) return null;
  return r.kind === "type" ? (r.setof ? "setof" : "type") : r.kind;
}

/** Tip / SETOF tip dönüşünün tip adı (diğerlerinde ""). */
export function returnTypeName(r: FunctionReturn | null): string {
  return r?.kind === "type" ? r.typeName : "";
}
/** TABLE dönüşünün kolonları (diğerlerinde boş). */
export function returnColumns(r: FunctionReturn | null): ReturnColumn[] {
  return r?.kind === "table" ? r.columns : [];
}

/** ROWS yalnız küme dönen (SETOF / TABLE) fonksiyonda. */
export function rowsVisible(d: FunctionDef): boolean {
  const k = returnKindOf(d.returns);
  return k === "setof" || k === "table";
}

/**
 * Dönüş seçimi değişimi: tip ↔ SETOF tip adı taşınır, TABLE önceki kolonları korur; argümanlar her zaman korunur
 * (trigger'da argüman arka uç hatasıdır, TABLE + OUT/INOUT `tableOutArgsWarning`). Küme dönmeyen türe geçişte ROWS
 * temizlenir (yalnız küme dönende geçerli; gizli alanda hata kalmasın).
 */
export function withReturns(d: FunctionDef, kind: ReturnChoice): FunctionDef {
  const prev = d.returns;
  let returns: FunctionReturn;
  if (kind === "type" || kind === "setof") {
    returns = { kind: "type", typeName: prev?.kind === "type" ? prev.typeName : "", setof: kind === "setof" };
  } else if (kind === "table") {
    returns = { kind: "table", columns: prev?.kind === "table" ? prev.columns : [] };
  } else {
    returns = { kind };
  }
  const next = { ...d, returns };
  return rowsVisible(next) ? next : { ...next, rows: null };
}

/** RETURNS TABLE ile OUT/INOUT argüman birlikte (PG reddeder): uyarı. */
export function tableOutArgsWarning(d: FunctionDef): boolean {
  return d.returns?.kind === "table" && d.args.some(isOutput);
}

/** Argüman modları (procedure'de OUT PG 14+; ipucuyla sunulur, gizlenmez — Karar 7). */
export function argModes(_kind: RoutineKind): ArgMode[] {
  return ["in", "out", "inout", "variadic"];
}

/** SECURITY DEFINER yeni açıldı (false → true; create'te açık) → uyarı. */
export function securityDefinerWarning(old: FunctionDef | null, d: FunctionDef): boolean {
  return d.securityDefiner && !(old?.securityDefiner ?? false);
}

/** Yeniden oluşturmada kaybolacaklar. */
export function recreateLoss(): string[] {
  return [trout.loss.grants, trout.loss.dependents];
}

/** Yeniden oluşturma gereksiniminin okunur adı (kayıp listesinin üstünde). */
export function recreateNeedLabel(path: string): string {
  const m = /^args\[(\d+)\]\.(typeName|name|default)$/.exec(path);
  if (m) return trout.recreate.paths[m[2] as "typeName" | "name" | "default"](Number(m[1]));
  return path === "returns" ? trout.recreate.paths.returns : trout.recreate.paths.args;
}

const RECREATE_MARK = "yeniden oluşturmayı gerektirir";
const under = (p: string, root: string) => p === root || p.startsWith(`${root}[`) || p.startsWith(`${root}.`);

/** Arka uç `args*` / `returns*` yolunda "…yeniden oluşturmayı gerektirir…" alan hatası verdi mi (kutuya odak). */
export function hasRecreateError(issues: FieldIssues): boolean {
  for (const [p, msgs] of issues.byPath) {
    if ((under(p, "args") || under(p, "returns")) && msgs.some((m) => m.includes(RECREATE_MARK))) return true;
  }
  return false;
}

/** Dil seçenekleri: mevcut dil listede yoksa başa (salt okunur/eski dil seçimi kaybolmasın). */
export function languageChoices(items: readonly LookupItem[], current: string): LookupItem[] {
  if (current === "" || items.some((i) => i.value === current)) return [...items];
  return [{ value: current, detail: null }, ...items];
}

export function newArg(): FunctionArg {
  return { mode: "in", name: null, typeName: "", default: null };
}
export function newReturnColumn(): ReturnColumn {
  return { name: "", typeName: "" };
}

/** Listede bir satırı günceller (değişmez; sınır dışı → aynı dizi). */
export function updateAt<T>(list: T[], i: number, patch: Partial<T>): T[] {
  if (!Number.isInteger(i) || i < 0 || i >= list.length) return list;
  return list.map((x, k) => (k === i ? { ...x, ...patch } : x));
}
