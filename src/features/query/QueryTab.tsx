// Sorgu çalışma alanı (tembel yüklenir): tek CodeMirror editörü (sekme başına `EditorState`), bağlam şeridi,
// sonuç ızgarası / Mesajlar / Geçmiş. Çalıştırma akışı burada; sekme yaşam döngüsü `query-controller`dadır.
import { closeCompletion } from "@codemirror/autocomplete";
import type { EditorState } from "@codemirror/state";
import { Download, PanelRight } from "lucide-solid";
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { t } from "../../app/strings";
import { readStorage, writeStorage } from "../../app/storage";
import { toast } from "../../app/toast";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { FormatOptions } from "../../ipc/bindings/FormatOptions";
import type { QueryEvent } from "../../ipc/bindings/QueryEvent";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import type { TxAction } from "../../ipc/bindings/TxAction";
import type { TxMode } from "../../ipc/bindings/TxMode";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { isProductionLabel } from "../servers/server-form";
import { CellPanel } from "./CellPanel";
import { createRunConfirm } from "./ConfirmRunDialog";
import { ContextBar } from "./ContextBar";
import { canCheck, createCheckScheduler, type CheckGate } from "./check-model";
import { createCompletionSource } from "./completion";
import { runWithGuards, type Snapshot } from "./confirm-model";
import { createQueryEditor, type QueryEditor, type RunKind } from "./editor";
import { EditorSettings } from "./EditorSettings";
import { hintFor } from "./errors";
import { exportSql, exportTarget } from "./export";
import { createExportUi } from "./ExportUi";
import { formatRange, formatReplacement, safeFormatSelection } from "./format-edit";
import { createSerialSaver } from "./settings-saver";
import { historyEntriesFor, insertAt } from "./history-model";
import { HistorySettings, HistoryView } from "./HistoryView";
import { MessagesView } from "./MessagesView";
import { errorRange } from "./offsets";
import type { QueryController, TabNote } from "./query-controller";
import { ResultGrid, type ActiveCell } from "./ResultGrid";
import { ResultStrip } from "./ResultStrip";
import { MAX_TAB_ROWS } from "./result-model";
import { canCancel, canCommit, canLoadMoreRows, canRollback } from "./run-model";
import { isQueryTabId } from "./tabs-store";
import { routeError, sameScope, swapEditorState, type SavedView } from "./workspace-model";

/** Arka uç geçmiş sınırı (`MAX_HISTORY`). */
const HISTORY_MAX = 500;

/** `qt_run` satır sınırı (spec: varsayılan 10.000; sekme sınırı `MAX_TAB_ROWS` = 50.000'i aşmaz). */
export const ROW_LIMIT = Math.min(10_000, MAX_TAB_ROWS);

const EDITOR_HEIGHT_KEY = "pgt.query.editorHeight";
const MIN_EDITOR = 80;

/** Arka uç `pgt_query::format::FORMAT_FAILED` (belirteç eşitliği/eşkuvvetlilik bozuldu). */
const FORMAT_FAILED = "biçimlendirilemedi";
const DEFAULT_FORMAT: FormatOptions = { keywordCase: "upper", indent: 2, comma: "trailing", maxLine: 120 };

type ResultPane = "result" | "messages" | "history";
type EditorPrefs = { checkEnabled: boolean; format: FormatOptions };

