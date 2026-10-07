// Role formunun saf modeli (tembel parça): tür daraltma, ayrıcalık uyarısı, parola niyeti seçenekleri ve iki girdinin
// eşleşme kuralı, üyelik listesi, VALID UNTIL dönüşümü ve kendini kilitleme uyarıları. SQL üretmez. Parola DEĞERİ bu
// modülde yoktur: yalnızca niyet (`PasswordChange`) ve çağıranın verdiği iki metnin karşılaştırması (saklanmaz).
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PasswordChange } from "../../ipc/bindings/PasswordChange";
import type { RoleDef } from "../../ipc/bindings/RoleDef";
import type { RoleMembership } from "../../ipc/bindings/RoleMembership";

export type RoleDraft = RoleDef & { kind: "role" };

export function asRole(d: ObjectDef | null): RoleDraft | null {
  return d && d.kind === "role" ? d : null;
}

export function roleEditPatch(d: ObjectDef, patch: Partial<RoleDef>): ObjectDef {
  return d.kind === "role" ? { ...d, ...patch } : d;
}

export const ROLE_FLAGS = ["login", "superuser", "createdb", "createrole", "inherit", "replication", "bypassrls"] as const;
export type RoleFlag = (typeof ROLE_FLAGS)[number];

const PRIVILEGED: ReadonlySet<RoleFlag> = new Set<RoleFlag>(["superuser", "replication", "bypassrls", "createrole"]);

/** Yeni verilen ayrıcalıklı öznitelikler (uyarı): superuser, replication, bypassrls, createrole; create'te true olanlar, alter'da false→true. */
export function privilegedGrants(original: RoleDef | null, d: RoleDef): RoleFlag[] {
  return ROLE_FLAGS.filter((f) => PRIVILEGED.has(f) && d[f] && !(original?.[f] ?? false));
}

/** Parola niyeti seçenekleri: create → ["keep", "set"] ("keep" = parolasız); alter → ["keep", "set", "clear"]. */
export function passwordChoices(mode: "create" | "alter"): PasswordChange[] {
  return mode === "create" ? ["keep", "set"] : ["keep", "set", "clear"];
}

/** İki girdi: boş → "empty"; farklı → "mismatch"; aynı → null. Değerler çağırana aittir (saklanmaz). */
export function passwordIssue(a: string, b: string): "empty" | "mismatch" | null {
  if (a === "") return "empty";
  return a === b ? null : "mismatch";
}

// ---- üyelikler (rol adıyla)

/** Boş/yinelenen/rolün kendisi eklenmez (aynı nesne döner); admin false, PG 16 seçenekleri null (üretici yazmaz). */
export function addMembership(d: RoleDef, role: string): RoleDef {
  if (role.trim() === "" || role === d.name || d.memberOf.some((m) => m.role === role)) return d;
  return { ...d, memberOf: [...d.memberOf, { role, admin: false, inheritOption: null, setOption: null }] };
}

export function removeMembership(d: RoleDef, i: number): RoleDef {
  if (!Number.isInteger(i) || i < 0 || i >= d.memberOf.length) return d;
  return { ...d, memberOf: d.memberOf.filter((_, k) => k !== i) };
}

export function toggleAdmin(d: RoleDef, i: number, admin: boolean): RoleDef {
  if (!Number.isInteger(i) || i < 0 || i >= d.memberOf.length) return d;
  return { ...d, memberOf: d.memberOf.map((m, k) => (k === i ? { ...m, admin } : m)) };
}

/** Taslakta olmayan yüklü üyelikler (rol adıyla, özgün sırayla) — REVOKE edilecekler. */
export function removedMemberships(original: RoleDef | null, d: RoleDef): RoleMembership[] {
  if (!original) return [];
  const names = new Set(d.memberOf.map((m) => m.role));
  return original.memberOf.filter((m) => !names.has(m.role));
}

/**
 * Kaldırılan üyeliği özgün nesnenin kopyasıyla (PG 16 seçenekleri dahil) özgün sıradaki yerine (yüklemede ad sırası;
 * önceki özgün komşusunun ardına) geri koyar. Zaten varsa ya da özgünde yoksa aynı nesne döner.
 */
export function restoreMembership(d: RoleDef, original: RoleDef, role: string): RoleDef {
  const idx = original.memberOf.findIndex((m) => m.role === role);
  if (idx < 0 || d.memberOf.some((m) => m.role === role)) return d;
  let at = 0;
  for (let k = idx - 1; k >= 0; k--) {
    const pos = d.memberOf.findIndex((m) => m.role === original.memberOf[k]!.role);
    if (pos >= 0) {
      at = pos + 1;
      break;
    }
  }
  const next = [...d.memberOf];
  next.splice(at, 0, { ...original.memberOf[idx]! });
  return { ...d, memberOf: next };
}

