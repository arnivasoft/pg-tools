import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SslMode } from "../../ipc/bindings/SslMode";

export interface ServerForm {
  name: string;
  group: string;
  host: string;
  port: string;
  maintenanceDb: string;
  username: string;
  password: string;
  savePassword: boolean;
  sslMode: SslMode;
  rootCert: string;
  clientCert: string;
  clientKey: string;
  color: string | null;
  label: string;
  readOnlyLock: boolean;
}

export type FormTab = "general" | "connection" | "ssl" | "security";

export const SERVER_COLORS = ["red", "orange", "amber", "green", "teal", "blue", "purple", "gray"] as const;

export type FormResult =
  | { ok: true; config: ServerConfig; password: string | null }
  | { ok: false; error: string; tab: FormTab };

export function emptyForm(): ServerForm {
  return {
    name: "", group: "", host: "", port: "5432", maintenanceDb: "postgres", username: "", password: "",
    savePassword: false, sslMode: "prefer", rootCert: "", clientCert: "", clientKey: "",
    color: null, label: "", readOnlyLock: false,
  };
}

export function formFromConfig(c: ServerConfig): ServerForm {
  return {
    name: c.name, group: c.group ?? "", host: c.host, port: String(c.port), maintenanceDb: c.maintenanceDb,
    username: c.username, password: "", savePassword: c.savePassword, sslMode: c.ssl.mode,
    rootCert: c.ssl.rootCert ?? "", clientCert: c.ssl.clientCert ?? "", clientKey: c.ssl.clientKey ?? "",
    color: c.color, label: c.label ?? "", readOnlyLock: c.readOnlyLock,
  };
}

const opt = (v: string): string | null => (v.trim() === "" ? null : v.trim());

export function toConfig(f: ServerForm, id: string): FormResult {
  const fail = (error: string, tab: FormTab): FormResult => ({ ok: false, error, tab });
  if (!f.name.trim()) return fail("Sunucu adı boş olamaz.", "general");
  if (!f.host.trim()) return fail("Host boş olamaz.", "connection");
  const port = Number(f.port.trim());
  if (!/^\d+$/.test(f.port.trim()) || port < 1 || port > 65535) return fail("Port 1-65535 arasında olmalı.", "connection");
  if (!f.maintenanceDb.trim()) return fail("Bakım veritabanı boş olamaz.", "connection");
  if (!f.username.trim()) return fail("Kullanıcı adı boş olamaz.", "connection");
  if ((opt(f.clientCert) === null) !== (opt(f.clientKey) === null)) {
    return fail("İstemci sertifikası ve anahtarı birlikte verilmelidir.", "ssl");
  }
  return {
    ok: true,
    password: f.password === "" ? null : f.password,
    config: {
      id,
      name: f.name.trim(),
      group: opt(f.group),
      host: f.host.trim(),
      port,
      maintenanceDb: f.maintenanceDb.trim(),
      username: f.username.trim(),
      savePassword: f.savePassword,
      ssl: { mode: f.sslMode, rootCert: opt(f.rootCert), clientCert: opt(f.clientCert), clientKey: opt(f.clientKey) },
      color: f.color,
      label: opt(f.label),
      readOnlyLock: f.readOnlyLock,
    },
  };
}

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
