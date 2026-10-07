import { describe, expect, it } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { classifyError, designerErrorText, failedText, isConnectionCode, isGateCode } from "./errors";

const p = (code: string, message = `m:${code}`, detail: string | null = null, hint: string | null = null): ErrorPayload =>
  ({ code, message, detail, hint });

describe("designer errors", () => {
  it("classifyError yalnızca code'a bakar (metin aynı olsa da)", () => {
    expect(classifyError(p("not_connected"))).toBe("disconnected");
    expect(classifyError(p("connection_lost"))).toBe("disconnected");
    expect(classifyError(p("unknown_server"))).toBe("gone");
    expect(classifyError(p("not_found"))).toBe("gone");
    expect(classifyError(p("plan_changed"))).toBe("planChanged");
    expect(classifyError(p("invalid_def"))).toBe("invalidDef");
    expect(classifyError(p("locked"))).toBe("locked");
    expect(classifyError(p("confirmation_required"))).toBe("confirmation");
    expect(classifyError(p("production_confirmation_required"))).toBe("confirmation");
    expect(classifyError(p("query", "plan_changed"))).toBe("other");
    expect(classifyError(p("unknown", "not_found"))).toBe("other");
  });

  it("isConnectionCode / isGateCode tabloları", () => {
    expect(["not_connected", "connection_lost", "unknown_server"].every(isConnectionCode)).toBe(true);
    expect(isConnectionCode("not_found")).toBe(false);
    expect(["locked", "confirmation_required", "production_confirmation_required", "plan_changed", "invalid_def"].every(isGateCode)).toBe(true);
    expect(isGateCode("query")).toBe(false);
  });

  it("designerErrorText: kapı kodları sabit metin, query → message — detail — hint, diğer → message", () => {
    expect(designerErrorText(p("plan_changed"))).toBe(tdes.gate.planChanged);
    expect(designerErrorText(p("query", "m", "d", "h"))).toBe("m — d — h");
    expect(designerErrorText(p("io", "m", "d"))).toBe("m");
  });

  it("role_password_missing → passwordMissing sınıfı ve tdes.gate.passwordMissing metni; kapı kodu", () => {
    expect(classifyError(p("role_password_missing"))).toBe("passwordMissing");
    expect(designerErrorText(p("role_password_missing", "arka uç iletisi"))).toBe(tdes.gate.passwordMissing);
    expect(isGateCode("role_password_missing")).toBe(true);
  });

  it("failedText: 55P03 → kilit zaman aşımı; diğerleri errorText", () => {
    const base = { outcome: "failed" as const, failedIndex: 1, committed: 0, position: null };
    expect(failedText({ ...base, sqlstate: "55P03", error: p("query", "canceling statement") })).toBe(tdes.apply.failed(1, tdes.apply.lockTimeout));
    expect(failedText({ ...base, sqlstate: "42P07", error: p("query", "m", "d") })).toBe(tdes.apply.failed(1, "m — d"));
  });
});