/** Seçilebilir roller: lookup (grantable_roles) − mevcut üyelikler − rolün kendisi. */
export function availableRoles(items: readonly LookupItem[], d: RoleDef): LookupItem[] {
  const taken = new Set(d.memberOf.map((m) => m.role));
  return items.filter((i) => i.value !== d.name && !taken.has(i.value));
}

// ---- VALID UNTIL

const pad = (n: number, w = 2) => String(n).padStart(w, "0");
const DATE_RE = /^([0-9]{4})-([0-9]{2})-([0-9]{2})$/;
const TIME_RE = /^([0-9]{2}):([0-9]{2})$/;

/** Saklanan metnin zamanı (ms); `infinity`/`null`/çözümlenemeyen → null. Mikro saniye kesri kırpılır. */
function validUntilTime(v: string | null): number | null {
  if (v === null || v === "infinity") return null;
  const t = Date.parse(v.replace(/(\.[0-9]{3})[0-9]+/, "$1"));
  return Number.isNaN(t) ? null : t;
}

/**
 * Saklanan değer → girdi parçaları (yerel saat). `null` ve `"infinity"` → `never` (süre sınırı yok). Geçersiz → null.
 */
export function validUntilParts(v: string | null): { never: boolean; date: string; time: string } | null {
  if (v === null || v === "infinity") return { never: true, date: "", time: "" };
  const t = validUntilTime(v);
  if (t === null) return null;
  const d = new Date(t);
  return {
    never: false,
    date: `${pad(d.getFullYear(), 4)}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    time: `${pad(d.getHours())}:${pad(d.getMinutes())}`,
  };
}

/**
 * Girdi parçaları → saklanacak değer: `never` → `"infinity"`; yerel tarih (+ saat, boşsa 00:00) → UTC `YYYY-MM-DDTHH:MM:SSZ`.
 * Boş/geçersiz tarih ya da saat (ör. 31 Şubat, 25:00) → null.
 */
export function validUntilFromParts(p: { never: boolean; date: string; time: string }): string | null {
  if (p.never) return "infinity";
  const dm = DATE_RE.exec(p.date);
  if (!dm) return null;
  const tm = p.time === "" ? ["", "00", "00"] : TIME_RE.exec(p.time);
  if (!tm) return null;
  const [y, mo, da, h, mi] = [Number(dm[1]), Number(dm[2]), Number(dm[3]), Number(tm[1]), Number(tm[2])];
  if (h > 23 || mi > 59) return null;
  const d = new Date(y, mo - 1, da, h, mi, 0, 0);
  if (Number.isNaN(d.getTime()) || d.getFullYear() !== y || d.getMonth() !== mo - 1 || d.getDate() !== da) return null;
  return d.toISOString().replace(/\.[0-9]{3}Z$/, "Z");
}

/** Alter'da yüklü değer varken "süre sınırı yok" seçeneği yalnızca "infinity" yazar (temizleme yok). */
export function validUntilClearValue(original: RoleDef | null): string | null {
  return original?.validUntil ? "infinity" : null;
}

// ---- kendini kilitleme

export type SelfLockout = "nologin" | "connectionLimitZero" | "validUntilPast" | "ownPassword";

/**
 * Yalnızca alter ve `connectedUser === original.name` (sunucu kaydındaki kullanıcı adı) iken: LOGIN kapanıyor, sınır 0'a
 * iniyor, VALID UNTIL geçmişe (now'a göre) alınıyor, parola set/clear. Değişmeyen (yüklendiği gibi) durum uyarı vermez.
 */
export function selfLockoutWarnings(connectedUser: string | null, original: RoleDef | null, d: RoleDef, now: Date): SelfLockout[] {
  if (!original || connectedUser === null || connectedUser !== original.name) return [];
  const out: SelfLockout[] = [];
  if (original.login && !d.login) out.push("nologin");
  if (d.connectionLimit === 0 && original.connectionLimit !== 0) out.push("connectionLimitZero");
  const t = validUntilTime(d.validUntil);
  if (d.validUntil !== original.validUntil && t !== null && t < now.getTime()) out.push("validUntilPast");
  if (d.password === "set" || d.password === "clear") out.push("ownPassword");
  return out;
}
