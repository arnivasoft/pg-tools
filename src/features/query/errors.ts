import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { QueryErrorCode } from "../../ipc/bindings/QueryErrorCode";

const CODES: Record<QueryErrorCode, true> = {
  confirmation_required: true,
  locked: true,
  busy: true,
  tx_failed: true,
  open_transaction: true,
  session_reset: true,
};

export function isQueryErrorCode(code: string): code is QueryErrorCode {
  return Object.prototype.hasOwnProperty.call(CODES, code);
}

// ErrorPayload SQLSTATE taşımaz; 25001 yalnızca PG iletisinden (message/detail) tanınır.
const NO_TX = /cannot run inside a transaction block|\b25001\b/i;

export function hintFor(payload: ErrorPayload): string | null {
  if (payload.code === "tx_failed") return t.query.hint.rollback;
  if (payload.code === "session_reset") return t.query.hint.sessionReset;
  if (payload.code === "locked") return t.query.hint.locked;
  if (payload.code === "query" && (NO_TX.test(payload.message) || NO_TX.test(payload.detail ?? ""))) {
    return t.query.hint.autocommit;
  }
  return null;
}