export default function QueryTab(props: { ctl: QueryController }) {
  const ctl = props.ctl;
  const r = t.query.results;
  /** Etkin sorgu sekmesi; "Nesne" ya da dashboard etkinse `null` (editör geçişi/çalıştırma/denetim yok). */
  const activeId = () => {
    const a = ctl.tabs.state.activeId;
    return isQueryTabId(ctl.tabs.state, a) ? a : null;
  };
  /** Durumu editörde olan sekme. */
  const [shownId, setShownId] = createSignal<string | null>(null);
  const model = () => {
    const id = shownId();
    return id ? ctl.model(id) : undefined;
  };
  const config = () => ctl.serverConfig(model()?.serverId ?? null);

  const runConfirm = createRunConfirm();
  const exportUi = createExportUi();
  const [pane, setPane] = createSignal<ResultPane>("result");
  const [cell, setCell] = createSignal<ActiveCell | null>(null);
  const [cellPanel, setCellPanel] = createSignal(false);
  const [historyEnabled, setHistoryEnabled] = createSignal(true);
  const toggleHistory = async (enabled: boolean) => {
    try {
      setHistoryEnabled((await commands.settingsSetHistory(enabled)).historyEnabled);
    } catch (e) {
      notify(toPayload(e).message);
    }
  };
  // Editör ayarları (settings.json): yazarken denetim + biçimlendirici seçenekleri.
  // Değişiklik hemen uygulanır, yazımlar sıralıdır; başarısız yazım geri alınır (`settings-saver`).
  const [editorPrefs, setEditorPrefs] = createSignal<EditorPrefs>({ checkEnabled: true, format: DEFAULT_FORMAT });
  const checkEnabled = () => editorPrefs().checkEnabled;
  const formatOptions = () => editorPrefs().format;
  const prefsSaver = createSerialSaver<EditorPrefs>({
    get: editorPrefs,
    set: (v) => setEditorPrefs(v),
    save: (v) => commands.settingsSetEditor(v.checkEnabled, v.format),
    failed: (e) => notify(toPayload(e).message),
  });
  const saveEditorSettings = (check: boolean, format: FormatOptions) => void prefsSaver.save({ checkEnabled: check, format });
  const [editorHeight, setEditorHeight] = createSignal(Math.max(MIN_EDITOR, Number(readStorage(EDITOR_HEIGHT_KEY)) || 220));

  let host!: HTMLDivElement;
  let editor: QueryEditor | undefined;
  const book = new Map<string, SavedView<EditorState>>();

  const notify = (text: string, kind: "error" | "info" | "success" = "error") => toast.show(text, kind);
  const addNote = (id: string, note: TabNote) => ctl.update(id, (m) => ({ ...m, notes: [...m.notes, note] }));

  // ---- editör
  const complete = createCompletionSource({
    fetch: async (request) => {
      const s = model()?.session;
      return s ? commands.qtComplete(s, request) : [];
    },
    isBusy: () => {
      const m = model();
      return !m || !m.session || m.run.status !== "idle" || m.op !== null;
    },
  });

  // ---- yazarken denetim (tasarım §4): yalnız gösterilen + etkin sekme, açık/boşta oturum, Failed dışı.
  const checkGate = (): CheckGate | null => {
    const id = shownId();
    const m = model();
    if (!id || !m || activeId() !== id) return null;
    return {
      enabled: checkEnabled() && !m.checkOff,
      session: m.session,
      status: m.run.status,
      op: m.op,
      tx: m.run.tx,
      serverGone: m.serverGone,
    };
  };
  const checker = createCheckScheduler({
    gate: checkGate,
    view: () => editor?.checkView() ?? null,
    request: commands.qtCheck,
    apply: (marks) => editor?.setLintMarks(marks),
    // Sessiz (tasarım §6): yalnızca hata kodu, SQL metni yok.
    failed: (e) => console.debug("denetim başarısız", toPayload(e).code),
  });
  /** Hedef ne olursa olsun yeniden denetle (sekme geçişi, oturum boşa döndü). */
  const recheck = () => {
    checker.invalidate();
    checker.moved();
  };
  createEffect(on(() => canCheck(checkGate()), (ok) => (ok ? recheck() : checker.invalidate())));
  // Denetim kapatılınca (ayar ya da sekme düğmesi) işaretler kalkar.
  createEffect(
    on(
      () => checkEnabled() && !(model()?.checkOff ?? false),
      (on_) => {
        if (!on_) editor?.setLintMarks([]);
      },
      { defer: true },
    ),
  );
  onCleanup(() => checker.dispose());

  const snapView = (): SavedView<EditorState> | null =>
    editor ? { state: editor.getState(), scrollTop: editor.view.scrollDOM.scrollTop, scrollLeft: editor.view.scrollDOM.scrollLeft } : null;

  onMount(() => {
    editor = createQueryEditor(host, "", {
      onRun: (kind: RunKind) => void run(kind),
      onCancel: () => cancel(),
      onChange: (doc) => {
        const id = shownId();
        if (id) ctl.setText(id, doc);
      },
      onUpdate: ({ edited }) => (edited ? checker.edited() : checker.moved()),
      onFormat: () => void formatNow(),
      complete,
    });
    ctl.setRunner((kind) => void run(kind));
    commands
      .settingsGet()
      .then((s) => {
        setHistoryEnabled(s.historyEnabled);
        setEditorPrefs({ checkEnabled: s.checkEnabled, format: s.format });
      })
      .catch(() => {});
    onCleanup(() => {
      ctl.setRunner(null);
      editor?.destroy();
      editor = undefined;
    });
    switchTo(activeId());
  });

  /** Etkin sekme değişince: eski sekmenin durumu/kaydırması saklanır, yenisininki yüklenir. */
  const switchTo = (id: string | null) => {
    if (!editor || id === null) return;
    const prev = shownId();
    const prevAlive = prev !== null && ctl.model(prev) !== undefined;
    const target = swapEditorState(book, prevAlive ? prev : null, prevAlive ? snapView() : null, id, () =>
      editor!.createState(ctl.text(id), { readOnly: ctl.model(id)?.serverGone ?? false }),
    );
    // Sunucusu sonradan silinen sekmenin kayıtlı durumu düzenlenebilir kalmasın.
    if (ctl.model(id)?.serverGone && !target.state.readOnly) {
      target.state = editor.createState(target.state.doc.toString(), { readOnly: true });
      book.set(id, target);
      if (prev === id) editor.setState(target.state);
    }
    if (prev !== id) {
      editor.setState(target.state);
      setShownId(id);
      recheck();
      setCell(null);
      setPane("result");
      requestAnimationFrame(() => {
        if (!editor) return;
        editor.view.scrollDOM.scrollTop = target.scrollTop;
        editor.view.scrollDOM.scrollLeft = target.scrollLeft;
      });
    }
  };
  createEffect(on(activeId, (id) => untrack(() => switchTo(id)), { defer: true }));
  // Gösterilen sekmenin sunucusu silinirse editör salt okunur olur.
  createEffect(
    on(
      () => model()?.serverGone ?? false,
      (gone) => {
        if (!gone || !editor || editor.getState().readOnly) return;
        const s = editor.view.scrollDOM;
        const top = s.scrollTop;
        editor.setState(editor.createState(editor.getState().doc.toString(), { readOnly: true }));
        s.scrollTop = top;
      },
    ),
  );

  // Kapanan sekmelerin defter kayıtları.
  createEffect(() => {
    const ids = new Set(ctl.tabs.state.tabs.map((x) => x.id));
    for (const k of [...book.keys()]) if (!ids.has(k)) book.delete(k);
    const s = untrack(shownId);
    if (s !== null && !ids.has(s)) setShownId(null);
  });

  // ---- çalıştırma
  const blocked = (id: string): boolean => {
    const m = ctl.model(id);
    if (!m || m.serverGone || m.op !== null) return true;
    return m.run.status === "running" || m.run.status === "cancelling" || m.run.status === "opening";
  };

  async function run(kind: RunKind) {
    const id = shownId();
    if (!id || !editor || activeId() !== id || blocked(id)) return;
    const session = await ctl.ensureSession(id);
    if (!session) return;
    const m = ctl.model(id);
    if (!m || m.run.status !== "idle" || m.op !== null) return;
    // Anlık görüntü oturum hazır olduktan SONRA (parola istemi sırasında yazılanlar dahil): analiz, onay ve
    // çalıştırma bu tek metin/kapsamı kullanır. Bu arada başka sekmeye geçildiyse çalıştırma yok.
    if (!editor || shownId() !== id || activeId() !== id) return;
    const snap: Snapshot = editor.getSnapshot(kind);
    if (editor && shownId() === id) {
      closeCompletion(editor.view); // tamamlama oturum kilidini (≤5 sn) tutabilir
      editor.setErrorMark(null);
    }
    // Arka uç süren denetimi kendisi keser; geç gelen sonucu yok saymak için sürüm artar.
    checker.invalidate();
    const runId = ctl.nextRunId();
    ctl.dispatch(id, { type: "runStart", runId });
    ctl.patch(id, { notes: [], last: { text: snap.text, scope: snap.scope }, selectedStmt: null });
    if (shownId() === id) setPane("result");
    let plans: StmtPlan[] = [];
    const production = isProductionLabel(ctl.serverConfig(m.serverId)?.label);
    const outcome = await runWithGuards(snap, {
      session,
      limit: ROW_LIMIT,
      analyze: async (s, text, scope) => (plans = await commands.qtAnalyze(s, text, scope)),
      run: commands.qtRun,
      confirm: (items) => runConfirm.ask(items, production),
      onEvent: (ev) => onEvent(id, runId, ev, snap, () => plans),
    });
    switch (outcome.kind) {
      case "done":
        return;
      case "cancelled":
        ctl.dispatch(id, { type: "runAborted", runId });
        return;
      case "empty":
        ctl.dispatch(id, { type: "runAborted", runId });
        addNote(id, { level: "info", text: t.query.confirm.empty });
        showMessages(id);
        return;
      case "locked":
        ctl.dispatch(id, { type: "runAborted", runId });
        addNote(id, { level: "error", text: t.query.confirm.lockedSummary, hint: t.query.hint.locked });
        for (const it of outcome.items) {
          const reason = it.reason ? t.query.confirm.locked[it.reason] : t.query.hint.locked;
          addNote(id, { level: "error", text: r.lockedItem(it.index + 1, reason) });
        }
        showMessages(id);
        return;
      case "rejected":
        ctl.dispatch(id, { type: "runRejected", runId, error: outcome.error });
        handleError(id, outcome.error, kind);
        return;
    }
  }

  const showMessages = (id: string) => {
    if (shownId() === id) setPane("messages");
  };

  function handleError(id: string, e: ErrorPayload, kind: RunKind) {
    switch (routeError(e)) {
      case "toast":
        notify(r.busy);
        return;
      case "reset":
        addNote(id, { level: "info", text: e.message, hint: hintFor(e), rerun: kind });
        showMessages(id);
        return;
      case "lost":
        ctl.sessionLost(id);
        showMessages(id);
        return;
      case "message":
        addNote(id, { level: "error", text: r.runError(e.message), hint: hintFor(e) });
        showMessages(id);
        return;
    }
  }

  function onEvent(id: string, runId: number, ev: QueryEvent, snap: Snapshot, plans: () => StmtPlan[]) {
    ctl.dispatch(id, { type: "event", runId, event: ev });
    // Çalışırken son satır döndüren ifadeyi göster (satırlar aktıkça ızgarada görünsün).
    if (ev.type === "started" && ev.columns.length > 0) ctl.patch(id, { selectedStmt: ev.stmtIndex });
    if (ev.type !== "scriptDone") return;
    const m = ctl.model(id);
    const res = m?.run.result;
    if (!m || !res || m.run.runId !== runId) return;
    // Geçmiş: çalıştırma ANINDAKİ metin (analiz edilen anlık görüntü).
    if (m.serverId) {
      const entries = historyEntriesFor(plans(), res, {
        text: snap.text,
        serverId: m.serverId,
        serverName: m.serverName,
        database: m.database,
        atMs: Date.now(),
        newId: () => crypto.randomUUID(),
      });
      // Tek çağrı, arka uçta tek dosya yazımı; yalnızca son MAX_HISTORY girdi anlamlıdır.
      if (entries.length > 0)
        void commands
          .qtHistoryPushMany(entries.slice(-HISTORY_MAX))
          .catch((err) => console.warn("geçmiş yazılamadı", toPayload(err).code))
          .finally(() => ctl.update(id, (x) => ({ ...x, historyKey: x.historyKey + 1 })));
    }
    void ctl.refreshTx(id);
    const withCols = [...res.stmts].reverse().find((s) => s.columns.length > 0 && !s.dropped);
    const last = res.stmts[res.stmts.length - 1];
    ctl.patch(id, { selectedStmt: (withCols ?? last)?.index ?? null });
    const failed = res.stmts.find((s) => s.status === "failed" && s.error);
    if (shownId() === id) {
      if (failed?.error && editor && editor.getState().doc.toString() === snap.text) {
        editor.setErrorMark(errorRange(snap.text, failed.error));
      }
      const problem = res.stmts.some((s) => s.status === "failed" || s.status === "cancelled");
      setPane(problem || !withCols ? "messages" : "result");
    }
  }

  const cancel = () => {
    const id = shownId();
    const m = id ? ctl.model(id) : undefined;
    if (!id || !m?.session || !canCancel(m.run)) return;
    ctl.dispatch(id, { type: "cancelRequested" });
    void commands.qtCancel(m.session).catch((e) => notify(toPayload(e).message));
  };

  const loadMore = async (stmtIndex: number) => {
    const id = shownId();
    const m = id ? ctl.model(id) : undefined;
    if (!id || !m?.session || m.op !== null || !canLoadMoreRows(m.run, stmtIndex)) return;
    const runId = m.run.runId;
    ctl.patch(id, { op: "more" });
    try {
      await commands.qtMore(m.session, stmtIndex, (ev) => ctl.dispatch(id, { type: "event", runId, event: ev }));
    } catch (err) {
      const e = toPayload(err);
      ctl.dispatch(id, { type: "optionsChanged" }); // "daha fazla" kapanır
      if (routeError(e) === "lost") ctl.sessionLost(id);
      else {
        addNote(id, { level: "error", text: r.moreFailed(e.message), hint: hintFor(e) });
        notify(r.moreFailed(e.message));
      }
    } finally {
      if (ctl.model(id)?.op === "more") ctl.patch(id, { op: null });
    }
  };

  const tx = async (action: TxAction) => {
    const id = shownId();
    const m = id ? ctl.model(id) : undefined;
    if (!id || !m?.session || m.op !== null) return;
    if (action === "commit" ? !canCommit(m.run) : !canRollback(m.run)) return;
    ctl.patch(id, { op: "tx" });
    try {
      ctl.dispatch(id, { type: "txChanged", tx: await commands.qtTx(m.session, action) });
    } catch (err) {
      const e = toPayload(err);
      const route = routeError(e);
      if (route === "lost") ctl.sessionLost(id);
      else if (route === "toast") notify(r.busy);
      else {
        addNote(id, { level: "error", text: r.txFailed(e.message), hint: hintFor(e) });
        showMessages(id);
      }
      void ctl.refreshTx(id);
    } finally {
      if (ctl.model(id)?.op === "tx") ctl.patch(id, { op: null });
    }
  };

  const setMode = (mode: TxMode) => {
    const id = shownId();
    const m = id ? ctl.model(id) : undefined;
    if (id && m) void ctl.setOptions(id, { ...m.options, mode });
  };
  const setTimeoutMs = (ms: number) => {
    const id = shownId();
    const m = id ? ctl.model(id) : undefined;
    if (id && m) void ctl.setOptions(id, { ...m.options, statementTimeoutMs: ms });
  };
  const reconnect = () => {
    const id = shownId();
    if (id) void ctl.ensureSession(id);
  };

  // ---- dışa aktarma
  const exportCsv = async () => {
    const id = shownId();
    if (!id || !editor || blocked(id)) return;
    const session = await ctl.ensureSession(id);
    const m = ctl.model(id);
    if (!session || !m || m.op !== null || m.run.status !== "idle") return;
    if (!editor || shownId() !== id) return;
    const snap = editor.getSnapshot("stmt");
    ctl.patch(id, { op: "export" });
    try {
      const plans = await commands.qtAnalyze(session, snap.text, snap.scope);
      // Şeritteki seçim yalnızca analiz kapsamı son çalıştırmanınkiyle aynıysa anlamlıdır (dizinler kapsam içidir).
      const preferred = m.last && m.last.text === snap.text && sameScope(m.last.scope, snap.scope) ? m.selectedStmt : null;
      const target = exportTarget(plans, true, preferred);
      if (!target) {
        notify(t.query.export.disabledHint, "info");
        return;
      }
      const outcome = await exportUi.start({ session, sql: exportSql(snap.text, target), relation: m.relation, stmtIndex: target.index });
      if (outcome.kind === "error" && routeError(outcome.error) === "lost") ctl.sessionLost(id);
      else void ctl.refreshTx(id);
    } catch (err) {
      const e = toPayload(err);
      if (routeError(e) === "lost") ctl.sessionLost(id);
      else notify(e.message);
    } finally {
      if (ctl.model(id)?.op === "export") ctl.patch(id, { op: null });
    }
  };

  // ---- biçimlendirme (Ctrl+Shift+F): seçim ya da tümü, tek geri alma adımı; oturum gerekmez.
  const formatNow = async () => {
    const id = shownId();
    const m = model();
    if (!editor || !id || !m || m.serverGone || editor.getState().readOnly) return;
    const state = editor.getState();
    const doc = state.doc.toString();
    const sel = state.selection.main;
    let range = formatRange(doc.length, sel.from, sel.to);
    if (!range.whole) {
      // Seçimin ucu dize/yorum/tırnaklı ad içinde ya da belirteç ortasındaysa çevreleyen ifadeye genişletilir.
      const safe = safeFormatSelection(doc, range.from, range.to);
      if (!safe) {
        notify(t.query.format.unsafeSelection, "info");
        return;
      }
      range = { ...range, from: safe.from, to: safe.to };
    }
    const original = doc.slice(range.from, range.to);
    if (original.trim() === "") return;
    try {
      const formatted = await commands.qtFormat(original, formatOptions());
      const insert = formatReplacement(original, formatted, range.whole);
      // Bu arada yazıldıysa ya da sekme değiştiyse sessizce vazgeç (metin değişmez).
      if (insert === null || !editor || shownId() !== id) return;
      editor.replaceFormatted(doc, range.from, range.to, insert);
    } catch (err) {
      const e = toPayload(err);
      notify(e.message === FORMAT_FAILED ? t.query.format.failed : `${t.query.format.failed} ${e.message}`);
    }
  };

  // ---- geçmişten ekleme
  const insertSql = (sql: string) => {
    const m = model();
    if (!editor || !m || m.serverGone) return;
    const v = editor.view;
    const sel = v.state.selection.main;
    const res = insertAt(v.state.doc.toString(), sel.from, sel.to, sql);
    v.dispatch({ changes: { from: sel.from, to: sel.to, insert: res.doc.slice(sel.from, res.cursor) }, selection: { anchor: res.cursor }, scrollIntoView: true });
    v.focus();
  };

  // ---- editör yüksekliği
  const onSplitDown = (e: PointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startH = editorHeight();
    const max = () => Math.max(MIN_EDITOR, (handle.parentElement?.clientHeight ?? 600) - 140);
    const move = (ev: PointerEvent) => setEditorHeight(Math.min(max(), Math.max(MIN_EDITOR, startH + ev.clientY - startY)));
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("lostpointercapture", up);
      writeStorage(EDITOR_HEIGHT_KEY, String(editorHeight()));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("lostpointercapture", up);
  };

  // ---- sonuç görünümü
  const result = () => model()?.run.result ?? null;
  const selectedStmt = createMemo(() => {
    const res = result();
    const idx = model()?.selectedStmt;
    if (!res || idx == null) return null;
    return res.stmts.find((s) => s.index === idx) ?? null;
  });
  const gridKey = createMemo(() => {
    const s = selectedStmt();
    return s && s.columns.length > 0 ? `${shownId()}:${result()?.runId}:${s.index}` : null;
  });
  const messageCount = () => (result()?.messages.length ?? 0) + (model()?.notes.length ?? 0);
  const canExportNow = () => {
    const m = model();
    return !!m && !m.serverGone && m.op === null && m.run.status !== "running" && m.run.status !== "cancelling" && m.run.status !== "opening";
  };

  const paneButton = (p: ResultPane, label: () => string) => (
    <button
      type="button"
      role="tab"
      id={`qt-pane-tab-${p}`}
      aria-controls="qt-pane-panel"
      aria-selected={pane() === p}
      class="border-b-2 border-transparent px-2 py-1.5 text-[12px] text-muted aria-selected:border-accent aria-selected:text-fg"
      onClick={() => setPane(p)}
    >
      {label()}
    </button>
  );

  return (
    <div class="flex h-full min-h-0 flex-col">
      <Show when={model()}>
        {(m) => (
          <ContextBar
            model={m()}
            config={config()}
            onRun={() => void run("stmt")}
            onRunAll={() => void run("all")}
            onCancel={cancel}
            onCommit={() => void tx("commit")}
            onRollback={() => void tx("rollback")}
            onMode={setMode}
            onTimeout={setTimeoutMs}
            onReconnect={reconnect}
            checkEnabled={checkEnabled()}
            onToggleCheck={() => {
              const id = shownId();
              const cur = id ? ctl.model(id) : undefined;
              if (id && cur) ctl.patch(id, { checkOff: !cur.checkOff });
            }}
          />
        )}
      </Show>
      <div
        ref={host}
        role="region"
        aria-label={t.query.tabs.editorAria}
        class="min-h-0 shrink-0 overflow-hidden"
        style={{ height: `${editorHeight()}px` }}
      />
      <div
        role="separator"
        aria-orientation="horizontal"
        aria-label={t.query.tabs.resizeEditor}
        class="h-1 shrink-0 cursor-row-resize border-y border-border bg-panel hover:bg-accent/40"
        onPointerDown={onSplitDown}
      />
      <div class="flex min-h-0 flex-1 flex-col">
        <div class="flex shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
          <div role="tablist" aria-label={r.panesAria} class="flex gap-1">
            {paneButton("result", () => r.result)}
            {paneButton("messages", () => (messageCount() > 0 ? `${r.messages} (${messageCount()})` : r.messages))}
            {paneButton("history", () => r.history)}
          </div>
          <div class="flex-1" />
          <Button
            variant="ghost"
            class="h-6 text-[12px]"
            aria-pressed={cellPanel()}
            title={r.cellPanel}
            aria-label={r.cellPanel}
            onClick={() => setCellPanel(!cellPanel())}
          >
            <PanelRight size={13} />
          </Button>
          <Button variant="ghost" class="h-6 text-[12px]" disabled={!canExportNow()} title={t.query.export.disabledHint} onClick={() => void exportCsv()}>
            <Download size={13} /> {t.query.export.button}
          </Button>
        </div>
        <div class="min-h-0 flex-1" role="tabpanel" id="qt-pane-panel" aria-labelledby={`qt-pane-tab-${pane()}`}>
          <Switch>
            <Match when={pane() === "result"}>
              <div class="flex h-full min-h-0 flex-col">
                <Show when={(result()?.stmts.length ?? 0) > 1}>
                  <ResultStrip
                    result={result()}
                    selected={model()?.selectedStmt ?? null}
                    onSelect={(index) => {
                      const id = shownId();
                      if (id) ctl.patch(id, { selectedStmt: index });
                    }}
                  />
                </Show>
                <div class="flex min-h-0 flex-1">
                  <div class="min-w-0 flex-1">
                    <Show
                      when={gridKey()}
                      keyed
                      fallback={<p class="p-3 text-[13px] text-muted">{selectedStmt() ? r.noColumns : r.none}</p>}
                    >
                      {(_key) => (
                        <Show when={selectedStmt()}>
                          {(s) => (
                            <ResultGrid
                              result={s()}
                              canLoadMore={(() => {
                                const m = model();
                                return !!m && m.op === null && canLoadMoreRows(m.run, s().index);
                              })()}
                              onLoadMore={() => void loadMore(s().index)}
                              onActiveCell={setCell}
                            />
                          )}
                        </Show>
                      )}
                    </Show>
                  </div>
                  <Show when={cellPanel()}>
                    <div class="w-72 shrink-0 border-l border-border">
                      <CellPanel cell={cell()} />
                    </div>
                  </Show>
                </div>
              </div>
            </Match>
            <Match when={pane() === "messages"}>
              <div class="flex h-full min-h-0 flex-col">
                <Show when={(model()?.notes.length ?? 0) > 0}>
                  <ul class="shrink-0 border-b border-border p-2 font-mono text-xs">
                    <For each={model()?.notes ?? []}>
                      {(n) => (
                        <li class="mb-1">
                          <span classList={{ "text-danger": n.level === "error", "text-fg": n.level === "info" }}>{n.text}</span>
                          <Show when={n.hint}>
                            <div class="pl-3 text-muted">
                              {t.query.messages.hint}: {n.hint}
                            </div>
                          </Show>
                          <Show when={n.rerun}>
                            {(kind) => (
                              <Button class="mt-1 h-6 text-[12px]" onClick={() => void run(kind())}>
                                {r.rerun}
                              </Button>
                            )}
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
                <div class="min-h-0 flex-1">
                  <MessagesView messages={result()?.messages ?? []} />
                </div>
              </div>
            </Match>
            <Match when={pane() === "history"}>
              <div class="flex h-full min-h-0 flex-col">
                <div class="min-h-0 flex-1">
                  <HistoryView onInsert={insertSql} refreshKey={model()?.historyKey ?? 0} />
                </div>
                <div class="flex shrink-0 flex-wrap gap-x-8 gap-y-3 border-t border-border p-2">
                  <HistorySettings
                    enabled={historyEnabled()}
                    onToggle={(v) => void toggleHistory(v)}
                    onCleared={() => {
                      const id = shownId();
                      if (id) ctl.update(id, (x) => ({ ...x, historyKey: x.historyKey + 1 }));
                      // Temizleme yalnızca geçmişi ve kapalı sekmelerin taslaklarını silmeli: açık sekmeler yeniden yazılır.
                      void ctl.resaveDrafts();
                    }}
                  />
                  <EditorSettings
                    checkEnabled={checkEnabled()}
                    format={formatOptions()}
                    onChange={saveEditorSettings}
                  />
                </div>
              </div>
            </Match>
          </Switch>
        </div>
      </div>
      <runConfirm.View />
      <exportUi.View />
    </div>
  );
}
