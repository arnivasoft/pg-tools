import type { ServerConfig } from "../../ipc/bindings/ServerConfig";

export function groupsOf(servers: ServerConfig[]): string[] {
  const set = new Set(servers.map((s) => s.group?.trim()).filter((g): g is string => !!g));
  return [...set].sort((a, b) => a.localeCompare(b, "tr"));
}

// Rust `char::is_whitespace` (Unicode White_Space) ile aynı küme; JS `trim()` U+FEFF'i de keser, Rust kesmez.
const RUST_WS = "\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000";
const TRIM_RE = new RegExp(`^[${RUST_WS}]+|[${RUST_WS}]+$`, "gu");

/** Rust `ServerConfig::is_production` ile birebir: kırp, U+0307'yi at, İ/ı → i, küçült, U+0307'yi tekrar at. */
export function isProductionLabel(label: string | null | undefined): boolean {
  if (label == null) return false;
  const folded = Array.from(label.replace(TRIM_RE, ""))
    .filter((ch) => ch !== "\u0307")
    .map((ch) => (ch === "İ" || ch === "ı" ? "i" : ch))
    .join("")
    .toLowerCase()
    .replace(/\u0307/g, "");
  return folded === "üretim" || folded === "uretim" || folded === "production" || folded === "prod";
}

/**
 * Etiket üretim değerine dönüştüğünde kilidi bir kez otomatik işaretler. Kullanıcı kilidi elle değiştirdiyse
 * (`userTouchedLock`) ya da etiket zaten üretimse (ör. var olan sunucu) dokunmaz.
 */
export function suggestLock(prevLabel: string, nextLabel: string, lock: boolean, userTouchedLock: boolean): boolean {
  if (lock || userTouchedLock) return lock;
  return !isProductionLabel(prevLabel) && isProductionLabel(nextLabel) ? true : lock;
}
