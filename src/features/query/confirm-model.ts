// Çalıştırma öncesi onay ve koruma akışı (saf). Karar arka uç planına dayanır; istemci üretim/kilit türetmez.
import type { Danger } from "../../ipc/bindings/Danger";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { LockedReason } from "../../ipc/bindings/LockedReason";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { Scope } from "../../ipc/bindings/Scope";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { toPayload } from "../../ipc/errors";
import { stmtText } from "./offsets";

export interface ConfirmItem {
  index: number;
  reason: "danger" | "production";
  danger?: Danger;
  /** Üretim yazması (tehlikeyle birlikte de olabilir). */
  production: boolean;
  text: string;
}

export interface LockedItem {
  index: number;
  reason: LockedReason | null;
  text: string;
}

export type Decision =
  | { kind: "empty" }
  | { kind: "run" }
  | { kind: "confirm"; items: ConfirmItem[] }
  | { kind: "locked"; items: LockedItem[] };

/**
 * Karar yalnızca arka uç planındaki bayraklardan (`danger`, `productionWrite`, `lockedReject`) gelir.
 * Üretim bandı çağıranın işidir: çağıran `isProductionLabel` (src/features/servers/server-form.ts)
 * ile `production` prop'unu verir; burada üretim yazması istemcide yeniden türetilmez.
 */
export function decide(plans: StmtPlan[], text: string): Decision {
  if (plans.length === 0) return { kind: "empty" };
  const locked = plans.filter((p) => p.lockedReject);
  if (locked.length > 0) {
    return {
      kind: "locked",
      items: locked.map((p) => ({ index: p.index, reason: p.lockedReason, text: stmtText(text, p) })),
    };
  }
  const items: ConfirmItem[] = [];
  for (const p of plans) {
    if (p.danger == null && !p.productionWrite) continue;
    items.push({
      index: p.index,
      reason: p.danger != null ? "danger" : "production",
      ...(p.danger != null ? { danger: p.danger } : {}),
      production: p.productionWrite,
      text: stmtText(text, p),
    });
  }
  return items.length > 0 ? { kind: "confirm", items } : { kind: "run" };
}

export interface Snapshot {
  text: string;
  scope: Scope;
}

export interface GuardDeps {
  analyze: (session: string, text: string, scope: Scope) => Promise<StmtPlan[]>;
  run: (
    session: string,
    text: string,
    scope: Scope,
    limit: number,
    confirmed: boolean,
    onEvent: (ev: QueryEvent) => void,
  ) => Promise<unknown>;
  confirm: (items: ConfirmItem[]) => Promise<boolean>;
  onEvent: (ev: QueryEvent) => void;
  session: string;
  limit: number;
}

export type GuardOutcome =
  | { kind: "done" }
  | { kind: "cancelled" }
  /** Boş / yalnızca yorum / boş seçim: çalıştırılacak ifade yok. */
  | { kind: "empty" }
  | { kind: "locked"; items: LockedItem[] }
  | { kind: "rejected"; error: ErrorPayload };

/** Anlık görüntü bir kez alınır; analiz, onay ve çalıştırma hep aynı metin/kapsamı kullanır. */
export async function runWithGuards(snapshot: Snapshot, deps: GuardDeps): Promise<GuardOutcome> {
  const { text, scope } = snapshot;
  let outcome: GuardOutcome = { kind: "cancelled" };
  for (let attempt = 0; attempt < 2; attempt++) {
    let decision: Decision;
    try {
      decision = decide(await deps.analyze(deps.session, text, scope), text);
    } catch (e) {
      return { kind: "rejected", error: toPayload(e) };
    }
    if (decision.kind === "empty") return { kind: "empty" };
    if (decision.kind === "locked") return { kind: "locked", items: decision.items };
    let confirmed = false;
    if (decision.kind === "confirm") {
      if (!(await deps.confirm(decision.items))) return { kind: "cancelled" };
      confirmed = true;
    }
    try {
      await deps.run(deps.session, text, scope, deps.limit, confirmed, deps.onEvent);
      return { kind: "done" };
    } catch (e) {
      const error = toPayload(e);
      outcome = { kind: "rejected", error };
      // Sunucu durumu arada değişti: yeni analizle bir kez daha sor.
      if (error.code !== "confirmation_required") return outcome;
    }
  }
  return outcome;
}

/**
 * Söz tabanlı onay kapısı (çerçeveden bağımsız). `ask` bekleyen önceki isteği `false` ile çözer;
 * `dispose` bekleyen isteği `false` ile çözer (görünüm kaldırılırsa `runWithGuards` asılı kalmaz).
 */
export function createConfirmGate<R>(onChange: (req: R | null) => void) {
  let pending: { resolve: (ok: boolean) => void } | null = null;
  const finish = (ok: boolean) => {
    const cur = pending;
    if (!cur) return;
    pending = null;
    onChange(null);
    cur.resolve(ok);
  };
  const ask = (req: R) =>
    new Promise<boolean>((resolve) => {
      finish(false);
      pending = { resolve };
      onChange(req);
    });
  return { ask, finish, dispose: () => finish(false) };
}
