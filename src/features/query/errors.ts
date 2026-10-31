import { t } from "../../app/strings";
import { thint } from "./strings-hint";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { QueryErrorCode } from "../../ipc/bindings/QueryErrorCode";

const CODES: Record<QueryErrorCode, true> = {
  confirmation_required: true,
  locked: true,
  busy: true,
  tx_failed: true,
  open_transaction: true,
  session_reset: true,
  explain_scope: true,
  explain_already: true,
  explain_unsupported: true,
  explain_option: true,
  explain_too_large: true,
};

export function isQueryErrorCode(code: string): code is QueryErrorCode {
  return Object.prototype.hasOwnProperty.call(CODES, code);
}

// ErrorPayload SQLSTATE taşımaz; 25001 yalnızca PG iletisinden (message/detail) tanınır.
const NO_TX = /cannot run inside a transaction block|\b25001\b/i;

export function hintFor(payload: ErrorPayload): string | null {
  if (payload.code === "tx_failed") return thint.rollback;
  if (payload.code === "session_reset") return thint.sessionReset;
  if (payload.code === "locked") return t.query.hint.locked;
  if (payload.code === "query" && (NO_TX.test(payload.message) || NO_TX.test(payload.detail ?? ""))) {
    return thint.autocommit;
  }
  return null;
}
