import { describe, expect, it } from "vitest";
import { nextTabIndex, panelId, tabId } from "./tabstrip-model";

describe("nextTabIndex", () => {
  it("sağ ok bir sonrakine gider ve sona gelince başa sarar", () => {
    expect(nextTabIndex(0, 3, "ArrowRight")).toBe(1);
    expect(nextTabIndex(2, 3, "ArrowRight")).toBe(0);
  });
  it("sol ok bir öncekine gider ve başta sona sarar", () => {
    expect(nextTabIndex(2, 3, "ArrowLeft")).toBe(1);
    expect(nextTabIndex(0, 3, "ArrowLeft")).toBe(2);
  });
  it("Home ilkine, End sona gider", () => {
    expect(nextTabIndex(1, 3, "Home")).toBe(0);
    expect(nextTabIndex(1, 3, "End")).toBe(2);
  });
  it("tek sekmede hep 0", () => {
    for (const k of ["ArrowLeft", "ArrowRight", "Home", "End"] as const) expect(nextTabIndex(0, 1, k)).toBe(0);
  });
  it("sekme yokken 0 döner", () => {
    expect(nextTabIndex(0, 0, "ArrowRight")).toBe(0);
  });
});

describe("id üretimi", () => {
  it("sekme ve panel kimlikleri", () => {
    expect(tabId("b", "ddl")).toBe("b-tab-ddl");
    expect(panelId("b", "ddl")).toBe("b-panel-ddl");
  });
});
