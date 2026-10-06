// Yazarken hata denetimi (tasarım §4) — saf model: kapı (ne zaman istenir), hedef hesabı (UTF-16 → scalar), tanıların
// UTF-16 işaretlere çevrilmesi, düzenlemelerde işaretlerin eşlenmesi/temizlenmesi ve sürüm sayaçlı zamanlayıcı.
// CodeMirror'a çalışma zamanında bağlı değildir (yalnızca tür); düğümde test edilir.
//
// Uyuyan/meşgul oturum: arka uç `qt_check`'te boş liste döner (beklemez). Arayüzde "hata yok" göstergesi yoktur; boş
// sonuç yalnızca işaret olmaması demektir. Böylece uyuyan oturumun boş sonucu "temiz" diye sunulmaz (ilk çalıştırma
// oturumu uyandırır, sonraki denetim gerçek sonucu getirir).
import type { CheckDiag } from "../../ipc/bindings/CheckDiag";
import type { CheckSeverity } from "../../ipc/bindings/CheckSeverity";
import type { CheckTarget } from "../../ipc/bindings/CheckTarget";
import { statementBounds } from "./completion";
import { errorRange, scalarToUtf16, utf16ToScalar } from "./offsets";
import type { TabOp } from "./query-controller";
import type { TabStatus, TxView } from "./run-model";

export const CHECK_DEBOUNCE_MS = 600;

export interface CheckGate {
  enabled: boolean;
  session: string | null;
  status: TabStatus;
  op: TabOp;
  tx: TxView;
  serverGone: boolean;
}

/** Yalnızca açık + boşta oturumda, hatalı transaction dışında ve başka işlem (more/export/tx…) yokken. */
export function canCheck(g: CheckGate | null): g is CheckGate & { session: string } {
  return !!g && g.enabled && g.session !== null && g.status === "idle" && g.op === null && g.tx !== "failed" && !g.serverGone;
}

/** Editörün o anki görünümü (UTF-16). */
export interface CheckView {
  text: string;
  cursor: number;
  visibleFrom: number;
  visibleTo: number;
}

/** UTF-16 görünümden arka uç hedefi (Unicode scalar). */
export function checkTarget(v: CheckView): CheckTarget {
  const a = Math.min(v.visibleFrom, v.visibleTo);
  const b = Math.max(v.visibleFrom, v.visibleTo);
  return { cursor: utf16ToScalar(v.text, v.cursor), visibleStart: utf16ToScalar(v.text, a), visibleEnd: utf16ToScalar(v.text, b) };
}

export interface LintMark {
  /** İşaretlenecek aralık (UTF-16). */
  from: number;
  to: number;
  /** İfadenin aralığı (UTF-16): bu aralığa dokunan düzenleme işareti kaldırır. */
  stmtFrom: number;
  stmtTo: number;
  severity: CheckSeverity;
  message: string;
  sqlstate: string | null;
}

/** Tanılar → UTF-16 işaretler. Konum varsa hata karakterinden kelime sonuna; yoksa bütün ifade. */
export function diagsToMarks(text: string, diags: readonly CheckDiag[]): LintMark[] {
  const out: LintMark[] = [];
  for (const d of diags) {
    const stmtFrom = Math.min(scalarToUtf16(text, d.charOffset), text.length);
    const stmtTo = Math.min(scalarToUtf16(text, d.charOffset + d.charLen), text.length);
    const r = d.position == null ? { from: stmtFrom, to: stmtTo } : errorRange(text, d);
    if (!r) continue;
    out.push({ from: r.from, to: r.to, stmtFrom, stmtTo, severity: d.severity, message: d.message, sqlstate: d.sqlstate });
  }
  return out;
}

/** CodeMirror `ChangeDesc`'in kullanılan kısmı. */
export interface ChangesLike {
  touchesRange(from: number, to?: number): boolean | "cover";
  mapPos(pos: number, assoc?: number): number;
}

