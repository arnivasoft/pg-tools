// Görsel EXPLAIN akışı (saf): seçenek kapıları (sunucu sürümü), gönderilecek seçenekler ve onaylı yeniden deneme.
// Yalnız tembel QueryTab parçası içe aktarır; ana paket modüllerine (`app/strings`, `explorer/*`) bağımlılığı yoktur.
// Sürüm kapıları arka uçta da doğrulanır (sessiz atlama yok); burada menüde gerekçeyle devre dışı bırakmak içindir.
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ExplainOptions } from "../../ipc/bindings/ExplainOptions";
import type { ExplainResult } from "../../ipc/bindings/ExplainResult";
import type { Scope } from "../../ipc/bindings/Scope";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { decide, type ConfirmItem, type Snapshot } from "./confirm-model";
import { stmtText } from "./offsets";
import { tplan } from "./strings-plan";

/** Arka uç varsayılanlarıyla aynı (`ExplainOptions::default`): ANALYZE kapalı; TIMING/COSTS/SUMMARY/geri alma açık. */
export const DEFAULT_EXPLAIN: ExplainOptions = {
  analyze: false,
  buffers: false,
  verbose: false,
  settings: false,
  wal: false,
  timing: true,
  costs: true,
  summary: true,
  rollback: true,
};

/** Menüde gösterilen seçenekler (sırasıyla). `summary` menüde yoktur (varsayılan). */
export const MENU_OPTIONS = ["analyze", "buffers", "verbose", "settings", "wal", "timing", "costs", "rollback"] as const;
export type MenuOption = (typeof MENU_OPTIONS)[number];

/**
 * Plan isteğinin türü: `menu` ContextBar düğmesi (menüdeki ANALYZE seçimi geçerli), `plain` F7 (ANALYZE'sız),
 * `analyze` Shift+F7 (ANALYZE ile; onay akışı geçerli).
 */
export type ExplainKind = "menu" | "plain" | "analyze";

/** Kısa sürüm metni ("16.4", "17beta1", "10.23") → `server_version_num` yaklaşığı; çözülemezse null. */
export function versionNumFromShort(v: string | null | undefined): number | null {
  const m = /^\s*(\d+)(?:\.(\d+))?/.exec(v ?? "");
  if (!m) return null;
  const major = Number(m[1]);
  const minor = m[2] ? Number(m[2]) : 0;
  // PG 10+: MMmmmm; 9.x: MMmmpp (9.6 → 90600).
  return major >= 10 ? major * 10_000 + minor : major * 10_000 + minor * 100;
}

/** Seçeneğin kapısı: `reason` doluysa devre dışıdır (gerekçe menüde metin olarak görünür). */
export type OptionGate = { reason: string | null };

/**
 * Sürüm ve ANALYZE bağımlılıkları (spec §1 "Seçenekler"). `version` null ise (sürüm bilinmiyor) yalnız ANALYZE
 * bağımlılıkları uygulanır; sürüm hatası arka uçtan ileti olarak döner.
 */
export function optionGates(opts: ExplainOptions, version: number | null): Record<MenuOption, OptionGate> {
  const g = tplan.gate;
  const below = (n: number) => version !== null && version < n;
  const needsAnalyze = opts.analyze ? null : g.needsAnalyze;
  return {
    analyze: { reason: null },
    buffers: { reason: !opts.analyze && below(130_000) ? g.buffersPre13 : null },
    verbose: { reason: null },
    settings: { reason: below(120_000) ? g.pg12 : null },
    wal: { reason: below(130_000) ? g.pg13 : needsAnalyze },
    timing: { reason: needsAnalyze },
    costs: { reason: null },
    rollback: { reason: needsAnalyze },
  };
}

/**
 * Gönderilecek seçenekler: istek türü ANALYZE'ı belirler (`menu` menüdekini kullanır), kapısı kapalı seçenekler
 * kapatılır (ör. ANALYZE kapatılınca WAL; PG 12 altı SETTINGS). TIMING/geri alma ANALYZE'sız varsayılana döner.
 */
