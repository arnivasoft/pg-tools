import { describe, expect, it } from "vitest";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { TxState } from "../../ipc/bindings/TxState";
import { USER_CANCEL_MESSAGE } from "./result-model";
import {
  canCancel,
  canCommit,
  canLoadMoreRows,
  canRollback,
  closeChoices,
  closeNeedsConfirm,
  initialRun,
  runReduce,
  type RunEvent,
  type RunState,
  type TabStatus,
} from "./run-model";

const reduce = (evs: RunEvent[], s: RunState = initialRun()) => evs.reduce(runReduce, s);
const ready = () => reduce([{ type: "open" }, { type: "opened" }]);
const ev = (runId: number, event: QueryEvent): RunEvent => ({ type: "event", runId, event });
const scriptDone = (runId: number, stoppedOnError = false) => ev(runId, { type: "scriptDone", stmtsRun: 1, stoppedOnError });
const st = (status: TabStatus, tx: TxState = "idle"): RunState => ({ status, tx, runId: 0, result: null });

describe("runReduce", () => {
  it("açılış", () => {
    expect(reduce([{ type: "open" }]).status).toBe("opening");
    expect(ready().status).toBe("idle");
    expect(reduce([{ type: "open" }, { type: "openFailed" }]).status).toBe("noSession");
    expect(reduce([{ type: "open" }, { type: "opened", tx: "inTransaction" }]).tx).toBe("inTransaction");
  });

  it("çalıştırma scriptDone ile biter", () => {
    let s = reduce([{ type: "runStart", runId: 1 }], ready());
    expect(s.status).toBe("running");
    expect(s.result?.runId).toBe(1);
    s = runReduce(s, ev(1, { type: "started", stmtIndex: 0, kind: "select", columns: [], charOffset: 0 }));
    expect(s.status).toBe("running");
    s = runReduce(s, scriptDone(1));
    expect(s.status).toBe("idle");
    expect(s.result?.done).toBe(true);
  });

  it("eski runId olayları yok sayılır", () => {
    const s = reduce([{ type: "runStart", runId: 1 }, scriptDone(1), { type: "runStart", runId: 2 }], ready());
    expect(runReduce(s, scriptDone(1))).toBe(s);
    expect(runReduce(s, { type: "runRejected", runId: 1, error: { code: "busy", message: "", detail: null, hint: null } })).toBe(s);
  });

  it("runStart yalnızca idle'da ve artan kimlikle", () => {
    const r = reduce([{ type: "runStart", runId: 1 }], ready());
    expect(runReduce(r, { type: "runStart", runId: 2 })).toBe(r);
    const done = runReduce(r, scriptDone(1));
    expect(runReduce(done, { type: "runStart", runId: 1 })).toBe(done);
    expect(runReduce(st("noSession"), { type: "runStart", runId: 1 }).status).toBe("noSession");
  });

  it("runRejected (olaysız red) idle'a döner", () => {
    const s = reduce(
      [{ type: "runStart", runId: 1 }, { type: "runRejected", runId: 1, error: { code: "busy", message: "", detail: null, hint: null } }],
      ready(),
    );
    expect(s.status).toBe("idle");
  });

  it("runAborted (onay reddi / kilit / boş) idle'a döner; eski çalıştırmanınki yok sayılır", () => {
    const s = reduce([{ type: "runStart", runId: 1 }, { type: "runAborted", runId: 1 }], ready());
    expect(s.status).toBe("idle");
    expect(s.result?.stmts).toEqual([]);
    const r = reduce([{ type: "runStart", runId: 2 }], ready());
    expect(runReduce(r, { type: "runAborted", runId: 1 })).toBe(r);
  });

  it("iptal: running → cancelling → (failed + scriptDone) → idle", () => {
    let s = reduce([{ type: "runStart", runId: 1 }, { type: "cancelRequested" }], ready());
    expect(s.status).toBe("cancelling");
    expect(canCancel(s)).toBe(true);
    s = runReduce(s, ev(1, { type: "failed", stmtIndex: 0, error: { code: "query", message: USER_CANCEL_MESSAGE, detail: null, hint: null }, position: null, charOffset: 0, notices: [] }));
    expect(s.result?.stmts[0].status).toBe("cancelled");
    s = runReduce(s, scriptDone(1, true));
    expect(s.status).toBe("idle");
    expect(runReduce(ready(), { type: "cancelRequested" }).status).toBe("idle");
  });

  it("sessionLost ve yeniden açma", () => {
    const s = reduce([{ type: "txChanged", tx: "inTransaction" }, { type: "sessionLost" }], ready());
    expect(s).toMatchObject({ status: "lost", tx: "idle" });
    expect(runReduce(s, { type: "open" }).status).toBe("opening");
  });

  it("optionsChanged sonrası daha fazla kapanır", () => {
    const s = reduce(
      [
        { type: "runStart", runId: 1 },
        ev(1, { type: "started", stmtIndex: 0, kind: "select", columns: [{ name: "a", typeName: "int4", typeOid: 23, typeDisplay: "integer" }], charOffset: 0 }),
        ev(1, { type: "rows", stmtIndex: 0, rows: [["1"]], more: true }),
        ev(1, { type: "done", stmtIndex: 0, tag: "SELECT 1", rowsAffected: null, rowsSent: 1, truncated: true, elapsedMs: 1, notices: [] }),
        scriptDone(1),
      ],
      ready(),
    );
    expect(canLoadMoreRows(s, 0)).toBe(true);
    expect(canLoadMoreRows(runReduce(s, { type: "optionsChanged" }), 0)).toBe(false);
    // yeni çalıştırma başlayınca eski ızgaradan istenemez
    expect(canLoadMoreRows(runReduce(s, { type: "runStart", runId: 2 }), 0)).toBe(false);
  });
});

describe("seçiciler", () => {
  const table: [TabStatus, TxState, boolean, boolean, boolean, boolean, boolean][] = [
    // status, tx, boşta (çalıştırılabilir), canCancel, canCommit, canRollback, closeNeedsConfirm
    ["noSession", "idle", false, false, false, false, false],
    ["opening", "idle", false, false, false, false, false],
    ["idle", "idle", true, false, false, false, false],
    ["idle", "inTransaction", true, false, true, true, true],
    ["idle", "failed", true, false, false, true, true],
    ["running", "idle", false, true, false, false, false],
    ["running", "inTransaction", false, true, false, false, true],
    ["cancelling", "failed", false, true, false, false, true],
    ["lost", "idle", false, false, false, false, false],
  ];
  it.each(table)("%s/%s", (status, tx, run, cancel, commit, rollback, confirm) => {
    const s = st(status, tx);
    expect([s.status === "idle", canCancel(s), canCommit(s), canRollback(s), closeNeedsConfirm(s)]).toEqual([run, cancel, commit, rollback, confirm]);
  });

  it("kapanış seçenekleri", () => {
    expect(closeChoices(st("idle", "idle"))).toEqual([]);
    expect(closeChoices(st("idle", "inTransaction"))).toEqual(["commit", "rollback", "cancel"]);
    expect(closeChoices(st("idle", "failed"))).toEqual(["rollback", "cancel"]);
    expect(closeChoices(st("running", "inTransaction"))).toEqual(["rollback", "cancel"]);
  });
});
