// Silme diyaloğunun durumu (tembel parça; sekme açmaz, tek örnek): `ddl_load` → `def` (derin kopya, olduğu gibi
// `Drop` isteğine gider) → önizleme → kapılı uygulama → Explorer yenileme ve açık alter sekmelerinin `gone` olması.
//
// Kurallar:
// - Durum makinesi: loading → ready ⇄ applying; `error` uç durumdur (yükleme hatası, nesne değişti/yok, kısmi silme):
//   yalnızca Vazgeç (ve uygunsa yeniden yükleme). Bağlantı kodları diyaloğu bildirimle kapatır.
// - Kuşak sayacı: açma/kapama/yeniden yükleme kuşağı artırır; eski kuşağın yanıtı durum yazmaz.
// - `cascade` değişince sürüm ilerler, onay sıfırlanır, önizleme beklemeden yeniden istenir (en son istek kazanır).
// - Uygulama tek uçuştadır; kapı çağrı anında sunucu kaydıyla yeniden hesaplanır, `applyOptionsFor` null ise hiçbir şey
//   gönderilmez. Uçuştayken kapatılırsa `ddl_cancel` istenir; geç gelen başarı yine Gezgini yeniler ve açık alter
//   sekmelerini `gone` yapar (nesne gerçekten silinmiş olabilir) ama diyalog durumu yazılmaz.
import { createSignal } from "solid-js";
import { tdes } from "../../app/strings-designer";
import type { ToastKind } from "../../app/toast-model";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Api } from "../../ipc/commands";
import { errorText, toPayload } from "../../ipc/errors";
import { applyGate, applyOptionsFor, canSubmit, cloneDef, type ApplyGate, type ConfirmState } from "./designer-model";
import { dropRequest, dropStatusText, type DropStatus } from "./drop-model";
import { isConnectionCode } from "./errors";
import { createPreviewer, type PreviewDeps } from "./preview";

export interface DropInput { serverId: string; databaseOid: number; object: ObjectRef; refreshNodeId: string | null; name: string }

export type DropPhase = "loading" | "ready" | "applying" | "error";

export interface DropState {
  input: DropInput;
  phase: DropPhase;
  /** `ddl_load` çıktısı (DEĞİŞTİRİLMEZ; isteğe derin kopyası gider). */
  def: ObjectDef | null;
  cascade: boolean;
  view: PlanView | null;
  viewFor: number;
  version: number;
  pending: boolean;
  previewError: string | null;
  confirm: ConfirmState;
  /** Uygulama/yükleme şeridi (`role="alert"`). */
  status: DropStatus | null;
  /** `error` aşamasında yeniden yükleme anlamlı mı (genel yükleme hatası, plan değişimi). */
  canReload: boolean;
  serverName: string;
  locked: boolean;
  production: boolean;
}

export interface DropControllerDeps {
  api: Pick<Api, "ddlLoad" | "ddlPreview" | "ddlApply" | "ddlCancel">;
  notify(text: string, kind: ToastKind): void;
  serverInfo(serverId: string): { name: string; locked: boolean; production: boolean } | null;
  /** Explorer düğüm yenilemesi (hatası yutulur). */
  refreshNode(nodeId: string): void;
  /** Başarılı silme: aynı nesnenin alter sekmeleri `gone`. */
  objectDropped(serverId: string, databaseOid: number, object: ObjectRef): void;
  lockTimeoutMs: number;
  previewDeps: PreviewDeps;
  newId: () => string;
}

const emptyConfirm = (): ConfirmState => ({ checked: false, typedName: "" });

interface PreviewOk { view: PlanView; version: number; gen: number }
class PreviewFailure {
  constructor(readonly error: unknown, readonly version: number, readonly gen: number) {}
}