/** Düzenleme sonrası işaretler: ifadesine dokunulanlar kaldırılır (sonraki sonuca kadar), diğerleri eşlenir. */
export function mapMarks(marks: readonly LintMark[], changes: ChangesLike): LintMark[] {
  const out: LintMark[] = [];
  for (const m of marks) {
    if (changes.touchesRange(m.stmtFrom, m.stmtTo)) continue;
    out.push({
      ...m,
      from: changes.mapPos(m.from, 1),
      to: changes.mapPos(m.to, -1),
      stmtFrom: changes.mapPos(m.stmtFrom, 1),
      stmtTo: changes.mapPos(m.stmtTo, -1),
    });
  }
  return out;
}

type Timer = unknown;
export interface CheckSchedulerDeps {
  /** Gösterilen sekmenin kapısı (sekme yoksa `null`). */
  gate(): CheckGate | null;
  view(): CheckView | null;
  request(session: string, sql: string, target: CheckTarget): Promise<CheckDiag[]>;
  /** Güncel sonucun işaretleri (UTF-16, istek anındaki metne göre — sürüm aynıysa metin de aynıdır). */
  apply(marks: LintMark[]): void;
  /** Sessiz hata (geliştirici günlüğü; SQL metni yok). */
  failed?(e: unknown): void;
  setTimer?(fn: () => void, ms: number): Timer;
  clearTimer?(t: Timer): void;
}

export interface CheckScheduler {
  /** Metin değişti: sürüm artar (süren isteğin sonucu yok sayılır), zamanlayıcı yeniden kurulur. */
  edited(): void;
  /** İmleç/kaydırma: zamanlayıcı yeniden kurulur; hedef (imlecin ifadesi + görünür alan) değişmediyse istek yok. */
  moved(): void;
  /** Çalıştırma/sekme geçişi/kapatma: sürüm artar, bekleyen zamanlayıcı iptal; sonraki `moved` yeniden ister. */
  invalidate(): void;
  /** Şimdiki sürüm (test/teşhis). */
  version(): number;
  dispose(): void;
}

export function createCheckScheduler(deps: CheckSchedulerDeps): CheckScheduler {
  const setTimer = deps.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = deps.clearTimer ?? ((t) => clearTimeout(t as ReturnType<typeof setTimeout>));
  let version = 0;
  let seq = 0;
  let appliedSeq = 0;
  let timer: Timer | null = null;
  /** Son istenen hedefin anahtarı (aynı sürümde aynı hedef yeniden istenmez). */
  let lastKey: string | null = null;
  let disposed = false;

  const stop = () => {
    if (timer !== null) clearTimer(timer);
    timer = null;
  };
  const arm = () => {
    stop();
    if (disposed) return;
    timer = setTimer(fire, CHECK_DEBOUNCE_MS);
  };

  function fire() {
    timer = null;
    const g = deps.gate();
    if (!canCheck(g)) return;
    const v = deps.view();
    if (!v) return;
    const stmt = statementBounds(v.text, v.cursor);
    const key = `${version}|${stmt.from}|${v.visibleFrom}|${v.visibleTo}`;
    if (key === lastKey) return;
    lastKey = key;
    const myVersion = version;
    const mySeq = ++seq;
    deps.request(g.session, v.text, checkTarget(v)).then(
      (diags) => {
        if (disposed || myVersion !== version || mySeq < appliedSeq) return;
        appliedSeq = mySeq;
        deps.apply(diagsToMarks(v.text, diags));
      },
      (e) => {
        if (lastKey === key) lastKey = null; // aynı hedef sonra yeniden denenebilsin
        deps.failed?.(e);
      },
    );
  }

  return {
    edited() {
      version++;
      arm();
    },
    moved: arm,
    invalidate() {
      version++;
      lastKey = null;
      stop();
    },
    version: () => version,
    dispose() {
      disposed = true;
      version++;
      stop();
    },
  };
}
