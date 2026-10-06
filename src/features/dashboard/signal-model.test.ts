import { describe, expect, it } from "vitest";
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { BlockSession } from "../../ipc/bindings/BlockSession";
import { minimalRow, resolveSignalRow, revalidate, signalWarnings } from "./signal-model";

const row = (p: Partial<ActivityRow> = {}): ActivityRow => ({
  pid: 10, leaderPid: null, database: "db", user: "alice", application: "psql", clientAddr: null, backendType: "client backend",
  state: "active", waitEventType: null, waitEvent: null, backendAgeSecs: 1, xactAgeSecs: null, queryAgeSecs: 1, stateAgeSecs: 1,
  query: "select 1", isSelf: false, ...p,
});
const kinds = (r: ActivityRow, k: "cancel" | "terminate") => signalWarnings(r, k).map((w) => w.kind);

describe("signalWarnings", () => {
  it("sıradan oturumda iptal uyarısız", () => {
    expect(kinds(row(), "cancel")).toEqual([]);
  });
  it("terminate her zaman terminate uyarısı", () => {
    expect(kinds(row(), "terminate")).toEqual(["terminate"]);
  });
  it("kendi satırı → self", () => {
    expect(kinds(row({ isSelf: true }), "cancel")).toEqual(["self"]);
  });
  it("pg-tools sorgu uygulaması → pgtoolsQuery (tam eşleşme)", () => {
    expect(kinds(row({ application: "pg-tools query" }), "cancel")).toEqual(["pgtoolsQuery"]);
    expect(kinds(row({ application: "pg-tools query 2" }), "cancel")).toEqual([]);
  });
  it("'idle in transaction' ve '(aborted)' → idleInTx", () => {
    expect(kinds(row({ state: "idle in transaction" }), "cancel")).toEqual(["idleInTx"]);
    expect(kinds(row({ state: "idle in transaction (aborted)" }), "cancel")).toEqual(["idleInTx"]);
    expect(kinds(row({ state: "idle" }), "cancel")).toEqual([]);
    expect(kinds(row({ state: null }), "cancel")).toEqual([]);
  });
  it("uyarılar birikir ve sabit sırada", () => {
    expect(kinds(row({ isSelf: true, application: "pg-tools query", state: "idle in transaction" }), "terminate")).toEqual([
      "self", "pgtoolsQuery", "idleInTx", "terminate",
    ]);
  });
});

describe("revalidate", () => {
  it("pid yok → gone (boş liste dahil)", () => {
    expect(revalidate(row(), [])).toEqual({ ok: false, reason: "gone" });
    expect(revalidate(row(), [row({ pid: 11 })])).toEqual({ ok: false, reason: "gone" });
  });
  it("pid aynı ama user/application/backendType değişti → changed", () => {
    expect(revalidate(row(), [row({ user: "bob" })])).toEqual({ ok: false, reason: "changed" });
    expect(revalidate(row(), [row({ application: "dbeaver" })])).toEqual({ ok: false, reason: "changed" });
    expect(revalidate(row(), [row({ backendType: "autovacuum worker" })])).toEqual({ ok: false, reason: "changed" });
    expect(revalidate(row({ user: null }), [row({ user: "bob" })])).toEqual({ ok: false, reason: "changed" });
  });
  it("aynı kimlik → ok; durum/sorgu değişse de", () => {
    expect(revalidate(row(), [row({ pid: 1 }), row({ state: "idle", query: "x" })])).toEqual({ ok: true });
  });
});

describe("minimalRow / resolveSignalRow", () => {
  const bs: BlockSession = { pid: 7, user: "alice", application: "psql", state: "active", query: "select 1", blockedBy: [], waitSecs: 4 };
  it("BlockSession alanlarını taşır, isSelf false", () => {
    expect(minimalRow(bs)).toMatchObject({ pid: 7, user: "alice", application: "psql", state: "active", query: "select 1", isSelf: false });
  });
  it("gerçek satır varsa onu, yoksa asgariyi seçer", () => {
    const real = row({ pid: 7, isSelf: false, state: "idle", query: "old", database: "d1" });
    expect(resolveSignalRow(bs, [row({ pid: 1 }), real])).toEqual({ ...real, state: "active", query: "select 1" });
    expect(resolveSignalRow(bs, [row({ pid: 7, user: "bob" })]).backendType).toBe("");
    expect(resolveSignalRow(bs, [row({ pid: 7, application: "x" })]).backendType).toBe("");
    expect(resolveSignalRow(bs, [row({ pid: 1 })]).backendType).toBe("");
    expect(resolveSignalRow(bs, null).pid).toBe(7);
  });
  it("asgari satır backendType yüzünden yanlış 'changed' vermez (autovacuum worker); user/app değişirse verir", () => {
    const shown = minimalRow(bs);
    expect(revalidate(shown, [row({ pid: 7, user: "alice", application: "psql", backendType: "autovacuum worker" })])).toEqual({ ok: true });
    expect(revalidate(shown, [row({ pid: 7, user: "bob", application: "psql" })])).toEqual({ ok: false, reason: "changed" });
  });
});
