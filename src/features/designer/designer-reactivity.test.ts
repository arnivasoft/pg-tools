// Tepkisellik gerilemesi: vitest "node" ortamında solid-js sunucu yapısına çözülür (sinyaller tepkisiz) ve diğer
// testler bunu göremez. Bu dosya `solid-js`'i yalnızca kendisi için TARAYICI yapısına yönlendirir; denetleyici de
// aynı (sahte) modülü içe aktarır. Sekme denetleyici girdisinden ÖNCE çizilir: model okuyucusu girdi eklenince
// yeniden çalışmalıdır (aksi halde sekme "Yükleniyor…"da kalır).
import { describe, expect, it, vi } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { DesignerControllerDeps } from "./designer-controller";

vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));

const schema = (): ObjectDef => ({ kind: "schema", name: "app", owner: "alice", comment: null });
const role = (): ObjectDef => ({
  kind: "role", name: "app_user", login: true, superuser: false, createdb: false, createrole: false, inherit: true,
  replication: false, bypassrls: false, connectionLimit: -1, validUntil: null, password: "keep", memberOf: [], settings: [],
  comment: null, unsupported: [],
});

async function setup() {
  const solid = await import("solid-js");
  const { createDesignerController } = await import("./designer-controller");
  const api = {
    ddlDefaults: vi.fn(async () => schema()),
    ddlLoad: vi.fn(async (): Promise<ObjectDef> => schema()),
    ddlLookup: vi.fn(async () => []),
    ddlPreview: vi.fn(() => new Promise(() => {})),
    ddlApply: vi.fn(),
    ddlCancel: vi.fn(async () => null),
  };
  const ctl = createDesignerController({
    api: api as unknown as DesignerControllerDeps["api"],
    notify: vi.fn(),
    serverInfo: () => ({ name: "srv", locked: false, production: false }),
    refreshNode: vi.fn(),
  });
  return { solid, ctl, api };
}

describe("designer-controller tepkisellik (tarayıcı yapısı)", () => {
  it("girdiden önce kurulan model okuyucusu açılışta/kapanışta güncellenir; ikinci sekme de", async () => {
    const { solid, ctl } = await setup();
    // Ön koşul: gerçekten tepkisel yapı (sunucu yapısında memo hiç yeniden çalışmaz).
    const probe = solid.createRoot((dispose) => {
      const [s, set] = solid.createSignal(1);
      const m = solid.createMemo(() => s() * 2);
      set(2);
      const v = m();
      dispose();
      return v;
    });
    expect(probe).toBe(4);

    await solid.createRoot(async (dispose) => {
      const first = solid.createMemo(() => ctl.model("T1")?.status ?? null);
      const second = solid.createMemo(() => ctl.model("T2")?.status ?? null);
      const ids = solid.createMemo(() => ctl.ids().join(","));
      expect([first(), second(), ids()]).toEqual([null, null, ""]);
      await ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: { kind: "schema", oid: 1 }, refreshNodeId: null });
      expect(first()).toBe("ready");
      const p = ctl.openAlter({ tabId: "T2", serverId: "S1", databaseOid: 5, object: { kind: "schema", oid: 2 }, refreshNodeId: null });
      expect(second()).toBe("loading"); // girdi eşzamanlı kuruldu
      await p;
      expect(second()).toBe("ready");
      expect(ids()).toBe("T1,T2");
      ctl.requestFocus("T2", "name");
      ctl.close("T1");
      expect([first(), ids()]).toEqual([null, "T2"]);
      dispose();
    });
  });

  it("secretReady ve parola kapısı setSecret sonrası memo'da güncellenir (harita tepkisiz; registry dürtülür)", async () => {
    const { solid, ctl, api } = await setup();
    api.ddlLoad.mockImplementation(async () => role());
    await solid.createRoot(async (dispose) => {
      const ready = solid.createMemo(() => ctl.secretReady("T1"));
      let runs = 0;
      solid.createComputed(() => {
        ctl.secretReady("T1");
        runs++;
      });
      expect(ready()).toBe(false);
      await ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: null, object: { kind: "role", oid: 16384 }, refreshNodeId: null });
      expect(ready()).toBe(false);
      ctl.setSecret("T1", "Gizli-1"); // password = keep: saklanmaz
      expect(ready()).toBe(false);
      ctl.edit("T1", (d) => {
        if (d.kind === "role") d.password = "set";
      });
      const base = runs;
      ctl.setSecret("T1", "Gizli-1");
      expect(ready()).toBe(true);
      expect(runs).toBe(base + 1);
      ctl.setSecret("T1", "Gizli-1"); // aynı değer: dürtme yok
      expect(runs).toBe(base + 1);
      ctl.setSecret("T1", null);
      expect(ready()).toBe(false);
      expect(runs).toBe(base + 2);
      ctl.setSecret("T1", null); // zaten yok: dürtme yok
      expect(runs).toBe(base + 2);
      ctl.setSecret("T1", "Gizli-1");
      expect(ready()).toBe(true);
      ctl.close("T1");
      expect(ready()).toBe(false);
      dispose();
    });
  });

  it("focusRequest okuyucusu girdi sonradan kurulsa da güncellenir", async () => {
    const { solid, ctl } = await setup();
    await solid.createRoot(async (dispose) => {
      const focus = solid.createMemo(() => ctl.focusRequest("T1")?.field ?? null);
      expect(focus()).toBeNull();
      const p = ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: { kind: "schema", oid: 1 }, refreshNodeId: null });
      ctl.requestFocus("T1", "name");
      expect(focus()).toBe("name");
      await p;
      dispose();
    });
  });

  it("kolon/kısıt odağı girdiden önce istenir; memo görür; consumeFocus sonrası null", async () => {
    const { solid, ctl } = await setup();
    await solid.createRoot(async (dispose) => {
      const focus = solid.createMemo(() => ctl.focusRequest("T1")?.field ?? null);
      const p = ctl.openAlter({ tabId: "T1", serverId: "S1", databaseOid: 5, object: { kind: "schema", oid: 1 }, refreshNodeId: null });
      ctl.requestFocus("T1", "column:3");
      expect(focus()).toBe("column:3");
      await p;
      expect(focus()).toBe("column:3");
      ctl.consumeFocus("T1", ctl.focusRequest("T1")!.seq);
      expect(focus()).toBeNull();
      dispose();
    });
  });
});
