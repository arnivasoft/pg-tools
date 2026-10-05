import { describe, expect, it } from "vitest";
import { forNode } from "./for-node";

describe("forNode", () => {
  it("returns the value when the id matches", () => {
    expect(forNode({ id: "a", value: "x" }, "a")).toBe("x");
  });
  it("returns undefined when the id differs", () => {
    expect(forNode({ id: "a", value: "x" }, "b")).toBeUndefined();
  });
  it("returns undefined for a missing entry or id", () => {
    expect(forNode(undefined, "a")).toBeUndefined();
    expect(forNode({ id: "a", value: "x" }, undefined)).toBeUndefined();
  });
  it("keeps empty values", () => {
    expect(forNode({ id: "a", value: "" }, "a")).toBe("");
  });
});
