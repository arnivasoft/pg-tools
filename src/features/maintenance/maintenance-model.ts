// Bakım diyaloğunun saf modeli (Plan 25). Ana paket modülü içe aktarmaz; yalnız tür ve tembel metin modülü kullanır.
// Kaynak: crates/pgt-ddl/src/maintenance.rs (`ops_for`) ve crates/pgt-ddl/src/build/maintenance.rs (kapılar, dışlamalar).
import { tm } from "../../app/strings-maintenance";
import type { MaintenanceOp } from "../../ipc/bindings/MaintenanceOp";
import type { MaintenanceProgress } from "../../ipc/bindings/MaintenanceProgress";
import type { MaintenanceTarget } from "../../ipc/bindings/MaintenanceTarget";
import type { MaintenanceTargetKind } from "../../ipc/bindings/MaintenanceTargetKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import type { ServerMessage } from "../../ipc/bindings/ServerMessage";

export type OpName = "vacuum" | "analyze" | "reindex" | "cluster";
export type OptionState = { visible: boolean; enabled: boolean; reason: string | null };

/** Rust `ops_for` aynası (spec §6.1). `versionNum` null → en geniş küme (arka uç doğrular). */
export function opsFor(kind: MaintenanceTargetKind, partitioned: boolean, versionNum: number | null): OpName[] {
  const v = versionNum ?? Number.MAX_SAFE_INTEGER;
  switch (kind) {
    case "table":
      if (!partitioned || v >= 150000) return ["vacuum", "analyze", "reindex", "cluster"];
      if (v >= 140000) return ["vacuum", "analyze", "reindex"];
      return ["vacuum", "analyze"];
    case "materialized_view":
      return ["vacuum", "analyze", "reindex"];
    case "index":
      return partitioned && v < 140000 ? [] : ["reindex"];
    case "schema":
      return ["reindex"];
    case "database":
      return ["vacuum", "analyze", "reindex"];
  }
}

/** Sürüm kapısı geçiyor mu? Sürüm bilinmiyorsa geçer (arka uç doğrular). */
const ok = (v: number | null, min: number) => v === null || v >= min;
const SHOWN: OptionState = { visible: true, enabled: true, reason: null };
const gated = (v: number | null, min: number, major: number): OptionState =>
  ok(v, min) ? SHOWN : { visible: true, enabled: false, reason: tm.gate(major) };

function clusterDefault(t: MaintenanceTarget): string | null {
  return t.clusterIndex ?? t.indexes[0] ?? null;
}

export function defaultOperation(op: OpName, target: MaintenanceTarget, versionNum: number | null): MaintenanceOp {
  switch (op) {
    case "vacuum":
      return {
        op: "vacuum",
        full: false,
        freeze: false,
        analyze: true,
        verbose: false,
        disablePageSkipping: false,
        skipLocked: false,
        indexCleanup: null,
        parallel: null,
      };
    case "analyze":
      return { op: "analyze", verbose: false, skipLocked: false };
    case "reindex":
      return { op: "reindex", concurrently: ok(versionNum, 120000) && !target.system, verbose: false };
    case "cluster":
      return { op: "cluster", verbose: false, usingIndex: clusterDefault(target) };
  }
}

/** Seçeneklerin görünürlük/etkinlik durumu; anahtarlar seçenek alan adlarıdır (+ `indexCleanupAuto`). */
export function optionStates(
  op: MaintenanceOp,
  target: MaintenanceTarget,
  versionNum: number | null,
): Record<string, OptionState> {
  const v = versionNum;
  switch (op.op) {
    case "vacuum": {
      const full: OptionState = { visible: true, enabled: false, reason: tm.excl.full };
      return {
        full: SHOWN,
        freeze: SHOWN,
        analyze: SHOWN,
        verbose: SHOWN,
        disablePageSkipping: op.full ? full : SHOWN,
        skipLocked: gated(v, 120000, 12),
        indexCleanup: gated(v, 120000, 12),
        indexCleanupAuto: gated(v, 140000, 14),
        parallel: op.full ? full : gated(v, 130000, 13),
      };
    }
    case "analyze":
      return { verbose: SHOWN, skipLocked: gated(v, 120000, 12) };
    case "reindex":
      return {
        verbose: SHOWN,
        concurrently: target.system
          ? { visible: true, enabled: false, reason: tm.excl.systemObject }
          : gated(v, 120000, 12),
      };
    case "cluster":
      return {
        verbose: SHOWN,
        usingIndex: target.indexes.length === 0 ? { visible: true, enabled: false, reason: tm.excl.noIndex } : SHOWN,
      };
  }
}

