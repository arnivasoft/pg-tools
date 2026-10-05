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
