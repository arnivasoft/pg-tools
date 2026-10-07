// Veri düzenleyici hata eşlemesi (saf). Davranış yalnızca `ErrorPayload.code` / `sqlstate` ile seçilir (metne
// bakılmaz). designer/errors.ts kalıbı; o modül veri parçasından içe aktarılmaz (Plan 17 Karar 2).
// Üretilen metinler hücre/anahtar/sürüm değeri içermez; PG'nin kendi iletisi yalnızca olduğu gibi aktarılır.
import { tdata } from "../../app/strings-data";
import type { DataOutcome } from "../../ipc/bindings/DataOutcome";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { RowChange } from "../../ipc/bindings/RowChange";
import { errorText } from "../../ipc/errors";

export type DataErrorClass =
  | "disconnected"
  | "gone"
  | "locked"
  | "plan_changed"
  | "confirmation_required"
  | "invalid_def"
  | "other";

const CLASSES: Record<string, DataErrorClass> = {
  not_connected: "disconnected",
  connection_lost: "disconnected",
  unknown_server: "gone",
  not_found: "gone",
  locked: "locked",
  plan_changed: "plan_changed",
  confirmation_required: "confirmation_required",
  production_confirmation_required: "confirmation_required",
  invalid_def: "invalid_def",
};

export function classifyDataError(p: ErrorPayload): DataErrorClass {
  return CLASSES[p.code] ?? "other";
}

/** Bağlantı kodları (`not_connected`, `connection_lost`, `unknown_server`). */
export function isConnectionCode(code: string): boolean {
  return code === "not_connected" || code === "connection_lost" || code === "unknown_server";
}

export type DataFailed = Extract<DataOutcome, { outcome: "failed" }>;
export type ChangeKind = "insert" | "update" | "delete";

/**
 * `Failed` sonucunun metni. `outcomeUnknown` önce (kalıcılaşmış olabilir); çakışmanın anlamı başarısız değişikliğin
 * türüne göre (INSERT → düşürüldü; UPDATE/DELETE → değiştirilmiş/silinmiş); aksi halde PG iletisi aynen
 * (`55P03` için satır kilidi ipucu).
 */
export function outcomeText(
  out: DataFailed,
  ctx: { changeKind: ChangeKind | null },
): { text: string; kind: "conflict" | "unknown" | "failed"; hint?: string } {
  if (out.outcomeUnknown) return { kind: "unknown", text: tdata.outcome.unknown };
  if (out.conflict) {
    const text = ctx.changeKind === "insert" ? tdata.conflict.insert : tdata.conflict.updateDelete;
    return { kind: "conflict", text, hint: tdata.conflict.reloadHint };
  }
  const text = errorText(out.error);
  return out.sqlstate === "55P03" ? { kind: "failed", text, hint: tdata.outcome.rowLocked } : { kind: "failed", text };
}

export interface FieldIssues {
  /** Anahtar: `${changeIndex}:${columnName}`. */
  cell: Map<string, string[]>;
  /** Anahtar: değişiklik indeksi (`changes[i]`). */
  row: Map<number, string[]>;
  general: string[];
}

const CHANGE_PATH = /^changes\[(\d+)\](?:\.(set|values)\[(\d+)\]|\.key(?:\[\d+\])?|\.version)?$/;

function push<K>(m: Map<K, string[]>, k: K, msg: string) {
  const list = m.get(k);
  if (list) list.push(msg);
  else m.set(k, [msg]);
}

/**
 * `data_preview` alan hatalarını (`PlanView.errors`) gönderilen değişiklik listesine göre yerleştirir:
 * `changes[i].set[j]` / `changes[i].values[j]` → i. değişikliğin j. düzenlemesinin kolonu (çözülemezse satır);
 * `changes[i]`, `.key`, `.key[j]`, `.version` → satır; `object`, `changes`, aralık dışı ve bilinmeyen → genel.
 */
export function fieldIssues(errors: readonly FieldError[], changeSet: { changes: readonly RowChange[] }): FieldIssues {
  const r: FieldIssues = { cell: new Map(), row: new Map(), general: [] };
  for (const e of errors) {
    const m = CHANGE_PATH.exec(e.path);
    const i = m ? Number(m[1]) : -1;
    const change = m ? changeSet.changes[i] : undefined;
    if (!change) {
      r.general.push(e.message);
      continue;
    }
    if (m?.[2]) {
      const edits = m[2] === "set" ? (change.op === "update" ? change.set : null) : change.op === "insert" ? change.values : null;
      const edit = edits?.[Number(m[3])];
      if (edit) {
        push(r.cell, `${i}:${edit.column}`, e.message);
        continue;
      }
    }
    push(r.row, i, e.message);
  }
  return r;
}