/** Devre dışı kalan seçenekleri temizler (FULL → parallel/disablePageSkipping; kapı altı; sistem nesnesi). */
export function normalize(op: MaintenanceOp, target: MaintenanceTarget, versionNum: number | null): MaintenanceOp {
  const s = optionStates(op, target, versionNum);
  switch (op.op) {
    case "vacuum": {
      const o = { ...op };
      if (!s.disablePageSkipping!.enabled) o.disablePageSkipping = false;
      if (!s.skipLocked!.enabled) o.skipLocked = false;
      if (!s.indexCleanup!.enabled || (o.indexCleanup === "auto" && !s.indexCleanupAuto!.enabled)) o.indexCleanup = null;
      if (!s.parallel!.enabled) o.parallel = null;
      return o;
    }
    case "analyze":
      return s.skipLocked!.enabled ? op : { ...op, skipLocked: false };
    case "reindex":
      return s.concurrently!.enabled ? op : { ...op, concurrently: false };
    case "cluster":
      // Listede olmayan indeks seçimi varsayılana döner.
      return op.usingIndex !== null && !target.indexes.includes(op.usingIndex)
        ? { ...op, usingIndex: clusterDefault(target) }
        : op;
  }
}

export function maintenanceRequest(object: ObjectRef, target: MaintenanceTarget, op: MaintenanceOp): PlanRequest {
  return { op: "maintenance", object, target, operation: op };
}

/** İlerleme çubuğu için yüzde (total yoksa null → belirsiz çubuk) ve metin. */
export function progressView(
  p: MaintenanceProgress | null,
  phaseLabel: (phase: string) => string,
): { percent: number | null; text: string } {
  if (!p) return { percent: null, text: tm.waitingProgress };
  const label = phaseLabel(p.phase);
  const unit = p.unit ? ` ${tm.units[p.unit]}` : "";
  if (p.done !== null && p.total !== null && p.total > 0) {
    const percent = Math.max(0, Math.min(100, Math.floor((p.done / p.total) * 100)));
    return { percent, text: `${label} — ${p.done}/${p.total}${unit} (%${percent})` };
  }
  if (p.done !== null) return { percent: null, text: `${label} — ${p.done}${unit}` };
  return { percent: null, text: label };
}

/** WARNING sayısı ve toplam ileti sayısı (taşma iletisi INFO'dur, uyarı sayılmaz). */
export function messageSummary(messages: ServerMessage[] | undefined): { warnings: number; total: number } {
  const list = messages ?? [];
  return { warnings: list.filter((m) => m.severity.toUpperCase() === "WARNING").length, total: list.length };
}

/** Başarısız/iptal REINDEX CONCURRENTLY için `*_ccnew` uyarısı gerekir mi (spec §6.6). */
export function needsCcnewNotice(op: MaintenanceOp, outcome: "applied" | "failed" | "cancelled"): boolean {
  return op.op === "reindex" && op.concurrently && outcome !== "applied";
}

/** mm:ss (60 dakikadan sonra dakika saymaya devam eder). */
export function elapsedLabel(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const mm = String(Math.floor(total / 60)).padStart(2, "0");
  const ss = String(total % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}

const POLL_MS = 1000;
const MAX_ERRORS = 3;

/**
 * İlerleme yoklayıcısı: istekler sırayla çalışır (önceki bitmeden yenisi yok), `stop` sonrası geri çağrı yok,
 * 3 ardışık hatada `onStop("errors")`. Zamanlayıcı ve getirici enjekte edilir.
 */
export function createProgressPoller(deps: {
  fetch: () => Promise<MaintenanceProgress | null>;
  schedule: (cb: () => void, ms: number) => () => void;
  onUpdate: (p: MaintenanceProgress | null) => void;
  onStop: (reason: "errors") => void;
}): { start(): void; stop(): void } {
  let gen = 0; // stop/start her çağrıda artar; eski turların sonuçları atılır
  let running = false;
  let failures = 0;
  let cancel: (() => void) | null = null;

  const arm = (g: number) => {
    cancel = deps.schedule(() => void tick(g), POLL_MS);
  };
  const tick = async (g: number) => {
    cancel = null;
    try {
      const p = await deps.fetch();
      if (g !== gen) return;
      failures = 0;
      deps.onUpdate(p);
    } catch {
      if (g !== gen) return;
      if (++failures >= MAX_ERRORS) {
        running = false;
        gen++;
        deps.onStop("errors");
        return;
      }
    }
    if (g === gen) arm(g);
  };

  return {
    start() {
      if (running) return;
      running = true;
      failures = 0;
      arm(++gen);
    },
    stop() {
      running = false;
      gen++;
      cancel?.();
      cancel = null;
    },
  };
}
