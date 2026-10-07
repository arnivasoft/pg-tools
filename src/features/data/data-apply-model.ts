// Veri düzenleyici uygulama kapısı, onay kuralı ve önizleme gösterimi (saf; Solid/DOM yok).
// Not: designer/* içe aktarılmaz (paket bütçesi, Plan 17 Karar 2) — yalnız tür içe aktarımı.
import type { DataApplyOptions } from "../../ipc/bindings/DataApplyOptions";
import type { PlanView } from "../../ipc/bindings/PlanView";

export type DataGate =
  | { kind: "ready" }
  | { kind: "needsConfirm"; production: boolean; destructive: boolean }
  | {
      kind: "blocked";
      reason: "locked" | "readOnly" | "empty" | "invalid" | "pending" | "applying" | "disconnected" | "loading" | "stale" | "unknown";
    };

export interface DataGateInput {
  locked: boolean;
  production: boolean;
  editable: boolean;
  view: PlanView | null;
  viewIsCurrent: boolean;
  pending: boolean;
  applying: boolean;
  connected: boolean;
  loading: boolean;
  needsReload: boolean;
  outcomeUnknown: boolean;
  dirty: boolean;
}

const blocked = (reason: Extract<DataGate, { kind: "blocked" }>["reason"]): DataGate => ({ kind: "blocked", reason });

/** Risk sırası: safe < destructive < data_loss. */
function destructiveRisk(v: PlanView): boolean {
  return v.risk === "destructive" || v.risk === "data_loss";
}

/**
 * Öncelik: disconnected > loading > locked > readOnly > unknown > stale > empty > invalid > pending > applying
 * > needsConfirm > ready. Kirli ama görünümü henüz gelmemiş (null) ya da bayat durum `pending` sayılır.
 */
export function dataGate(i: DataGateInput): DataGate {
  if (!i.connected) return blocked("disconnected");
  if (i.loading) return blocked("loading");
  if (i.locked) return blocked("locked");
  if (!i.editable) return blocked("readOnly");
  if (i.outcomeUnknown) return blocked("unknown");
  if (i.needsReload) return blocked("stale");
  if (!i.dirty || (i.view !== null && i.view.empty)) return blocked("empty");
  if (i.view !== null && i.view.errors.length > 0) return blocked("invalid");
  if (i.pending || !i.viewIsCurrent || i.view === null) return blocked("pending");
  if (i.applying) return blocked("applying");
  const view = i.view;
  const destructive = destructiveRisk(view);
  if (destructive || (i.production && !view.empty)) return { kind: "needsConfirm", production: i.production, destructive };
  return { kind: "ready" };
}

export function canSubmit(g: DataGate, checked: boolean): boolean {
  switch (g.kind) {
    case "ready": return true;
    case "needsConfirm": return checked;
    case "blocked": return false;
  }
}

export function applyOptions(applyId: string, view: PlanView, checked: boolean, lockTimeoutMs = 5000): DataApplyOptions {
  return { applyId, lockTimeoutMs, expectedHash: view.hash, confirmed: checked };
}

/** Yıkıcı (DELETE) ya da üretim sunucusunda boş olmayan plan onay ister. */
export function riskNeedsConfirm(view: PlanView, production: boolean): boolean {
  return destructiveRisk(view) || (production && !view.empty);
}

/**
 * Bidi biçimlendirme, sıfır genişlik/görünmez biçim (U+2060–206F, SHY, CGJ, MVS), satır/paragraf ayırıcı ve
 * C0/C1 denetimleri (\t \n \r hariç). Varyasyon seçicileri ve etiket karakterleri (emoji parçaları) kaçışlanmaz.
 */
function isControl(c: number): boolean {
  return (
    (c >= 0x202a && c <= 0x202e) || (c >= 0x2060 && c <= 0x206f) || (c >= 0x200b && c <= 0x200f) ||
    c === 0x2028 || c === 0x2029 || c === 0x061c || c === 0xfeff ||
    c === 0x00ad || c === 0x034f || c === 0x180e ||
    (c <= 0x1f && c !== 0x09 && c !== 0x0a && c !== 0x0d) || (c >= 0x7f && c <= 0x9f) ||
    (c >= 0xd800 && c <= 0xdfff)
  );
}

function escapeUnit(c: number): string {
  return `⟦U+${c.toString(16).toUpperCase().padStart(4, "0")}⟧`;
}

/**
 * `ch` for..of ile gelen bir kod noktası dizgesidir; tek başına vekil birim (eşsiz) ise uzunluğu 1'dir ve
 * isControl onu yakalar; geçerli çiftler (uzunluk 2) olduğu gibi kalır.
 */
function piece(ch: string): string {
  if (ch.length === 1) {
    const c = ch.charCodeAt(0);
    if (isControl(c)) return escapeUnit(c);
    return ch;
  }
  return ch;
}

export function visibleControls(text: string): string {
  let out = "";
  for (const ch of text) out += piece(ch);
  return out;
}

/** En çok `max` kod birimi; vekil çift ortadan kesilmez; `omitted` = kalan kod birimi sayısı. */
export function shortenStatement(sql: string, max = 4000): { text: string; omitted: number } {
  if (sql.length <= max) return { text: sql, omitted: 0 };
  let cut = Math.max(0, max);
  if (cut > 0 && cut < sql.length) {
    const prev = sql.charCodeAt(cut - 1);
    const next = sql.charCodeAt(cut);
    if (prev >= 0xd800 && prev <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) cut -= 1;
  }
  return { text: sql.slice(0, cut), omitted: sql.length - cut };
}

/**
 * Kaçışlar uygulanmış gösterim; kaçışlı metin uzunluğu `max`'ı aşmaz ve kaçış ortadan bölünmez.
 * `omitted` = gösterilmeyen özgün UTF-16 kod birimi sayısı.
 */
export function statementDisplay(sql: string, max = 4000): { text: string; omitted: number } {
  let out = "";
  let consumed = 0;
  for (const ch of sql) {
    const p = piece(ch);
    if (out.length + p.length > max) break;
    out += p;
    consumed += ch.length;
  }
  return { text: out, omitted: sql.length - consumed };
}
