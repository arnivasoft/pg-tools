// Şema karşılaştırma saf modeli (Plan 30): kategori/filtre/arama, seçim kümesi, seçici durumu, kapı gerekçeleri ve
// metin yardımcıları. DOM, IPC ve Solid yok; yalnızca `strings-diff` ve tür içe aktarılır (tek istisna
// `isProductionLabel`: başka tembel parçalar da aynı adı alır, ana pakete yeni dışa aktarma eklemez).
import { tsd } from "../../app/strings-diff";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import type { DiffLine } from "../../ipc/bindings/DiffLine";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import { isProductionLabel } from "../servers/server-form";

export type Category = "onlySource" | "onlyTarget" | "different" | "same" | "problem";
/** Gösterim sırası. */
export const CATEGORIES: readonly Category[] = ["onlySource", "onlyTarget", "different", "same", "problem"];

/** Her öğe tek kategoridedir: `problem` doluysa durumdan bağımsız "Sorunlu". */
export function categoryOf(i: DiffItem): Category {
  if (i.problem !== null) return "problem";
  switch (i.status) {
    case "only_source":
      return "onlySource";
    case "only_target":
      return "onlyTarget";
    case "different":
      return "different";
    default:
      return "same";
  }
}

export function defaultFilters(): Record<Category, boolean> {
  return { onlySource: true, onlyTarget: true, different: true, same: false, problem: true };
}

export function counts(items: readonly DiffItem[]): Record<Category, number> {
  const c: Record<Category, number> = { onlySource: 0, onlyTarget: 0, different: 0, same: 0, problem: 0 };
  for (const i of items) c[categoryOf(i)]++;
  return c;
}

const fold = (s: string) => s.toLocaleLowerCase("tr-TR");
export function foldSearch(q: string): string {
  return fold(q.trim());
}

export function kindLabel(kind: ObjectKind): string {
  return tsd.kind[kind] ?? kind;
}

export function matches(i: DiffItem, folded: string): boolean {
  if (folded === "") return true;
  const hay = [kindLabel(i.kind), i.table ?? "", i.name, i.signature ?? ""];
  return hay.some((h) => fold(h).includes(folded));
}

export function visibleItems(items: readonly DiffItem[], filters: Record<Category, boolean>, folded: string): DiffItem[] {
  return items.filter((i) => filters[categoryOf(i)] && matches(i, folded));
}

export function initialSelection(items: readonly DiffItem[]): ReadonlySet<string> {
  return new Set(items.filter((i) => i.defaultSelected).map((i) => i.key));
}

export function toggleKey(sel: ReadonlySet<string>, item: DiffItem): ReadonlySet<string> {
  if (!item.selectable) return sel;
  const next = new Set(sel);
  if (!next.delete(item.key)) next.add(item.key);
  return next;
}

/** Yalnız seçilebilir öğeleri ekler/çıkarır; verilmeyenlere dokunmaz. */
export function setMany(sel: ReadonlySet<string>, items: readonly DiffItem[], on: boolean): ReadonlySet<string> {
  const next = new Set(sel);
  for (const i of items) {
    if (!i.selectable) continue;
    if (on) next.add(i.key);
    else next.delete(i.key);
  }
  return next;
}

export function selectAllState(sel: ReadonlySet<string>, items: readonly DiffItem[]): "none" | "some" | "all" {
  const sels = items.filter((i) => i.selectable);
  const n = sels.filter((i) => sel.has(i.key)).length;
  if (n === 0) return "none";
  return n === sels.length ? "all" : "some";
}

export function selectionSummary(sel: ReadonlySet<string>, visible: readonly DiffItem[]): { selected: number; hidden: number } {
  const shown = visible.filter((i) => sel.has(i.key)).length;
  return { selected: sel.size, hidden: sel.size - shown };
}

/** Sonuç sırasıyla; sonuçta artık olmayan anahtar atılır. */
export function selectedKeys(sel: ReadonlySet<string>, items: readonly DiffItem[]): string[] {
  return items.filter((i) => sel.has(i.key)).map((i) => i.key);
}

