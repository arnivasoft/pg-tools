// Silme diyaloğunun saf yardımcıları: tek `Drop` isteği tanımı, hata/sonuç metni ve sekme–nesne eşleşmesi.
// Davranış yalnızca `ErrorPayload.code` ve `Failed.sqlstate` ile seçilir (metne asla bakılmaz).
import { tdes } from "../../app/strings-designer";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import { errorText } from "../../ipc/errors";
import { cloneDef } from "./designer-model";
import { classifyError, designerErrorText, failedText, isConnectionCode } from "./errors";

/**
 * `Drop` isteğinin TEK tanımı (designer `planRequest(m, "drop")` de bunu kullanır): `def` `ddl_load`'un döndürdüğü
 * tanımdır ve olduğu gibi (derin kopya) gider; `object` de kopyalanır (istek modelle nesne paylaşmaz).
 */
export function dropRequest(object: ObjectRef, def: ObjectDef, cascade: boolean): Extract<PlanRequest, { op: "drop" }> {
  return { op: "drop", object: { ...object }, def: cloneDef(def), cascade };
}

/** PostgreSQL `dependent_objects_still_exist`: CASCADE olmadan silinemeyen nesne. */
export const DEPENDENT_OBJECTS_SQLSTATE = "2BP01";

/**
 * Silme hatasının diyalogdaki karşılığı:
 * - `dependents`: bağımlı nesneler var (`Failed.sqlstate === 2BP01`); `detail` PG'nin bağımlı nesne listesidir;
 * - `stale`: nesne değişti/yok (`plan_changed`, `not_found`) → Gezgin yenilenmeli, silme yapılamaz;
 * - `disconnected`: bağlantı kodları → diyalog bildirimle kapanır;
 * - `locked`: sunucu salt okunur kilitli;
 * - `other`: diğer her şey (`invalid_def`, onay kodları, SQL hataları; `Failed` için `failedText`).
 */
export type DropStatusKind = "dependents" | "stale" | "disconnected" | "locked" | "other";
export interface DropStatus { kind: DropStatusKind; text: string; detail: string | null }

type Failed = Extract<ApplyOutcome, { outcome: "failed" }>;

/** `ddl_*` reddi (`error`) ya da `Ok(Failed)` (`outcome`) → diyalog şeridi. İkisi de yoksa null. */
export function dropStatusText(error: ErrorPayload | null, outcome: Failed | null = null): DropStatus | null {
  if (outcome) {
    if (outcome.sqlstate === DEPENDENT_OBJECTS_SQLSTATE) {
      // PG `detail` bağımlı nesneleri satır satır listeler; yoksa PG iletisi gösterilir.
      return { kind: "dependents", text: tdes.drop.dependents, detail: outcome.error.detail ?? errorText(outcome.error) };
    }
    const partial = outcome.committed > 0 ? tdes.apply.committed(outcome.committed) : null;
    return { kind: partial ? "stale" : "other", text: failedText(outcome), detail: partial };
  }
  if (!error) return null;
  if (isConnectionCode(error.code)) return { kind: "disconnected", text: errorText(error), detail: null };
  switch (classifyError(error)) {
    case "planChanged":
    case "gone":
      return { kind: "stale", text: tdes.drop.stale, detail: null };
    case "locked":
      return { kind: "locked", text: tdes.gate.locked, detail: null };
    default:
      return { kind: "other", text: designerErrorText(error), detail: null };
  }
}

/** Designer sekmesi bu nesnenin alter sekmesi mi (aynı sunucu, veritabanı, tür ve oid). */
export function matchesObject(
  tab: { serverId: string; databaseOid: number; mode: "create" | "alter"; object: ObjectRef | null },
  ref: { serverId: string; databaseOid: number; object: ObjectRef },
): boolean {
  return tab.mode === "alter" && tab.object !== null && tab.serverId === ref.serverId && tab.databaseOid === ref.databaseOid
    && tab.object.kind === ref.object.kind && tab.object.oid === ref.object.oid;
}
