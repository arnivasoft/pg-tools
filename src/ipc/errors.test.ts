import { describe, expect, it } from "vitest";
import { IpcError, toPayload } from "./errors";

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
