// Nesne diyaloğu hata eşlemesi: davranış yalnızca `ErrorPayload.code` ile seçilir (metne asla bakılmaz).
import { tdes } from "../../app/strings-designer";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { errorText } from "../../ipc/errors";

/**
 * Kodun designer sekmesindeki karşılığı:
 * - `disconnected`: bağlantı yok/koptu → sekme "bağlantı kesildi" salt okunur;
 * - `gone`: sunucu kaydı (`unknown_server`), nesne (`not_found`) ya da rol (`role_not_found`; varsayılan yetkilerde
 *   denetleyici kapsamın rolü için ayrıca ele alır) yok;
 * - `planChanged`: nesne başka yerde değişti → yeniden yükleme önerisi;
 * - `invalidDef`: tanım geçersiz → önizleme yenilenir;
 * - `locked`: sunucu salt okunur kilitli;
 * - `confirmation`: onay eksik (denetleyici kapısı normalde engeller; savunma);
 * - `passwordMissing`: gizlenmiş (parola) ifadeli planda parola gelmedi (`role_password_missing`; kapı normalde engeller);
 * - `other`: `errorText` ile gösterilir.
 */
export type ErrorClass = "disconnected" | "gone" | "planChanged" | "invalidDef" | "locked" | "confirmation" | "passwordMissing" | "other";

const CLASSES: Record<string, ErrorClass> = {
  not_connected: "disconnected",
  connection_lost: "disconnected",
  unknown_server: "gone",
  not_found: "gone",
  role_not_found: "gone",
  plan_changed: "planChanged",
  invalid_def: "invalidDef",
  locked: "locked",
  confirmation_required: "confirmation",
  production_confirmation_required: "confirmation",
  role_password_missing: "passwordMissing",
};

export function classifyError(p: ErrorPayload): ErrorClass {
  return CLASSES[p.code] ?? "other";
}

/** Bağlantı kodları (`not_connected`, `connection_lost`, `unknown_server`). */
export function isConnectionCode(code: string): boolean {
  return code === "not_connected" || code === "connection_lost" || code === "unknown_server";
}

/** Uygulama kapısı kodları (`ddl_apply` reddi: plan uygulanmadı). */
export function isGateCode(code: string): boolean {
  return code === "locked" || code === "confirmation_required" || code === "production_confirmation_required"
    || code === "plan_changed" || code === "invalid_def" || code === "role_password_missing";
}

/** Kullanıcı metni: `message — detail — hint` (SQL hataları); kapı kodları sabit metinle. */
export function designerErrorText(p: ErrorPayload): string {
  switch (classifyError(p)) {
    case "planChanged": return tdes.gate.planChanged;
    case "invalidDef": return tdes.gate.invalidDef;
    case "locked": return tdes.gate.locked;
    case "confirmation": return tdes.gate.confirmationRequired;
    case "passwordMissing": return tdes.gate.passwordMissing;
    default: return errorText(p);
  }
}

/** `Failed` sonucunun metni: `55P03` (lock_not_available) → kilit zaman aşımı metni, aksi halde arka uç hatası. */
export function failedText(o: Extract<ApplyOutcome, { outcome: "failed" }>): string {
  const text = o.sqlstate === "55P03" ? tdes.apply.lockTimeout : errorText(o.error);
  return tdes.apply.failed(o.failedIndex, text);
}
