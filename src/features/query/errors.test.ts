import { describe, expect, it } from "vitest";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { hintFor, isQueryErrorCode } from "./errors";

const p = (code: string, message = "m", detail: string | null = null): ErrorPayload => ({ code, message, detail, hint: null });

describe("isQueryErrorCode", () => {
  it("bilinen kodlar", () => {
    for (const c of ["confirmation_required", "locked", "busy", "tx_failed", "open_transaction", "session_reset"])
      expect(isQueryErrorCode(c)).toBe(true);
  });
  it("diğerleri", () => {
    for (const c of ["query", "invalid", "toString", "__proto__", ""]) expect(isQueryErrorCode(c)).toBe(false);
  });
});

describe("hintFor", () => {
  it("kod tabanlı", () => {
    expect(hintFor(p("tx_failed"))).toBe(t.query.hint.rollback);
    expect(hintFor(p("session_reset"))).toBe(t.query.hint.sessionReset);
    expect(hintFor(p("locked"))).toBe(t.query.hint.locked);
  });
  it("25001 ileti/ayrıntıdan", () => {
    expect(hintFor(p("query", "VACUUM cannot run inside a transaction block"))).toBe(t.query.hint.autocommit);
    expect(hintFor(p("query", "x", "CREATE DATABASE CANNOT RUN INSIDE A TRANSACTION BLOCK"))).toBe(t.query.hint.autocommit);
    expect(hintFor(p("query", "x", "SQLSTATE 25001"))).toBe(t.query.hint.autocommit);
  });
  it("ilgisiz", () => {
    expect(hintFor(p("query", "syntax error"))).toBeNull();
    expect(hintFor(p("busy"))).toBeNull();
  });
});