export interface SideSel {
  serverId: string | null;
  databaseOid: number | null;
  schema: string | null;
}
export function emptySide(): SideSel {
  return { serverId: null, databaseOid: null, schema: null };
}
export function sideComplete(s: SideSel): s is { serverId: string; databaseOid: number; schema: string } {
  return s.serverId !== null && s.databaseOid !== null && s.schema !== null;
}
export function sameSide(a: SideSel, b: SideSel): boolean {
  return (
    sideComplete(a) && sideComplete(b) && a.serverId === b.serverId && a.databaseOid === b.databaseOid && a.schema === b.schema
  );
}
export function swapSides<T>(a: T, b: T): [T, T] {
  return [b, a];
}

export type Blocked = keyof typeof tsd.blocked;

/** `known`: sunucu kayıtlı mı (silinmiş sunucuyu tutan taraf eksik sayılır). */
export function compareBlocked(
  src: SideSel, tgt: SideSel, running: boolean, known: (serverId: string) => boolean = () => true,
): Blocked | null {
  if (running) return "running";
  if (!sideComplete(src) || !sideComplete(tgt) || !known(src.serverId) || !known(tgt.serverId)) return "incomplete";
  if (sameSide(src, tgt)) return "same";
  return null;
}

export function scriptBlocked(i: { running: boolean; hasResult: boolean; stale: boolean; selected: number }): Blocked | null {
  if (i.running) return "running";
  if (!i.hasResult) return "noResult";
  if (i.stale) return "stale";
  if (i.selected === 0) return "noSelection";
  return null;
}

/** Explorer girişi (Kararlar 4): şema düğümü etiketiyle, veritabanı düğümü oid ile; başka tür → null. */
export function entryFromTarget(t: DdlTarget, label: string): SideSel | null {
  const o = t.object;
  if (o === null) return null;
  if (o.kind === "schema") {
    if (t.databaseOid === null) return null;
    return { serverId: t.serverId, databaseOid: t.databaseOid, schema: label };
  }
  if (o.kind === "database") return { serverId: t.serverId, databaseOid: o.oid, schema: null };
  return null;
}

type Named = { db: string; schema: string };
export function tabTitle(src: Named, tgt: Named): string {
  return `${src.db}.${src.schema} ↔ ${tgt.db}.${tgt.schema}`;
}

const pad = (n: number) => String(n).padStart(2, "0");
// eslint-disable-next-line no-control-regex
const BAD_FILE_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g;
const MAX_NAME = 120;

export function suggestScriptName(src: Named, tgt: Named, now: Date): string {
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}`;
  const base = `${src.db}_${src.schema}__${tgt.db}_${tgt.schema}_${stamp}`.replace(BAD_FILE_CHARS, "_");
  // Kesimden sonra sondaki boşluk ve nokta atılır (Windows dosya adında bunları sessizce siler).
  return `${Array.from(base).slice(0, MAX_NAME - 4).join("").replace(/[\s.]+$/u, "")}.sql`;
}

export function progressView(p: { loaded: number; total: number } | null): { percent: number | null; text: string } {
  if (p === null) return { percent: null, text: "" };
  if (p.total <= 0) return { percent: null, text: `${p.loaded}` };
  return { percent: Math.min(100, Math.round((p.loaded / p.total) * 100)), text: `${p.loaded} / ${p.total}` };
}

export function itemLabel(i: DiffItem): string {
  if (i.table !== null) return `${i.table}.${i.name}`;
  if (i.signature !== null) return `${i.name}(${i.signature})`;
  return i.name;
}

export function problemText(i: DiffItem): string | null {
  return i.problem === null ? null : tsd.problem[i.problem];
}

export function errorText(p: ErrorPayload): string {
  if (p.code === "password_required") return `${p.message} ${tsd.passwordRequiredHint}`;
  const known = tsd.errors[p.code];
  if (known !== undefined) return known;
  return p.hint ? `${p.message} ${p.hint}` : p.message;
}

export function targetBanner(cfg: ServerConfig | null): { production: boolean; locked: boolean; name: string | null } {
  if (cfg === null) return { production: false, locked: false, name: null };
  return { production: isProductionLabel(cfg.label), locked: cfg.readOnlyLock, name: cfg.name };
}

export function gutter(l: DiffLine): { target: string; source: string; sign: "+" | "−" | " " } {
  return {
    target: l.targetNo === null ? "" : String(l.targetNo),
    source: l.sourceNo === null ? "" : String(l.sourceNo),
    sign: l.kind === "added" ? "+" : l.kind === "removed" ? "−" : " ",
  };
}
