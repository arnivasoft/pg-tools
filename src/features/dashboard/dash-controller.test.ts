import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { DashInfo } from "../../ipc/bindings/DashInfo";
import type { Overview } from "../../ipc/bindings/Overview";
import type { Sample } from "../../ipc/bindings/Sample";
import type { Api } from "../../ipc/commands";
import { createDashController } from "./dash-controller";

const info = (n: number): DashInfo => ({ id: `D${n}`, versionNum: 160000, selfPid: 1, database: null });
const overview: Overview = { version: "PostgreSQL 16", uptimeSecs: 10, maxConnections: 100, connections: 3, oldestXactSecs: null, database: null };
let clock = 1000;
const sample = (): Sample => {
  clock += 2;
  return {
    at: clock, xactCommit: clock * 10, xactRollback: 0, tupReturned: 0, tupFetched: 0, tupInserted: 0, tupUpdated: 0, tupDeleted: 0,
    blksRead: 0, blksHit: 0, sessionsActive: 1, sessionsIdle: 0, sessionsIdleInTx: 0, sessionsOther: 0,
  };
};
const row = (pid: number, p: Partial<ActivityRow> = {}): ActivityRow => ({
  pid, leaderPid: null, database: "app", user: "alice", application: "psql", clientAddr: null, backendType: "client backend",
  state: "active", waitEventType: null, waitEvent: null, backendAgeSecs: 1, xactAgeSecs: 1, queryAgeSecs: 1, stateAgeSecs: 1,
  query: "select 1", isSelf: false, ...p,
});
const err = (code: string) => ({ code, message: `m:${code}`, detail: null, hint: null });

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function setup() {
  let opened = 0;
  const api = {
    dashOpen: vi.fn(async (_serverId: string, _db: number | null) => info(++opened)),
    dashClose: vi.fn(async (_id: string) => null),
    dashOverview: vi.fn(async (_id: string) => overview),
    dashSample: vi.fn(async (_id: string) => sample()),
    dashActivity: vi.fn(async (_id: string) => ({ rows: [row(10), row(11)], truncated: false })),
    dashLocks: vi.fn(async (_id: string) => ({ sessions: [], locks: [], truncated: false })),
    dashStatements: vi.fn(async (_id: string, _o: string) => ({ kind: "available" as const, rows: [] })),
    dashSettings: vi.fn(async (_id: string) => []),
    dashPrepared: vi.fn(async (_id: string) => []),
    dashSignal: vi.fn(async (_id: string, _pid: number, _kind: string) => "sent" as const),
  };
  const notify = vi.fn();
  const ctl = createDashController({
    api: api as unknown as Api,
    notify,
    poller: { visible: () => true, onVisibility: () => () => {} },
  });
  return { ctl, api, notify };
}

/** Bekleyen sözleri ve sıfır gecikmeli işleri boşalt. */
const flush = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

async function openActive(s: ReturnType<typeof setup>, serverId = "S1") {
  const id = await s.ctl.open({ serverId, serverName: "srv", databaseOid: null });
  s.ctl.setActive(id);
  await flush();
  return id;
}

