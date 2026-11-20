// Tablespace formunun saf modeli (tembel parça; Solid içe aktarmaz): seçenek metni yazımı ve engellemeyen biçim
// uyarıları (Karar 7). SQL üretmez; asıl doğrulama arka uç alan hatasıdır.
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TablespaceDef } from "../../ipc/bindings/TablespaceDef";
import type { TablespaceOptions } from "../../ipc/bindings/TablespaceOptions";

export function asTablespace(d: ObjectDef | null): (TablespaceDef & { kind: "tablespace" }) | null {
  return d && d.kind === "tablespace" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind` korunur). Tür uyuşmazlığında aynı nesne. */
export function tablespacePatch(d: ObjectDef, patch: Partial<TablespaceDef>): ObjectDef {
  return d.kind === "tablespace" ? { ...d, ...patch, kind: "tablespace" } : d;
}

export type TablespaceOptionKey = keyof TablespaceOptions;

/** Metni kırpar; boş → null. Diğer seçenekler değişmez (yeni nesne). */
export function tablespaceOptionPatch(d: TablespaceDef, key: TablespaceOptionKey, text: string): TablespaceDef {
  const t = text.trim();
  return { ...d, options: { ...d.options, [key]: t === "" ? null : t } };
}

/** Karar 7 — engellemeyen biçim uyarıları; boş/null alan uyarı üretmez. */
export type OptionWarning = "notNumber" | "negative" | "notInteger" | "outOfRange";

const COST_KEYS = ["seqPageCost", "randomPageCost"] as const;
const IO_KEYS = ["effectiveIoConcurrency", "maintenanceIoConcurrency"] as const;

export function tablespaceOptionWarnings(o: TablespaceOptions): Partial<Record<TablespaceOptionKey, OptionWarning>> {
  const out: Partial<Record<TablespaceOptionKey, OptionWarning>> = {};
  for (const k of COST_KEYS) {
    const v = o[k]?.trim();
    if (!v) continue;
    const n = Number(v);
    if (!Number.isFinite(n)) out[k] = "notNumber";
    else if (n < 0) out[k] = "negative";
  }
  for (const k of IO_KEYS) {
    const v = o[k]?.trim();
    if (!v) continue;
    if (/^\d+$/.test(v)) {
      if (Number(v) > 1000) out[k] = "outOfRange";
    } else if (v.startsWith("-")) out[k] = "negative";
    else out[k] = Number.isFinite(Number(v)) ? "notInteger" : "notNumber";
  }
  return out;
}
