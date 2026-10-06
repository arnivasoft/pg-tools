import type { ErrorPayload } from "./bindings/ErrorPayload";

export class IpcError extends Error {
  constructor(public payload: ErrorPayload) {
    super(payload.message);
  }
}

function isPayload(e: unknown): e is ErrorPayload {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export function toPayload(e: unknown): ErrorPayload {
  if (e instanceof IpcError) return e.payload;
  if (isPayload(e)) return { code: e.code, message: e.message, detail: e.detail ?? null, hint: e.hint ?? null };
  const message = e instanceof Error ? e.message : String(e);
  return { code: "unknown", message, detail: null, hint: null };
}

/**
 * Kullanıcıya gösterilecek hata metni. SQL hatalarında (`code === "query"`) arka uç dostça `message`'ı verir, PostgreSQL'in
 * kendi metni `detail`'dadır (ör. 42501 için "permission denied …"): ikisi (ve varsa ipucu) birlikte gösterilir.
 * Dashboard ve nesne diyalogları ortak kullanır.
 */
export function errorText(p: ErrorPayload): string {
  if (p.code !== "query") return p.message;
  return [p.message, p.detail, p.hint].filter(Boolean).join(" — ");
}