describe("dash-controller açma ve tur", () => {
  it("open: dash_open çağrılır, status ready olur, ilk tick overview+sample çeker", async () => {
    const s = setup();
    const id = await s.ctl.open({ id: "T1", serverId: "S1", serverName: "srv", databaseOid: 7 });
    expect(id).toBe("T1");
    expect(s.api.dashOpen).toHaveBeenCalledWith("S1", 7);
    expect(s.ctl.model(id)?.status).toBe("ready");
    expect(s.ctl.model(id)?.info?.id).toBe("D1");
    expect(s.api.dashOverview).not.toHaveBeenCalled(); // etkin değil: istek yok
    s.ctl.setActive(id);
    await flush();
    expect(s.api.dashOverview).toHaveBeenCalledWith("D1");
    expect(s.api.dashSample).toHaveBeenCalledWith("D1");
    const m1 = s.ctl.model(id)!;
    expect(m1.overview).toEqual(overview);
    expect(m1.ring.samples()).toHaveLength(1);
    expect(m1.rates.commit).toEqual([]); // ilk örnek yalnızca taban
    const snap = m1.ring.samples();
    await vi.advanceTimersByTimeAsync(2000);
    const m2 = s.ctl.model(id)!;
    expect(s.api.dashSample).toHaveBeenCalledTimes(2);
    expect(m2).not.toBe(m1); // yeni model nesnesi (Solid değişimi görür)
    expect(m2.ring.samples()).not.toBe(snap); // yeni anlık görüntü
    expect(m2.rates).not.toBe(m1.rates);
    expect(m2.rates.commit).toEqual([{ t: m2.ring.samples()[1].at, v: 10 }]);
  });

  it("dash_open hatası: unknown_server → gone, diğerleri → lost (hata metniyle)", async () => {
    const s = setup();
    s.api.dashOpen.mockRejectedValueOnce(err("unknown_server"));
    const a = await s.ctl.open({ serverId: "S1", serverName: "srv", databaseOid: null });
    expect(s.ctl.model(a)?.status).toBe("gone");
    s.api.dashOpen.mockRejectedValueOnce(err("network"));
    const b = await s.ctl.open({ serverId: "S1", serverName: "srv", databaseOid: null });
    expect(s.ctl.model(b)?.status).toBe("lost");
    expect(s.ctl.model(b)?.error).toBe("m:network");
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("etkin olmayan ya da duraklatılmış sekmede istek yok", async () => {
    const s = setup();
    const a = await openActive(s);
    const b = await s.ctl.open({ serverId: "S1", serverName: "srv", databaseOid: null });
    s.ctl.setActive(b); // a arka plana
    await flush();
    const calls = () => s.api.dashSample.mock.calls.filter(([id]) => id === "D1").length;
    const before = calls();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(calls()).toBe(before);
    s.ctl.setPaused(b, true);
    const bCalls = s.api.dashSample.mock.calls.length;
    await vi.advanceTimersByTimeAsync(20_000);
    expect(s.api.dashSample.mock.calls.length).toBe(bCalls);
    s.ctl.setActive(null);
    s.ctl.setPaused(b, false);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(s.api.dashSample.mock.calls.length).toBe(bCalls);
    s.ctl.setActive(a); // dönüşte hemen
    await flush();
    expect(calls()).toBe(before + 1);
  });

  it("yalnızca açık bölümün liste isteği yapılır (overview açıkken activity/locks istenmez)", async () => {
    const s = setup();
    const id = await openActive(s);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashActivity).not.toHaveBeenCalled();
    expect(s.api.dashLocks).not.toHaveBeenCalled();
    expect(s.api.dashStatements).not.toHaveBeenCalled();
    expect(s.api.dashSettings).not.toHaveBeenCalled();

    s.ctl.setSection(id, "sessions"); // girişte hemen
    await flush();
    expect(s.api.dashActivity).toHaveBeenCalledTimes(1);
    expect(s.ctl.model(id)?.activity?.rows).toHaveLength(2);
    // grafik 2 sn'de bir; liste max(5000, aralık) − yarım tur = 4 sn'de vadesine girer (6 sn'ye kaymaz)
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(3);
    expect(s.api.dashLocks).not.toHaveBeenCalled();

    s.ctl.setSection(id, "locks");
    await flush();
    expect(s.api.dashLocks).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(3);

    s.ctl.setSection(id, "statements");
    await flush();
    expect(s.api.dashStatements).toHaveBeenCalledWith("D1", "total_time");
    s.ctl.setStatementsOrder(id, "calls"); // hemen
    await flush();
    expect(s.api.dashStatements).toHaveBeenLastCalledWith("D1", "calls");
    expect(s.api.dashSettings).not.toHaveBeenCalled();
  });

  it("aralık 10 sn ise liste de 10 sn'de bir (max(5000, aralık))", async () => {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setIntervalMs(id, 10_000);
    s.ctl.setSection(id, "sessions");
    await flush();
    expect(s.api.dashActivity).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(2);
    expect(s.ctl.model(id)?.intervalMs).toBe(10_000);
  });

  it("aralık 1 sn ise liste 5 sn'de bir; geçersiz aralık sınırlanır (0/negatif → 1 sn, NaN → 2 sn)", async () => {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setIntervalMs(id, 1000);
    s.ctl.setSection(id, "sessions");
    await flush();
    expect(s.api.dashActivity).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(s.api.dashActivity).toHaveBeenCalledTimes(2);

    s.ctl.setIntervalMs(id, 0);
    expect(s.ctl.model(id)?.intervalMs).toBe(1000);
    s.ctl.setIntervalMs(id, -500);
    expect(s.ctl.model(id)?.intervalMs).toBe(1000);
    s.ctl.setIntervalMs(id, Number.NaN);
    expect(s.ctl.model(id)?.intervalMs).toBe(2000);
    // sıkı döngü yok: 2 sn'de bir örnek
    const n = s.api.dashSample.mock.calls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashSample.mock.calls.length - n).toBeLessThanOrEqual(5);
  });

  it("etkin olmayan sekmede bölüm/sıralama değişimi istek yapmaz; etkinleşince yeni bölüm hemen çekilir", async () => {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setActive(null);
    s.ctl.setSection(id, "sessions");
    s.ctl.refreshNow(id);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashActivity).not.toHaveBeenCalled();
    s.ctl.setSection(id, "statements");
    s.ctl.setStatementsOrder(id, "calls");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashStatements).not.toHaveBeenCalled();
    s.ctl.setSection(id, "settings");
    await flush();
    expect(s.api.dashSettings).not.toHaveBeenCalled();
    const n = s.api.dashSample.mock.calls.length;
    s.ctl.setActive(id);
    await flush();
    expect(s.api.dashSample.mock.calls.length).toBe(n + 1);
    expect(s.api.dashSettings).toHaveBeenCalledTimes(1); // arka planda girilen settings etkinleşince
    s.ctl.setActive(null);
    s.ctl.setSection(id, "statements");
    s.ctl.setActive(id);
    await flush();
    expect(s.api.dashStatements).toHaveBeenCalledTimes(1);
    expect(s.api.dashStatements).toHaveBeenCalledWith("D1", "calls");
    expect(s.api.dashActivity).not.toHaveBeenCalled();
  });

  it("settings yalnızca talep üzerine: bölüme girişte bir kez, otomatik turda yok, elle Yenile ile yeniden", async () => {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setSection(id, "settings");
    await flush();
    expect(s.api.dashSettings).toHaveBeenCalledTimes(1);
    expect(s.ctl.model(id)?.settings).toEqual([]);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(s.api.dashSettings).toHaveBeenCalledTimes(1);
    s.ctl.setSection(id, "overview");
    s.ctl.setSection(id, "settings"); // zaten yüklü
    await flush();
    expect(s.api.dashSettings).toHaveBeenCalledTimes(1);
    s.ctl.refreshNow(id);
    await flush();
    expect(s.api.dashSettings).toHaveBeenCalledTimes(2);
    await s.ctl.loadPrepared(id);
    expect(s.api.dashPrepared).toHaveBeenCalledWith("D1");
    expect(s.ctl.model(id)?.prepared).toEqual([]);
    // prepared hatası kendi anahtarına yazılır, settings'i kirletmez
    s.api.dashPrepared.mockRejectedValueOnce(err("timeout"));
    await s.ctl.loadPrepared(id);
    expect(s.ctl.model(id)?.sectionError).toEqual({ prepared: "m:timeout" });
    expect(s.ctl.model(id)?.prepared).toEqual([]);
  });

  it("bölüm hatası öncekı veriyi korur ve sectionError'a yazar", async () => {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setSection(id, "sessions");
    await flush();
    const data = s.ctl.model(id)?.activity;
    expect(data?.rows).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(2000); // 2. sn turu: liste henüz vadesinde değil
    s.api.dashActivity.mockRejectedValueOnce({ ...err("query"), detail: "permission denied for view x" });
    s.api.dashOverview.mockRejectedValueOnce(err("timeout"));
    await vi.advanceTimersByTimeAsync(2000); // 4. sn: özet hatası + liste hatası aynı turda
    const m = s.ctl.model(id)!;
    expect(m.activity).toBe(data);
    expect(m.sectionError.sessions).toBe("m:query — permission denied for view x");
    expect(m.sectionError.overview).toBe("m:timeout");
    expect(m.overview).toEqual(overview);
    expect(m.status).toBe("ready");
    expect(s.notify).not.toHaveBeenCalled();
    // sonraki başarı hatayı temizler, yenileme sürer
    await vi.advanceTimersByTimeAsync(6000);
    expect(s.ctl.model(id)?.sectionError).toEqual({});
  });
});