export function effectiveOptions(opts: ExplainOptions, kind: ExplainKind, version: number | null): ExplainOptions {
  const analyze = kind === "menu" ? opts.analyze : kind === "analyze";
  const base: ExplainOptions = { ...opts, analyze };
  const gates = optionGates(base, version);
  const out: ExplainOptions = { ...base };
  for (const k of MENU_OPTIONS) if (gates[k].reason !== null) out[k] = DEFAULT_EXPLAIN[k];
  return out;
}

/**
 * Menüde görünen işaretler: açık seçenekte saklı değer; devre dışı seçenekte ANALYZE ile (Shift+F7) gönderilecek
 * değer (ör. ANALYZE kapalıyken TIMING/geri alma saklı değeriyle; sürüm altı seçenek kapalı) — işaret gönderilene doğru.
 */
export function shownOptions(opts: ExplainOptions, version: number | null): ExplainOptions {
  const gates = optionGates(opts, version);
  const sent = effectiveOptions(opts, "analyze", version);
  const out: ExplainOptions = { ...opts };
  for (const k of MENU_OPTIONS) if (gates[k].reason !== null) out[k] = sent[k];
  return out;
}

export interface ExplainDeps {
  session: string;
  options: ExplainOptions;
  explain: (session: string, sql: string, scope: Scope, options: ExplainOptions, confirmed: boolean) => Promise<ExplainResult>;
  /** Onay öğeleri için ifadenin sınıflandırması (yalnız `confirmation_required` sonrası çağrılır). */
  analyze: (session: string, sql: string, scope: Scope) => Promise<StmtPlan[]>;
  confirm: (items: ConfirmItem[]) => Promise<boolean>;
  /** Üretim etiketli sunucu (yedek onay öğesinin rozeti). */
  production: boolean;
  payload: (e: unknown) => ErrorPayload;
}

export type ExplainOutcome =
  | { kind: "done"; result: ExplainResult }
  | { kind: "cancelled" }
  | { kind: "rejected"; error: ErrorPayload };

/**
 * Onay gerekiyorsa (`confirmation_required`: ANALYZE ile tehlikeli/üretim yazımı) ifade sınıflandırılır, onay
 * diyaloğu sorulur ve `confirmed: true` ile yeniden denenir (yalnız bir kez; onaylı istek
 * yine reddedilirse hata olarak döner). Kilit (`locked`) ve diğer hatalar `rejected` olarak döner.
 */
export async function explainWithGuards(snap: Snapshot, deps: ExplainDeps): Promise<ExplainOutcome> {
  let confirmed = false;
  let last: ErrorPayload | null = null;
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      return { kind: "done", result: await deps.explain(deps.session, snap.text, snap.scope, deps.options, confirmed) };
    } catch (e) {
      last = deps.payload(e);
      if (last.code !== "confirmation_required" || attempt === 1) return { kind: "rejected", error: last };
    }
    let items: ConfirmItem[];
    try {
      items = confirmItems(await deps.analyze(deps.session, snap.text, snap.scope), snap.text, deps.production);
    } catch (e) {
      return { kind: "rejected", error: deps.payload(e) };
    }
    if (!(await deps.confirm(items))) return { kind: "cancelled" };
    confirmed = true;
  }
  return { kind: "rejected", error: last! };
}

/**
 * Onay öğeleri: düz ifadenin planındaki bayraklar (tehlike/üretim yazımı; EXPLAIN ANALYZE sarmalı aynı ifadeyi
 * çalıştırır). Bayrak yoksa (ör. `EXECUTE` sarmalanınca yazma sayılır) ifade metniyle tek öğe.
 */
export function confirmItems(plans: StmtPlan[], text: string, production: boolean): ConfirmItem[] {
  const d = decide(plans, text);
  if (d.kind === "confirm") return d.items;
  const p = plans[0];
  return [{ index: p?.index ?? 0, reason: production ? "production" : "danger", production, text: p ? stmtText(text, p) : text.trim() }];
}
