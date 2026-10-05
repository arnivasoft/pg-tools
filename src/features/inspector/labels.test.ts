import { describe, expect, it } from "vitest";
import { kindLabel } from "./labels";

describe("kindLabel", () => {
  it("uses PostgreSQL terms", () => {
    expect(kindLabel("table")).toBe("Table");
    expect(kindLabel("serverGroup")).toBe("Server Group");
    expect(kindLabel("tableCollection")).toBe("Tables");
    expect(kindLabel("columnCollection")).toBe("Columns");
    expect(kindLabel("database")).toBe("Database");
  });
});
