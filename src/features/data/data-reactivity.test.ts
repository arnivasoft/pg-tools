// Tepkisellik: vitest "node" ortamında solid-js sunucu yapısına çözülür (sinyaller tepkisiz). Bu dosya `solid-js`'i
// yalnızca kendisi için TARAYICI yapısına yönlendirir; denetleyici de aynı (sahte) modülü içe aktarır. Sekme
// denetleyici girdisinden ÖNCE çizilebilir: model okuyucusu girdi eklenince/silinince yeniden çalışmalıdır.
import { describe, expect, it, vi } from "vitest";
import type { DataSource } from "../../ipc/bindings/DataSource";
import type { DataApi } from "./data-commands";

vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));

const source = (): DataSource => ({
  object: { kind: "table", oid: 1 }, schema: "public", name: "t", relkind: "table",
  columns: [
    { attnum: 1, name: "id", typeOid: 23, typeDisplay: "integer", notNull: true, hasDefault: false, identity: "none", generated: false, insertable: true, updatable: true },
    { attnum: 2, name: "v", typeOid: 25, typeDisplay: "text", notNull: false, hasDefault: false, identity: "none", generated: false, insertable: true, updatable: true },
  ],
  key: { source: "primary_key", name: "pk", columns: ["id"] }, canSelect: true, canInsert: true, canUpdate: true, canDelete: true,
  locked: false, editable: true, readOnlyReason: null, fingerprint: "fp",
});

const flush = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

describe("data-controller tepkisellik (tarayıcı yapısı)", () => {
  it("model(tabId) registry'ye bağlı: girdiden önce kurulan okuyucu açılış/düzenleme/kapanışta güncellenir; isDirty", async () => {
    const solid = await import("solid-js");
    const { createDataController } = await import("./data-controller");
    const { setCell } = await import("./data-edit-model");
    const api = {
      dataLoad: vi.fn(async () => source()),
      dataSelect: vi.fn(async () => ({ result: "ok", select: { sql: "q", versionIndex: 2, hasVersion: true } })),
      qtOpenReadonly: vi.fn(async () => "s"),
      qtRun: vi.fn<DataApi["qtRun"]>(async (_s, _q, _sc, _l, _c, on) => {
        on({ type: "started", stmtIndex: 0, kind: "select", columns: [0, 1, 2].map((i) => ({ name: `c${i}`, typeName: "t", typeOid: 25, typeDisplay: "t" })), charOffset: 0 });
        on({ type: "rows", stmtIndex: 0, rows: [["1", "a", "x1"]], more: false });
        return null;
      }),
      qtMore: vi.fn(), qtClose: vi.fn(async () => null), qtCancel: vi.fn(), ddlTargetOf: vi.fn(),
      dataPreview: vi.fn(() => new Promise(() => {})), dataApply: vi.fn(), dataCancel: vi.fn(async () => null),
      refreshServers: vi.fn(), connect: vi.fn(async () => true),
    };
    const ctl = createDataController({
      api: api as unknown as DataApi,
      serverInfo: () => ({ name: "srv", locked: false, production: false }),
      timer: { set: () => 0, clear: () => {} },
    });
    const seen: (string | null)[] = [];
    const dirty: boolean[] = [];
    const dispose = solid.createRoot((d) => {
      const status = solid.createMemo(() => ctl.model("g1")?.status ?? null);
      solid.createEffect(() => seen.push(status()));
      const isDirty = solid.createMemo(() => ctl.isDirty("g1"));
      solid.createEffect(() => dirty.push(isDirty()));
      return d;
    });
    expect(seen).toEqual([null]);
    ctl.openTab({ tabId: "g1", serverId: "S1", databaseOid: 5, object: { kind: "table", oid: 1 } });
    await flush();
    expect(seen).toEqual([null, "loading", "ready"]);
    expect(dirty).toEqual([false]);
    ctl.edit("g1", (l, e) => setCell(l, e, 0, "v", { kind: "text", value: "b" }));
    expect(dirty).toEqual([false, true]);
    expect(ctl.ids()).toEqual(["g1"]);
    ctl.close("g1");
    expect(seen.at(-1)).toBeNull();
    expect(dirty.at(-1)).toBe(false);
    dispose();
  });
});