describe("dash-controller kayıp / kapatma", () => {
  it("not_found → status lost, zamanlayıcı durur; reopen yeni DashId ile devam eder ve halkayı sıfırlar", async () => {
    const s = setup();
    const id = await openActive(s);
    await vi.advanceTimersByTimeAsync(2000);
    const oldRing = s.ctl.model(id)!.ring;
    expect(oldRing.samples()).toHaveLength(2);
    s.api.dashSample.mockRejectedValueOnce(err("not_found"));
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.ctl.model(id)?.status).toBe("lost");
    expect(s.ctl.model(id)?.error).toBe("m:not_found");
    expect(s.api.dashClose).toHaveBeenCalledWith("D1");
    expect(s.notify).not.toHaveBeenCalled(); // ham hata toast'ı değil, durum
    const n = s.api.dashSample.mock.calls.length;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(s.api.dashSample.mock.calls.length).toBe(n);

    await s.ctl.reopen(id);
    await flush();
    const m = s.ctl.model(id)!;
    expect(m.status).toBe("ready");
    expect(m.info?.id).toBe("D2");
    expect(m.ring).not.toBe(oldRing);
    expect(m.ring.samples()).toHaveLength(1);
    expect(s.api.dashSample).toHaveBeenLastCalledWith("D2");
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.ctl.model(id)?.rates.commit).toHaveLength(1);
  });

  it("not_connected / connection_lost da lost yapar; ready sekmede reopen no-op", async () => {
    const s = setup();
    const id = await openActive(s);
    await s.ctl.reopen(id);
    expect(s.api.dashOpen).toHaveBeenCalledTimes(1);
    s.api.dashOverview.mockRejectedValueOnce(err("connection_lost"));
    await vi.advanceTimersByTimeAsync(2000);
    expect(s.ctl.model(id)?.status).toBe("lost");
  });

  it("serverSessionsClosed(gone=true) → status gone; reopen yok", async () => {
    const s = setup();
    const a = await openActive(s, "S1");
    const b = await s.ctl.open({ serverId: "S2", serverName: "other", databaseOid: null });
    s.ctl.serverSessionsClosed("S1", true);
    expect(s.ctl.model(a)?.status).toBe("gone");
    expect(s.ctl.model(b)?.status).toBe("ready");
    const n = s.api.dashSample.mock.calls.length;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(s.api.dashSample.mock.calls.length).toBe(n);
    await s.ctl.reopen(a);
    expect(s.api.dashOpen).toHaveBeenCalledTimes(2); // yalnızca a ve b'nin açılışı
    expect(s.ctl.model(a)?.status).toBe("gone");
    // gone=false → lost, yeniden açılabilir
    s.ctl.serverSessionsClosed("S2", false);
    expect(s.ctl.model(b)?.status).toBe("lost");
    await s.ctl.reopen(b);
    expect(s.ctl.model(b)?.status).toBe("ready");
  });

  it("serverSessionsClosed sırasında uçuştaki yanıt yoksayılır", async () => {
    const s = setup();
    const d = deferred<Overview>();
    s.api.dashOverview.mockReturnValueOnce(d.promise);
    const id = await openActive(s);
    s.ctl.serverSessionsClosed("S1", false);
    d.resolve({ ...overview, connections: 99 });
    await flush();
    expect(s.ctl.model(id)?.overview).toBeNull();
    expect(s.ctl.model(id)?.status).toBe("lost");
  });

  it("reopen sonrası eski DashId'nin yanıtı yoksayılır", async () => {
    const s = setup();
    const id = await openActive(s);
    const d = deferred<Sample>();
    s.api.dashSample.mockReturnValueOnce(d.promise);
    await vi.advanceTimersByTimeAsync(2000); // D1 örneği uçuşta
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.ctl.model(id)?.status).toBe("lost");
    await s.ctl.reopen(id);
    await flush();
    expect(s.ctl.model(id)?.info?.id).toBe("D2");
    // eski tur hâlâ uçuşta: yeni kuşağın ilk turu onun bitişini bekler (tek uçuşta)
    expect(s.ctl.model(id)?.ring.samples()).toHaveLength(0);
    d.resolve(sample());
    await flush();
    // D1 örneği yazılmadı; yalnızca D2'nin ilk örneği var
    expect(s.ctl.model(id)?.ring.samples()).toHaveLength(1);
    expect(s.api.dashSample).toHaveBeenLastCalledWith("D2");
    expect(s.ctl.model(id)?.info?.id).toBe("D2");
    expect(s.ctl.model(id)?.status).toBe("ready");
  });

  it("close: dash_close çağrılır; uçuştaki yanıt modeli bozmaz", async () => {
    const s = setup();
    const ov = deferred<Overview>();
    const sm = deferred<Sample>();
    s.api.dashOverview.mockReturnValueOnce(ov.promise);
    s.api.dashSample.mockReturnValueOnce(sm.promise);
    const id = await openActive(s);
    s.ctl.close(id);
    expect(s.api.dashClose).toHaveBeenCalledTimes(1);
    expect(s.api.dashClose).toHaveBeenCalledWith("D1");
    expect(s.ctl.model(id)).toBeUndefined();
    ov.resolve(overview);
    sm.reject(err("not_found")); // kayıp yolu da çalışmamalı (ikinci dash_close / toast yok)
    await flush();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(s.api.dashClose).toHaveBeenCalledTimes(1);
    expect(s.api.dashSample).toHaveBeenCalledTimes(1);
    expect(s.notify).not.toHaveBeenCalled();
    expect(s.ctl.ids()).toEqual([]);
  });

  it("dash_open uçuştayken kapanan sekmenin yeni kimliği hemen kapatılır", async () => {
    const s = setup();
    const d = deferred<DashInfo>();
    s.api.dashOpen.mockReturnValueOnce(d.promise);
    const p = s.ctl.open({ id: "T", serverId: "S1", serverName: "srv", databaseOid: null });
    expect(s.ctl.model("T")?.status).toBe("connecting");
    s.ctl.setActive("T");
    await flush();
    expect(s.api.dashOverview).not.toHaveBeenCalled(); // kimlik yokken tur yok
    s.ctl.close("T");
    expect(s.api.dashClose).not.toHaveBeenCalled();
    d.resolve(info(9));
    await p;
    expect(s.api.dashClose).toHaveBeenCalledWith("D9");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(s.api.dashOverview).not.toHaveBeenCalled();
  });
});

