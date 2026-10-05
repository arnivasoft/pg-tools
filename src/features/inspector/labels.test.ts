import { describe, expect, it } from "vitest";
import { kindLabel } from "./labels";

describe("kindLabel", () => {
  it("uses PostgreSQL terms", () => {
    expect(kindLabel("table")).toBe("Table");
    expect(kindLabel("serverGroup")).toBe("Server Group");
    expect(kindLabel("tableCollection")).toBe("Tables");
    expect(kindLabel("columnCollection")).toBe("Columns");
    expect(kindLabel("database")).toBe("Database");
    expect(kindLabel("policyCollection")).toBe("RLS Policies");
    expect(kindLabel("triggerFunctionCollection")).toBe("Trigger Functions");
    expect(kindLabel("materializedView")).toBe("Materialized View");
    expect(kindLabel("roleCollection")).toBe("Login/Group Roles");
  });
});
