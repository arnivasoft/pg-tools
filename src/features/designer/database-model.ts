// Database formunun saf modeli (tembel parça): tür daraltma, create-only kilidi, yerel ayar alanları ve bağlantı sınırı
// girdisi. SQL üretmez; alan değerleri olduğu gibi gider.
import type { DatabaseDef } from "../../ipc/bindings/DatabaseDef";
import type { LocaleProvider } from "../../ipc/bindings/LocaleProvider";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";

export type DatabaseDraft = DatabaseDef & { kind: "database" };

export function asDatabase(d: ObjectDef | null): DatabaseDraft | null {
  return d && d.kind === "database" ? d : null;
}

export function databaseEditPatch(d: ObjectDef, patch: Partial<DatabaseDef>): ObjectDef {
  return d.kind === "database" ? { ...d, ...patch } : d;
}

/** Create-only alanlar (şablon, kodlama, yerel ayar) alter'da salt okunur. */
export function createOnlyLocked(mode: "create" | "alter"): boolean {
  return mode === "alter";
}

/** Sağlayıcıya göre görünen yerel ayar alanları: libc → LC_COLLATE/LC_CTYPE; icu/builtin → sağlayıcı yerel ayarı. */
export function localeFields(provider: LocaleProvider | null): { libc: boolean; providerLocale: "icu" | "builtin" | null } {
  if (provider === "icu" || provider === "builtin") return { libc: false, providerLocale: provider };
  return { libc: true, providerLocale: null };
}

/** Sağlayıcı değişimi: libc/null'a dönünce providerLocale null'a çekilir. */
export function withProvider(d: DatabaseDef, p: LocaleProvider | null): DatabaseDef {
  return { ...d, localeProvider: p, providerLocale: p === "icu" || p === "builtin" ? d.providerLocale : null };
}

/** Kodlama/yerel ayar dolu ve şablon template0 değilse ipucu (aksi halde sunucu reddedebilir). */
export function template0Hint(d: DatabaseDef): boolean {
  const set = d.encoding !== null || d.lcCollate !== null || d.lcCtype !== null;
  return set && d.template !== "template0";
}

/**
 * Bağlantı sınırı girdisi: boş/negatif/kesirli/çözümlenemeyen → undefined (olay yok sayılır). Sınırsız (-1) yalnızca
 * "Sınırsız" kutusuyla seçilir.
 */
export function connectionLimitFromInput(s: string, badInput = false): number | undefined {
  if (badInput) return undefined;
  const t = s.trim();
  if (!/^[0-9]{1,9}$/.test(t)) return undefined;
  return Number(t);
}

/** Alter'da yeniden adlandırma ya da tablespace değişimi → bağlı oturum olmamalı. */
export function needsNoSessions(original: DatabaseDef | null, d: DatabaseDef): boolean {
  return original !== null && (original.name !== d.name || original.tablespace !== d.tablespace);
}

/** "Sınırsız" kutusu: işaretli → -1; kaldırılınca yüklü sınır >= 0 ise o (0 dahil), yoksa 100. */
export function unlimitedToggle(original: DatabaseDef | null, unlimited: boolean): number {
  if (unlimited) return -1;
  return original !== null && original.connectionLimit >= 0 ? original.connectionLimit : 100;
}
