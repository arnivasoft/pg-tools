// Tepkisellik gerilemesi: vitest "node" ortamında solid-js sunucu yapısına çözülür (sinyaller tepkisiz). Bu dosya
// `solid-js`'i yalnızca kendisi için TARAYICI yapısına yönlendirir. dash-open sekmeyi modelden ÖNCE açar: model
// okuyucusu girdi eklenince yeniden çalışmalıdır (aksi halde ikinci dashboard sekmesi hiç yüklenmez görünür).
import { describe, expect, it, vi } from "vitest";
import type { Api } from "../../ipc/commands";

vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));

describe("dash-controller tepkisellik (tarayıcı yapısı)", () => {
  it("girdiden önce kurulan model/ids okuyucusu açılışta ve kapanışta güncellenir; ikinci sekme de", async () => {
    const solid = await import("solid-js");
    const { createDashController } = await import("./dash-controller");
    let n = 0;
    const api = {
      dashOpen: vi.fn(async () => ({ id: `D${++n}`, versionNum: 160000, selfPid: 1, database: null })),
      dashClose: vi.fn(async () => null),
    };
    const ctl = createDashController({
      api: api as unknown as Api,
      notify: vi.fn(),
      poller: { visible: () => true, onVisibility: () => () => {} },
    });
    await solid.createRoot(async (dispose) => {
      const a = solid.createMemo(() => ctl.model("A")?.status ?? null);
      const b = solid.createMemo(() => ctl.model("B")?.status ?? null);
      const ids = solid.createMemo(() => ctl.ids().join(","));
      expect([a(), b(), ids()]).toEqual([null, null, ""]);
      await ctl.open({ id: "A", serverId: "S1", serverName: "srv", databaseOid: null });
      expect(a()).toBe("ready");
      await ctl.open({ id: "B", serverId: "S1", serverName: "srv", databaseOid: 5 });
      expect(b()).toBe("ready");
      expect(ids()).toBe("A,B");
      ctl.close("A");
      expect([a(), ids()]).toEqual([null, "B"]);
      ctl.close("B");
      dispose();
    });
  });
});