export function createDropController(deps: DropControllerDeps) {
  const { api, notify } = deps;
  const [state, setState] = createSignal<DropState | null>(null);
  let gen = 0;
  let applyId: string | null = null;

  const patch = (p: Partial<DropState>) => setState((s) => (s ? { ...s, ...p } : s));
  const cancelQuietly = (id: string) => {
    try {
      void api.ddlCancel(id).catch(() => {});
    } catch {
      /* eşzamanlı hata da yok sayılır */
    }
  };
  const quiet = (f: () => void) => {
    try {
      f();
    } catch {
      /* yenileme/sekme hatası silme sonucunu etkilemez */
    }
  };

  /** Gezgin düğümü yenilenir (silme, nesne değişti/yok, kısmi silme); hata yutulur. */
  function refreshTree(input: DropInput) {
    const id = input.refreshNodeId;
    if (id !== null) quiet(() => void Promise.resolve(deps.refreshNode(id)).catch(() => {}));
  }

  const onPreview = (r: PreviewOk) => {
    const s = state();
    if (!s || r.gen !== gen) return;
    const current = r.version === s.version;
    const samePlan = s.view !== null && s.view.hash === r.view.hash;
    patch({
      view: r.view, viewFor: r.version, pending: current ? false : s.pending, previewError: current ? null : s.previewError,
      confirm: samePlan ? s.confirm : emptyConfirm(),
    });
  };
  const onPreviewError = (x: unknown) => {
    const s = state();
    if (!s || !(x instanceof PreviewFailure) || x.gen !== gen) return;
    const st = dropStatusText(toPayload(x.error));
    if (!st) return;
    if (st.kind === "disconnected") return closeWith(st.text);
    if (st.kind === "stale") {
      refreshTree(s.input);
      return patch({ phase: "error", status: st, pending: false, view: null, viewFor: -1, canReload: false });
    }
    if (x.version === s.version) patch({ pending: false, previewError: st.text });
  };
  const previewer = createPreviewer<PreviewOk>(deps.previewDeps, onPreview, onPreviewError, 0);

  /** Mevcut tanım + cascade için önizleme (beklemeden; en son istek kazanır). Yalnızca `ready`. */
  const preview = () => {
    const s = state();
    if (!s || s.phase !== "ready" || !s.def) return;
    const req = dropRequest(s.input.object, s.def, s.cascade);
    const { version } = s;
    const g = gen;
    patch({ pending: true });
    previewer.runNow(() =>
      api.ddlPreview(s.input.serverId, s.input.databaseOid, req).then(
        (view): PreviewOk => ({ view, version, gen: g }),
        (err: unknown) => { throw new PreviewFailure(err, version, g); },
      ),
    );
  };

  /** Diyaloğu kapatır (kuşak artar; uçuştaki uygulama için `ddl_cancel`). */
  const close = () => {
    gen++;
    previewer.cancel();
    if (applyId) cancelQuietly(applyId);
    applyId = null;
    setState(null);
  };
  function closeWith(text: string) {
    close();
    notify(text, "error");
  }

  const load = async () => {
    const s = state();
    if (!s) return;
    const g = ++gen;
    previewer.cancel();
    patch({ phase: "loading", def: null, view: null, viewFor: -1, pending: false, previewError: null, status: null, confirm: emptyConfirm(), canReload: false });
    try {
      const def = await api.ddlLoad(s.input.serverId, s.input.databaseOid, s.input.object);
      if (g !== gen) return;
      setState((cur) => (cur ? { ...cur, phase: "ready", def: cloneDef(def), version: cur.version + 1 } : cur));
      preview();
    } catch (err) {
      if (g !== gen) return;
      const p = toPayload(err);
      if (isConnectionCode(p.code)) return closeWith(errorText(p));
      const st = dropStatusText(p);
      const general = st?.kind === "other";
      if (st?.kind === "stale") refreshTree(s.input);
      patch({ phase: "error", status: general ? { kind: "other", text: errorText(p), detail: null } : st, canReload: general });
    }
  };

  /** Diyaloğu açar (tek örnek: açık diyalog varsa yerini alır). */
  const open = (input: DropInput) => {
    close();
    const info = deps.serverInfo(input.serverId);
    setState({
      input: { ...input, object: { ...input.object } }, phase: "loading", def: null, cascade: false,
      view: null, viewFor: -1, version: 0, pending: false, previewError: null, confirm: emptyConfirm(), status: null,
      canReload: false, serverName: info?.name ?? "", locked: info?.locked ?? false, production: info?.production ?? false,
    });
    void load();
  };

  const gateFor = (s: DropState, fresh: boolean): ApplyGate => {
    const info = fresh ? deps.serverInfo(s.input.serverId) : null;
    return applyGate({
      locked: s.locked || (info?.locked ?? false),
      production: s.production || (info?.production ?? false),
      serverName: fresh ? (info?.name ?? "") : s.serverName,
      view: s.view,
      viewIsCurrent: s.view !== null && s.viewFor === s.version,
      pending: s.pending,
      applying: s.phase === "applying",
      connected: !fresh || info !== null,
      loading: s.phase === "loading" || s.phase === "error",
    });
  };

  const setCascade = (v: boolean) => {
    const s = state();
    if (!s || s.phase !== "ready" || s.cascade === v) return;
    patch({ cascade: v, version: s.version + 1, confirm: emptyConfirm(), status: null });
    preview();
  };
  const setConfirm = (c: Partial<ConfirmState>) => {
    const s = state();
    if (!s || s.phase === "applying") return;
    patch({ confirm: { ...s.confirm, ...c } });
  };

  /** Silindi: Gezgin düğümü yenilenir, aynı nesnenin alter sekmeleri `gone` (hataları yutulur). */
  const succeeded = (input: DropInput) => {
    refreshTree(input);
    quiet(() => deps.objectDropped(input.serverId, input.databaseOid, input.object));
  };

  /** Kapı → `ddl_apply` → sonuç. Kapı kapalıysa ya da uçuştaysa hiçbir şey yapmaz. */
  const apply = async (): Promise<void> => {
    const s = state();
    if (!s || s.phase !== "ready" || !s.def || !s.view) return;
    const gate = gateFor(s, true);
    if (!canSubmit(gate, s.confirm)) return;
    const options = applyOptionsFor(deps.newId(), deps.lockTimeoutMs, s.view, gate, s.confirm);
    if (!options) return;
    const req = dropRequest(s.input.object, s.def, s.cascade);
    const g = gen;
    const input = s.input;
    previewer.cancel();
    applyId = options.applyId;
    patch({ phase: "applying", status: null, pending: false });
    let out: ApplyOutcome;
    try {
      out = await api.ddlApply(input.serverId, input.databaseOid, req, options);
    } catch (err) {
      if (g !== gen) return;
      applyId = null;
      const st = dropStatusText(toPayload(err))!;
      if (st.kind === "disconnected") return closeWith(st.text);
      if (st.kind === "stale") refreshTree(input);
      // Görünüm (hash) tüketildi; onay sıfırlanır.
      patch({ phase: st.kind === "stale" ? "error" : "ready", status: st, view: null, viewFor: -1, confirm: emptyConfirm(), locked: st.kind === "locked" || s.locked, canReload: false });
      if (st.kind === "other") preview();
      return;
    }
    if (out.outcome === "applied") {
      // Kapatılmış diyalogun geç başarısı da Gezgini ve sekmeleri günceller ve bildirilir (nesne silindi);
      // diyalog yalnızca hâlâ aynı kuşaktaysa kapatılır (yeni açılan diyaloğa dokunulmaz).
      succeeded(input);
      notify(tdes.drop.done(input.name), "success");
      if (g !== gen) return;
      applyId = null;
      close();
      return;
    }
    if (g !== gen) return;
    applyId = null;
    const st = dropStatusText(null, out)!;
    // Kısmi silme (committed > 0): nesne değişmiş olabilir; Gezgin yenilenir.
    if (st.kind === "stale") refreshTree(input);
    patch({ phase: st.kind === "stale" ? "error" : "ready", status: st, view: null, viewFor: -1, confirm: emptyConfirm(), canReload: false });
    if (st.kind !== "stale") preview();
  };

  /** Uçuştaki uygulamayı iptal ister; sonuç `apply`'ın yanıtıyla gelir. */
  const cancelApply = () => {
    const s = state();
    if (s?.phase === "applying" && applyId) cancelQuietly(applyId);
  };

  /** Sunucu kaydı değişti: ad/kilit/üretim yenilenir; biri değiştiyse onay sıfırlanır. */
  const serverChanged = (serverId: string) => {
    const s = state();
    const info = deps.serverInfo(serverId);
    if (!s || !info || s.input.serverId !== serverId) return;
    if (s.serverName === info.name && s.locked === info.locked && s.production === info.production) return;
    patch({ serverName: info.name, locked: info.locked, production: info.production, confirm: emptyConfirm() });
  };
  /** Sunucunun oturumları kapandı (sil/kes/kilit): diyalog bildirimsiz kapanır. */
  const serverSessionsClosed = (serverId: string) => {
    if (state()?.input.serverId === serverId) close();
  };

  return {
    state,
    open,
    close,
    reload: () => {
      const s = state();
      if (s?.phase === "error" && s.canReload) void load();
    },
    setCascade,
    setConfirm,
    apply,
    cancelApply,
    serverChanged,
    serverSessionsClosed,
    /** Arayüz kapısı (denetleyici `apply` anında yeniden hesaplar). */
    gate: (): ApplyGate => {
      const s = state();
      return s ? gateFor(s, false) : { kind: "blocked", reason: "disconnected" };
    },
  };
}

export type DropController = ReturnType<typeof createDropController>;