describe("dash-controller sinyal", () => {
  async function ready() {
    const s = setup();
    const id = await openActive(s);
    s.ctl.setSection(id, "sessions");
    await flush();
    return { ...s, id };
  }

  it("signal: sent / notFound / yetki (query) hatası → durum + uygun toast; çift çağrı (uçuşta) ikinci sinyali göndermez", async () => {
    const s = await ready();
    const overviewCalls = s.api.dashOverview.mock.calls.length;
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("sent");
    expect(s.api.dashSignal).toHaveBeenCalledWith("D1", 10, "cancel");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.sent("cancel"), "success");
    await flush();
    expect(s.api.dashOverview.mock.calls.length).toBe(overviewCalls + 1); // hemen bir yenileme
    expect(s.ctl.model(s.id)?.signaling).toBe(false);

    s.api.dashSignal.mockResolvedValueOnce("not_found" as never);
    expect(await s.ctl.signal(s.id, row(10), "terminate")).toBe("notFound");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.notFound, "info");

    // SQLSTATE 42501 arka uçtan `code: "query"` ile gelir: dostça metin + PostgreSQL'in metni birlikte
    s.api.dashSignal.mockRejectedValueOnce({
      code: "query", message: "Bu işlem için yetkiniz yok.", detail: "permission denied to terminate process", hint: null,
    });
    expect(await s.ctl.signal(s.id, row(10), "terminate")).toBe("error");
    const text = s.notify.mock.calls.at(-1)?.[0] as string;
    expect(text).toContain("Bu işlem için yetkiniz yok.");
    expect(text).toContain("permission denied to terminate process");
    expect(s.notify.mock.calls.at(-1)?.[1]).toBe("error");

    s.api.dashSignal.mockRejectedValueOnce(err("invalid"));
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("error");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.ownSession, "error");

    s.api.dashSignal.mockRejectedValueOnce(err("query"));
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("error");
    expect(s.notify).toHaveBeenLastCalledWith("m:query", "error");
    expect(s.ctl.model(s.id)?.status).toBe("ready");

    // uçuşta ikinci çağrı
    const d = deferred<"sent">();
    s.api.dashSignal.mockReturnValueOnce(d.promise as never);
    s.api.dashSignal.mockClear();
    const first = s.ctl.signal(s.id, row(10), "cancel");
    expect(s.ctl.model(s.id)?.signaling).toBe(true);
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("busy");
    await flush();
    d.resolve("sent");
    expect(await first).toBe("sent");
    expect(s.api.dashSignal).toHaveBeenCalledTimes(1);
  });

  it("signal onay anında revalidate başarısızsa dash_signal çağrılmaz", async () => {
    const s = await ready();
    // canlı listede pid yok → gone
    s.api.dashActivity.mockResolvedValueOnce({ rows: [row(11)], truncated: false });
    expect(await s.ctl.signal(s.id, row(10), "terminate")).toBe("notFound");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.notFound, "info");
    // pid yeniden kullanıldı (kullanıcı değişti) → changed
    s.api.dashActivity.mockResolvedValueOnce({ rows: [row(10, { user: "bob" })], truncated: false });
    expect(await s.ctl.signal(s.id, row(10), "terminate")).toBe("changed");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.changed, "info");
    // kesilmiş listede pid yok → "oturum yok" denmez, doğrulanamadı
    s.api.dashActivity.mockResolvedValueOnce({ rows: [row(11)], truncated: true });
    expect(await s.ctl.signal(s.id, row(10), "terminate")).toBe("error");
    expect(s.notify).toHaveBeenLastCalledWith(t.dashboard.signal.unverified, "error");
    // canlı liste alınamadı → gönderilmez; SQL hatası ayrıntısıyla
    s.api.dashActivity.mockRejectedValueOnce({ ...err("query"), detail: "d", hint: "h" });
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("error");
    expect(s.notify).toHaveBeenLastCalledWith("m:query — d — h", "error");
    expect(s.api.dashSignal).not.toHaveBeenCalled();
    // canlı liste en taze haliyle modele yazılır
    s.api.dashActivity.mockResolvedValueOnce({ rows: [row(10), row(12)], truncated: false });
    s.api.dashSignal.mockResolvedValueOnce("not_found" as never); // `sent` olsaydı hemen yenileme listeyi yeniden yazardı
    await s.ctl.signal(s.id, row(10), "cancel");
    expect(s.ctl.model(s.id)?.activity?.rows.map((r) => r.pid)).toEqual([10, 12]);
  });

  it("sinyal sırasında kimlik kaybolursa durum lost, toast yok", async () => {
    const s = await ready();
    s.api.dashSignal.mockRejectedValueOnce(err("not_found"));
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("error");
    expect(s.ctl.model(s.id)?.status).toBe("lost");
    expect(s.notify).not.toHaveBeenCalled();
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("error"); // lost: gönderilmez
    expect(s.api.dashSignal).toHaveBeenCalledTimes(1);
  });

  it("signaling kuşağa bağlı: eski sinyalin bitişi yeni kuşağın uçuştaki sinyalini serbest bırakmaz", async () => {
    const s = await ready();
    const d1 = deferred<"sent">();
    const d2 = deferred<"sent">();
    s.api.dashSignal.mockReturnValueOnce(d1.promise as never).mockReturnValueOnce(d2.promise as never);
    const first = s.ctl.signal(s.id, row(10), "cancel");
    await flush();
    expect(s.api.dashSignal).toHaveBeenCalledTimes(1);
    s.ctl.serverSessionsClosed("S1", false);
    expect(s.ctl.model(s.id)?.signaling).toBe(false);
    await s.ctl.reopen(s.id);
    const second = s.ctl.signal(s.id, row(10), "cancel");
    await flush();
    expect(s.api.dashSignal).toHaveBeenCalledTimes(2);
    expect(s.api.dashSignal).toHaveBeenLastCalledWith("D2", 10, "cancel");
    d1.resolve("sent");
    expect(await first).toBe("sent");
    expect(s.ctl.model(s.id)?.signaling).toBe(true);
    expect(await s.ctl.signal(s.id, row(10), "cancel")).toBe("busy");
    expect(s.api.dashSignal).toHaveBeenCalledTimes(2);
    d2.resolve("sent");
    expect(await second).toBe("sent");
    expect(s.ctl.model(s.id)?.signaling).toBe(false);
  });

  it("sekme kapandıktan sonra gelen sinyal yanıtı toast üretmez", async () => {
    const s = await ready();
    const d = deferred<"sent">();
    s.api.dashSignal.mockReturnValueOnce(d.promise as never);
    const p = s.ctl.signal(s.id, row(10), "cancel");
    await flush();
    s.ctl.close(s.id);
    d.resolve("sent");
    await p;
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("signalGate: istek → onay → revalidate → dash_signal; iptal → çağrı yok; ikinci istek ilkini false ile çözer", async () => {
    const s = await ready();
    s.api.dashActivity.mockClear();
    // iptal
    const p1 = s.ctl.confirmSignal(s.id, row(10, { application: "pg-tools query" }), "terminate");
    const req = s.ctl.signalGate.request();
    expect(req?.tabId).toBe(s.id);
    expect(req?.row.pid).toBe(10);
    expect(req?.warnings.map((w) => w.kind)).toEqual(["pgtoolsQuery", "terminate"]);
    s.ctl.signalGate.answer(false);
    expect(await p1).toBe("cancelled");
    expect(s.ctl.signalGate.request()).toBeNull();
    expect(s.api.dashActivity).not.toHaveBeenCalled();
    expect(s.api.dashSignal).not.toHaveBeenCalled();

    // ikinci istek ilkini false ile çözer
    const a = s.ctl.confirmSignal(s.id, row(10), "cancel");
    const b = s.ctl.confirmSignal(s.id, row(11), "cancel");
    expect(await a).toBe("cancelled");
    expect(s.ctl.signalGate.request()?.row.pid).toBe(11);

    // onay → canlı doğrulama → sinyal
    s.ctl.signalGate.answer(true);
    expect(await b).toBe("sent");
    expect(s.api.dashSignal).toHaveBeenCalledTimes(1);
    // canlı liste sinyalden önce çekildi (sonraki çağrı `sent` sonrası yenilemedir)
    expect(s.api.dashActivity.mock.invocationCallOrder[0]).toBeLessThan(s.api.dashSignal.mock.invocationCallOrder[0]);
    expect(s.api.dashSignal).toHaveBeenCalledWith("D1", 11, "cancel");
    // sekme kapanınca açık onay vazgeçilir
    const c = s.ctl.confirmSignal(s.id, row(10), "cancel");
    s.ctl.close(s.id);
    expect(await c).toBe("cancelled");
    expect(s.ctl.signalGate.request()).toBeNull();
    // yanıtsız answer no-op
    s.ctl.signalGate.answer(true);
    expect(s.api.dashSignal).toHaveBeenCalledTimes(1);
  });
});
