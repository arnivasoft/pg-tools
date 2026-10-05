// Çalıştırma öncesi onay ve koruma akışı (saf). Karar arka uç planına dayanır; istemci üretim/kilit türetmez.
import type { Danger } from "../../ipc/bindings/Danger";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { LockedReason } from "../../ipc/bindings/LockedReason";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { Scope } from "../../ipc/bindings/Scope";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { toPayload } from "../../ipc/errors";
import { stmtText } from "./offsets";

export interface ServerGuardInfo {
  label: string | null;
  readOnlyLock: boolean;
  isProduction: boolean;
}

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
  | { kind: "run" }
  | { kind: "confirm"; items: ConfirmItem[] }
  | { kind: "locked"; items: LockedItem[] };

/** `_server` yalnızca arayüz biçimi içindir; karar arka uç planındaki bayraklardan gelir. */
export function decide(plans: StmtPlan[], text: string, _server?: ServerGuardInfo): Decision {
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
