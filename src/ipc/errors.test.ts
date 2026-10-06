import { describe, expect, it } from "vitest";
import { IpcError, errorText, toPayload } from "./errors";

describe("toPayload", () => {
  it("keeps ErrorPayload objects from Tauri", () => {
    const p = { code: "auth", message: "m", detail: null, hint: null };
    expect(toPayload(p)).toEqual(p);
    expect(toPayload(new IpcError(p))).toEqual(p);
  });

  it("wraps strings and unknown errors", () => {
    expect(toPayload("boom")).toEqual({ code: "unknown", message: "boom", detail: null, hint: null });
    expect(toPayload(new Error("x")).message).toBe("x");
  });
});

describe("errorText", () => {
  it("query kodunda message — detail — hint; boş parçalar atlanır", () => {
    expect(errorText({ code: "query", message: "m", detail: "d", hint: "h" })).toBe("m — d — h");
    expect(errorText({ code: "query", message: "m", detail: null, hint: "h" })).toBe("m — h");
  });
  it("diğer kodlarda yalnızca message", () => {
    expect(errorText({ code: "locked", message: "m", detail: "d", hint: "h" })).toBe("m");
  });
});
