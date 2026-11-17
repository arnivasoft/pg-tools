import { beforeEach, describe, expect, it, vi } from "vitest";
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
import { searchObjects } from "./search-commands";

describe("searchObjects", () => {
  beforeEach(() => {
    invoke.mockReset();
  });
  it("parametre adları arka uçla birebir", async () => {
    invoke.mockResolvedValue({ hits: [], truncated: false });
    await searchObjects("S", 5, "ad", ["table"]);
    expect(invoke).toHaveBeenCalledWith("search_objects", { serverId: "S", databaseOid: 5, query: "ad", kinds: ["table"] });
  });
  it("hata yükünün alanlarını taşır", async () => {
    invoke.mockRejectedValue({ code: "not_connected", message: "Bağlı değil", detail: null, hint: null });
    await expect(searchObjects("S", 5, "ad", [])).rejects.toMatchObject({ code: "not_connected", message: "Bağlı değil" });
  });
});
