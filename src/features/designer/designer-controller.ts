// Nesne diyaloğu (designer) sekmelerinin durumu (tembel parça): sekme başına model sinyali, önizleme zamanlayıcısı ve kuşak
// sayacı. Taslak düzenleme → debounce'lu önizleme → kapılı uygulama → sonuç işleme ve yaşam döngüsü (kapatma, bağlantı
// kaybı, sunucu kaydı değişimi) buradadır. İstemci SQL üretmez; yalnızca `PlanRequest` kurar ve `PlanView`'i yorumlar.
//
// Kurallar:
// - `original` (alter: `ddl_load` çıktısı) hiçbir yerde değiştirilmez; isteklere derin kopyası (`old`/`def`) gider.
// - Uygulama tek uçuştadır; kapı çağrı anında yeniden hesaplanır (sunucu kaydı dahil), arayüz durumuna güvenilmez;
//   kapı kapalıysa hiçbir şey gönderilmez (istek de toast da yok).
// - Uygulama sırasında düzenleme/önizleme yok sayılır (taslak gönderilenle aynı kalır).
// - Kapanan sekmenin / eski kuşağın yanıtı durum yazmaz, bildirim ve ağaç yenilemesi yapmaz.
// - `plan_changed` sonrası otomatik yeniden yükleme yapılmaz (taslak ezilmez); kullanıcı `reload(force)` ile ister.
//   Temel tanımın bayat olabileceği her durumda (`plan_changed`, kısmen kalıcı `Failed`, uygulama sonrası başarısız
//   yeniden yükleme) `needsReload = true` olur; arayüz yeniden yükleme ipucunu buna göre gösterir.
// - Sonuçsuz create başarısı (`created = null`): sekme "tamam" durumundadır; düzenleme/önizleme/uygulama kapalı.
// - Tepkisellik: `model(tabId)` yalnızca `openCreate`/`openAlter` girdiyi kaydettikten sonra tepkiseldir. Girdi bu
//   çağrıların içinde EŞZAMANLI kurulur (`makeEntry` eşzamanlıdır, ilk `await`'ten önce); arayüz (Task 3) sekmeyi
//   çizmeden önce bu çağrıları eşzamanlı başlatmalıdır (sözü beklemek gerekmez).
// - Varyant (Plan 22): `object` nesne diyaloğu; `acl` (`openAcl`) ve `default_acl` (`openDefaultAcl`) yetki sekmeleridir.
//   Yetki sekmeleri her zaman `alter` kipindedir; yükleme/yeniden yükleme varyanta göre `ddl_load_acl` /
//   `ddl_load_default_acl` ile yapılır. İstekler `privileges` / `default_privileges`; uygulama sonrası yalnız yeniden
//   yüklenir (başlık değişimi, `onApplied`, ağaç yenilemesi yok).
// - Parola (Plan 15): değer yalnızca tepkisiz `secrets` haritasında (sekme başına) ve tek `ddl_apply` çağrısının
//   `options.password` alanında bulunur; modele, `PlanRequest`'e, kirli karşılaştırmasına, bildirime, günlüğe girmez.
//   Başarılı uygulamada, yeniden yüklemede, kapanışta, sekme düşünce (bağlantı kaybı/oturumlar kapandı/nesne yok) ve
//   taslağın parola niyeti `set` dışına çıkınca silinir. `secretReady` `registry` sinyaline bağlıdır.
import { createSignal, type Accessor, type Setter } from "solid-js";
import { tdes } from "../../app/strings-designer";
import type { ToastKind } from "../../app/toast-model";
import type { AclDef } from "../../ipc/bindings/AclDef";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { DdlCreate } from "../../ipc/bindings/DdlCreate";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { DefaultAclScope } from "../../ipc/bindings/DefaultAclScope";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { LookupRequest } from "../../ipc/bindings/LookupRequest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ObjectKind } from "../../ipc/bindings/ObjectKind";
import type { ObjectRef } from "../../ipc/bindings/ObjectRef";
import type { PlanRequest } from "../../ipc/bindings/PlanRequest";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Api } from "../../ipc/commands";
import { errorText, toPayload } from "../../ipc/errors";
import {
  applyGate, applyOptionsFor, canSubmit, cloneDef, defsEqual, isAclDef, isDefaultAclDef, isObjectDef, isServerScoped, localBlock, planHasRedacted,
  type ApplyGate, type ConfirmState, type DesignerDef, type DesignerVariant,
} from "./designer-model";
import { createDropController } from "./drop-controller";
import { dropRequest, matchesObject } from "./drop-model";
import { classifyError, designerErrorText, failedText } from "./errors";
import { PREVIEW_DELAY_MS, createPreviewer, type PreviewDeps, type Previewer } from "./preview";

export { aclTabKey, defaultAclTabKey } from "./designer-model";

export type DesignerMode = "create" | "alter";
/**
 * `error`: ilk yükleme genel hatayla bitti (`error` metni; yeniden dene = `reload`). `gone`: nesne/sunucu yok.
 * Yalnızca `ready` düzenlenebilir.
 */
export type DesignerStatus = "loading" | "ready" | "error" | "disconnected" | "gone";

export interface DesignerModel {
  /** Sekme kimliği. */
  id: string;
  serverId: string;
  serverName: string;
  /** Sunucu kapsamlı türlerde (Database/Role) `null` (bakım veritabanı). */
  databaseOid: number | null;
  mode: DesignerMode;
  /** Plan 22: sekme türü (nesne / nesne yetkileri / varsayılan yetkiler). */
  variant: DesignerVariant;
  /** object/acl: nesne türü. default_acl: kapsamın türü (`schema` ya da veritabanı geneli için `database`). */
  kind: ObjectKind;
  /**
   * alter: yüklenen nesne; create: başarıdan sonra `created` ile dolar. acl: yetkisi düzenlenen nesne. default_acl:
   * kapsam nesnesi (şema ya da veritabanı; silinince sekme `gone` olsun diye).
   */
  object: ObjectRef | null;
  /** Yalnız default_acl: yükleme/istek kapsamı (`role = null` → bağlı rol); diğerlerinde null. */
  aclScope: DefaultAclScope | null;
  /**
   * Yalnız default_acl: yeniden yüklemede artık var olmayan (`role_not_found`) seçili rol; kapsam bağlı role
   * (`role = null`) düşürüldü (form bilgi notu gösterir). Sonraki rol değişiminde temizlenir.
   */
  aclRoleGone: string | null;
  /**
   * alter: `ddl_load` (acl: `ddl_load_acl`, default_acl: `ddl_load_default_acl`) çıktısı (DEĞİŞTİRİLMEZ, `old` olarak
   * gider); create: null.
   */
  original: DesignerDef | null;
  /** Formun düzenlediği tanım (her düzenlemede yeni nesne; türü `variant`'a göredir). */
  draft: DesignerDef | null;
  status: DesignerStatus;
  /** Yükleme/bağlantı hatası metni. */
  error: string | null;
  dirty: boolean;
  /** Son önizleme ve hangi taslak sürümüne ait olduğu (-1: yok). */
  view: PlanView | null;
  viewFor: number;
  /** Taslak sürümü (her düzenlemede/yüklemede +1). */
  version: number;
  /** Önizleme zamanlayıcıda/uçuşta. */
  pending: boolean;
  previewError: string | null;
  applying: boolean;
  applyId: string | null;
  /** Son `Applied`/`Failed` (Failed → vurgulama/uyarı). */
  outcome: ApplyOutcome | null;
  /** `Failed` metni (`55P03` → kilit zaman aşımı). */
  applyError: string | null;
  /** `ddl_apply` reddi (Err) metni (`code`'a göre) ya da kısmen kalıcı `Failed` uyarısı. */
  gateError: string | null;
  /** Temel tanım bayat olabilir (plan_changed / kısmi uygulama / uygulama sonrası yükleme hatası): yeniden yükleyin. */
  needsReload: boolean;
  lockTimeoutMs: number;
  confirm: ConfirmState;
  /** Sunucu kaydından (kayıt değişince `serverChanged` ile güncellenir). */
  locked: boolean;
  production: boolean;
  /** Yalnız index create. */
  concurrently: boolean;
  /** Silme isteği için (`planRequest(m, "drop")`); acl: REVOKE ... CASCADE (`privileges` isteği). */
  cascade: boolean;
  /**
   * Son başarılı uygulamanın `applied.warnings` iletileri (arka uç metni; SQL/değer/parola içermez). Yeniden yüklemede
   * korunur; bir sonraki uygulamada ve kapanışta temizlenir.
   */
  postApplyWarnings: string[];
  /**
   * Kısmi create (`failed.created`): oluşturulan nesne. Doluyken create yeniden gönderilmez (kapı `partial`);
   * `openCreatedAsAlter` sekmeyi bu nesnenin düzenlemesine çevirir. Yalnızca create kipinde dolar.
   */
  partialCreated: ObjectRef | null;
}

/**
 * Nesne formlarının gördüğü model (`variant = "object"`): tanımlar `ObjectDef`. DesignerView formu varyanta göre seçer;
 * nesne formuna yalnız nesne sekmesinin modeli verilir.
 */
export type ObjectDesignerModel = Omit<DesignerModel, "original" | "draft"> & { original: ObjectDef | null; draft: ObjectDef | null };

/** `username`: sunucu kaydındaki bağlantı kullanıcısı (Role formunun kendini kilitleme uyarıları; isteğe bağlı). */
export interface ServerInfo { name: string; locked: boolean; production: boolean; username?: string }
export interface FocusRequest { field: string; seq: number }

export const DEFAULT_LOCK_TIMEOUT_MS = 5000;
/** Kısa ömürlü lookup türlerinin önbellek süresi. */
export const LOOKUP_TTL_MS = 30_000;
/** Oturum süresince tutulan lookup türleri (diğerleri `LOOKUP_TTL_MS`). */
const SESSION_LOOKUPS = new Set<LookupRequest["kind"]>([
  "roles", "schemas", "tablespaces", "available_extensions", "index_methods", "collations",
  // Plan 15 (sunucu kapsamlı): anahtar isteğin tamamını içerir — `locales{provider}` sağlayıcı başına ayrı girdidir.
  "grantable_roles", "encodings", "template_databases", "locales", "settings",
]);

const emptyConfirm = (): ConfirmState => ({ checked: false, typedName: "" });

/**
 * Taslaktan istek: create → `{create, def, concurrently}`; alter → `{alter, object, old: original, new: draft}`;
 * `"drop"` → `{drop, object, def: original, cascade}` (`dropRequest`; silme diyaloğuyla tek tanım). Tanımlar derin
 * kopyalanır (istek nesnesi modelle paylaşılmaz). Plan 22: acl → `{privileges, object, old, new, cascade}`;
 * default_acl → `{default_privileges, scope, old, new}` (ikisinde de silme yok). `variant` verilmezse `object`; tanım
 * türü varyantla uyuşmazsa null.
 */
export function planRequest(
  m: Pick<DesignerModel, "mode" | "kind" | "object" | "original" | "draft" | "concurrently" | "cascade">
    & Partial<Pick<DesignerModel, "variant" | "aclScope">>,
  op: "save" | "drop" = "save",
): PlanRequest | null {
  const { original: o, draft: d } = m;
  const variant = m.variant ?? "object";
  if (variant === "acl") {
    return op === "save" && m.object && o && d && isAclDef(o) && isAclDef(d)
      ? { op: "privileges", object: { ...m.object }, old: cloneDef(o), new: cloneDef(d), cascade: m.cascade }
      : null;
  }
  if (variant === "default_acl") {
    return op === "save" && m.aclScope && o && d && isDefaultAclDef(o) && isDefaultAclDef(d)
      ? { op: "default_privileges", scope: { ...m.aclScope }, old: cloneDef(o), new: cloneDef(d) }
      : null;
  }
  if (op === "drop") {
    return m.mode === "alter" && m.object && o && isObjectDef(o)
      ? dropRequest(m.object, o, m.cascade)
      : null;
  }
  if (!d || !isObjectDef(d)) return null;
  if (m.mode === "create") return { op: "create", def: cloneDef(d), concurrently: m.kind === "index" && m.concurrently };
  return m.object && o && isObjectDef(o) ? { op: "alter", object: { ...m.object }, old: cloneDef(o), new: cloneDef(d) } : null;
}

/** Düz `LookupRequest` için anahtar sırasından bağımsız JSON. */
const lookupKey = (serverId: string, databaseOid: number | null, r: LookupRequest) =>
  `${serverId}|${databaseOid}|${JSON.stringify(r, Object.keys(r).sort())}`;

interface PreviewOk { view: PlanView; version: number; gen: number }
class PreviewFailure {
  constructor(readonly error: unknown, readonly version: number, readonly gen: number) {}
}

interface Entry {
  model: Accessor<DesignerModel>;
  set: Setter<DesignerModel>;
  /** Kuşak: yükleme/kayıp/kapatmada artar; eski kuşağın yanıtı yoksayılır. */
  gen: number;
  previewer: Previewer<PreviewOk>;
  refreshNodeId: string | null;
  create: DdlCreate | null;
  /** create: varsayılan tanım (kirli karşılaştırması). */
  base: DesignerDef | null;
  focus: Accessor<FocusRequest | null>;
  setFocus: Setter<FocusRequest | null>;
}

interface LookupSlot { promise: Promise<LookupItem[]>; at: number | null; session: boolean }

/** Başarılı uygulama sonrası (create'te yalnızca `created` doluysa): açıcı başlığı ve ek ağaç yenilemelerini yapar. */
export interface AppliedInfo {
  tabId: string;
  kind: ObjectKind;
  /** Uygulama anındaki kip. */
  mode: DesignerMode;
  /** alter: yüklenen tanım; create: null. */
  before: ObjectDef | null;
  /** Uygulanan taslak. */
  after: ObjectDef;
  /** Uygulama sonrası nesne kimliği (create/rename: `created`; aksi halde eski `object`). */
  object: ObjectRef | null;
}

export interface DesignerControllerDeps {
  api: Pick<Api, "ddlDefaults" | "ddlLoad" | "ddlLoadAcl" | "ddlLoadDefaultAcl" | "ddlLookup" | "ddlPreview" | "ddlApply" | "ddlCancel">;
  notify(text: string, kind: ToastKind): void;
  serverInfo(serverId: string): ServerInfo | null;
  /** Explorer düğüm yenilemesi (hatası yutulur). */
  refreshNode(nodeId: string): Promise<void> | void;
  /** Başarılı uygulama sonrası (hatası yutulur; uygulama sonucunu etkilemez). */
  onApplied?(info: AppliedInfo): void;
  /** Başarılı her uygulama sonrası Inspector'ı (özellikler/DDL) yeniden okut (hatası yutulur). */
  refreshInspector?(): void;
  previewDelay?: number;
  previewDeps?: Partial<PreviewDeps>;
  newId?: () => string;
  /** Lookup TTL saati (test için). */
  now?: () => number;
}

export interface OpenCreateInput { tabId: string; serverId: string; databaseOid: number | null; create: DdlCreate; refreshNodeId: string | null }
export interface OpenAlterInput { tabId: string; serverId: string; databaseOid: number | null; object: ObjectRef; refreshNodeId: string | null }
/** Plan 22: yetki sekmesi açılış bağlamı (`openAcl` / `openDefaultAcl`). */
export interface AclOpenContext { serverId: string; databaseOid: number | null; refreshNodeId: string | null }

export function createDesignerController(deps: DesignerControllerDeps) {
  const { api, notify } = deps;
  const newId = deps.newId ?? (() => crypto.randomUUID());
  const now = deps.now ?? (() => Date.now());
  const pd: PreviewDeps = {
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    ...deps.previewDeps,
  };
  const delay = deps.previewDelay ?? PREVIEW_DELAY_MS;
  const entries = new Map<string, Entry>();
  // Kayıt sinyali: `entries` düz Map'tir (tepkisiz). Sekme denetleyici girdisinden ÖNCE çizilebilir (opener
  // `openDesigner` → `openCreate/openAlter`; store eşzamanlı akar); okuyucular bu sinyale bağlanır, girdi
  // eklenince/silinince yeniden çalışır.
  const [registry, bump] = createSignal<void>(undefined, { equals: false });
  const lookups = new Map<string, LookupSlot>();
  /** Sekme → parola (tepkisiz, kalıcı değil; yalnızca `ddl_apply` seçeneğine girer). Okuyucular `registry`'ye bağlanır. */
  const secrets = new Map<string, string>();
  let focusSeq = 0;

  const model = (tabId: string): DesignerModel | undefined => (registry(), entries.get(tabId)?.model());
  const update = (e: Entry, f: (m: DesignerModel) => DesignerModel) => e.set((m) => f(m));
  const patch = (e: Entry, p: Partial<DesignerModel>) => update(e, (m) => ({ ...m, ...p }));
  /** Yanıt hâlâ geçerli mi: sekme açık ve aynı kuşak. */
  const alive = (tabId: string, e: Entry, gen: number) => entries.get(tabId) === e && e.gen === gen;

  /** En iyi çaba iptal: hata (zaten bitmiş/bilinmeyen kimlik) yok sayılır. */
  const cancelQuietly = (applyId: string) => {
    try {
      void api.ddlCancel(applyId).catch(() => {});
    } catch {
      /* eşzamanlı hata da yok sayılır */
    }
  };
  const refresh = (nodeId: string | null) => {
    if (nodeId === null) return;
    try {
      void Promise.resolve(deps.refreshNode(nodeId)).catch(() => {});
    } catch {
      /* ağaç yenileme hatası uygulama sonucunu etkilemez */
    }
  };
  const dropLookups = (serverId: string, databaseOid?: number | null) => {
    const prefix = databaseOid === undefined ? `${serverId}|` : `${serverId}|${databaseOid}|`;
    for (const k of [...lookups.keys()]) if (k.startsWith(prefix)) lookups.delete(k);
  };

  /** Parola girdisini siler; varsa okuyucular (`secretReady`) yeniden çalışır. */
  const forgetSecret = (tabId: string) => {
    if (secrets.delete(tabId)) bump();
  };
  /** Bu sekmede geçerli (boş olmayan) parola var mı. Tepkisel (`registry`). */
  const secretReady = (tabId: string): boolean => (registry(), secrets.has(tabId));

  /** Sekme salt okunur (`disconnected`/`gone`): kuşak artar, önizleme iptal, uçuştaki apply için `ddl_cancel`, parola silinir. */
  const markDown = (e: Entry, status: "disconnected" | "gone", error: string | null) => {
    const m = e.model();
    forgetSecret(m.id);
    e.gen++;
    e.previewer.cancel();
    if (m.applying && m.applyId) cancelQuietly(m.applyId);
    patch(e, { status, error, pending: false, applying: false, applyId: null });
  };
  /** Sonuçsuz create başarısı: sekme "tamam" (arayüz Kapat sunar); düzenleme/önizleme/uygulama yok. */
  const finished = (m: DesignerModel) => m.mode === "create" && m.outcome?.outcome === "applied";
  /** Düzenleme/önizleme yapılabilir mi: `ready`, uygulama dışında ve "tamam" değil. */
  const editable = (m: DesignerModel) => m.status === "ready" && !m.applying && !finished(m);
  /** Kirli: taslak temelden (alter: original; create: varsayılan) farklı ya da create'te CONCURRENTLY seçili. */
  const dirtyOf = (e: Entry, draft: DesignerDef, m: Pick<DesignerModel, "original" | "mode" | "concurrently">) => {
    const ref = m.original ?? e.base;
    if (!ref) return true;
    return !defsEqual(draft, ref) || (m.mode === "create" && m.concurrently);
  };
  const goneText = (p: ErrorPayload) => (p.code === "unknown_server" ? tdes.status.serverGone : tdes.status.gone);

  /** Bağlantı/yokluk hatası ise sekmeyi düşürür ve true döner. */
  const markIfDown = (e: Entry, p: ErrorPayload): boolean => {
    const c = classifyError(p);
    if (c === "disconnected") markDown(e, "disconnected", p.message);
    else if (c === "gone") markDown(e, "gone", goneText(p));
    else return false;
    return true;
  };

  // ---- kapı
  /**
   * `fresh`: uygulama anı — sunucu kaydı yeniden okunur (kayıt yoksa bağlı sayılmaz), model ile kayıttan katı olanı alınır.
   * Aksi halde (arayüz) modeldeki değerler.
   */
  const gateFor = (m: DesignerModel, fresh: boolean): ApplyGate => {
    const info = fresh ? deps.serverInfo(m.serverId) : null;
    return applyGate({
      locked: m.locked || (info?.locked ?? false),
      production: m.production || (info?.production ?? false),
      serverName: fresh ? (info?.name ?? "") : m.serverName,
      view: m.view,
      viewIsCurrent: m.view !== null && m.viewFor === m.version,
      pending: m.pending,
      applying: m.applying,
      connected: m.status !== "disconnected" && m.status !== "gone" && (!fresh || info !== null),
      loading: m.status === "loading" || m.status === "error",
      secretMissing: planHasRedacted(m.view) && !secretReady(m.id),
      partial: m.mode === "create" && m.partialCreated !== null,
    });
  };

  // ---- önizleme
  const onPreview = (tabId: string) => (r: PreviewOk) => {
    const e = entries.get(tabId);
    if (!e || !alive(tabId, e, r.gen)) return;
    update(e, (m) => {
      const current = r.version === m.version;
      // Plan değiştiyse (farklı hash) önceki onay geçersiz.
      const sameplan = m.view !== null && m.view.hash === r.view.hash;
      return {
        ...m,
        view: r.view,
        viewFor: r.version,
        pending: current ? false : m.pending,
        previewError: current ? null : m.previewError,
        confirm: sameplan ? m.confirm : emptyConfirm(),
      };
    });
  };
  const onPreviewError = (tabId: string) => (x: unknown) => {
    const e = entries.get(tabId);
    if (!e || !(x instanceof PreviewFailure) || !alive(tabId, e, x.gen)) return;
    const p = toPayload(x.error);
    if (markIfDown(e, p)) return;
    update(e, (m) => (x.version === m.version ? { ...m, pending: false, previewError: designerErrorText(p) } : m));
  };

  /** Mevcut taslağın önizlemesini ister (`now`: beklemeden). Yalnızca `ready` ve uygulama dışında. */
  const startPreview = (e: Entry, immediately: boolean) => {
    const m = e.model();
    if (!editable(m)) return;
    const req = planRequest(m);
    if (!req) return;
    // Yerel engel (ör. biçimsiz sequence sayısı): istek gönderilmez; bekleyen önizleme iptal, görünüm bayat kalır
    // (sürüm ilerledi → kapı "stale"). Sayılar düzelince sonraki düzenleme normal önizleme ister ve bu metin hemen kalkar.
    if (localBlock(m.draft)) {
      e.previewer.cancel();
      patch(e, { pending: false, previewError: tdes.sequence.fixNumbers });
      return;
    }
    const { version } = m;
    const gen = e.gen;
    patch(e, { pending: true, previewError: m.previewError === tdes.sequence.fixNumbers ? null : m.previewError });
    const run = () =>
      api.ddlPreview(m.serverId, m.databaseOid, req).then(
        (view): PreviewOk => ({ view, version, gen }),
        (err: unknown) => { throw new PreviewFailure(err, version, gen); },
      );
    if (immediately) e.previewer.runNow(run);
    else e.previewer.schedule(run);
  };

  // ---- açma / yükleme
  const makeEntry = (
    tabId: string, serverId: string, databaseOid: number | null, mode: DesignerMode, kind: ObjectKind, object: ObjectRef | null,
    refreshNodeId: string | null, create: DdlCreate | null, variant: DesignerVariant = "object", aclScope: DefaultAclScope | null = null,
  ): Entry => {
    const info = deps.serverInfo(serverId);
    const [m, set] = createSignal<DesignerModel>({
      id: tabId, serverId, serverName: info?.name ?? "", databaseOid, mode, variant, kind, object, aclScope, aclRoleGone: null,
      original: null, draft: null, status: "loading", error: null, dirty: false,
      view: null, viewFor: -1, version: 0, pending: false, previewError: null,
      applying: false, applyId: null, outcome: null, applyError: null, gateError: null, needsReload: false,
      lockTimeoutMs: DEFAULT_LOCK_TIMEOUT_MS, confirm: emptyConfirm(),
      locked: info?.locked ?? false, production: info?.production ?? false,
      concurrently: false, cascade: false, postApplyWarnings: [], partialCreated: null,
    });
    const [focus, setFocus] = createSignal<FocusRequest | null>(null);
    const previewer = createPreviewer<PreviewOk>(pd, onPreview(tabId), onPreviewError(tabId), delay);
    const e: Entry = { model: m, set, gen: 0, previewer, refreshNodeId, create, base: null, focus, setFocus };
    entries.set(tabId, e);
    bump();
    return e;
  };

  /** Yükleme hatası: gone/disconnected ya da genel hata (önceki taslak varsa `ready` korunur). */
  const loadFailed = (e: Entry, err: unknown, prev: DesignerStatus, prevScope?: DefaultAclScope) => {
    // Rol değişimi (default_acl) başarısız: kapsam eski role döner; tanım/taslak/görünüm o role aittir, tutarlı kalır.
    if (prevScope) patch(e, { aclScope: prevScope });
    const p = toPayload(err);
    // default_acl: `role_not_found` kapsamın rolünün yokluğudur (şema/veritabanı yokluğu düz `not_found` → gone).
    const roleMissing = e.model().variant === "default_acl" && p.code === "role_not_found";
    // Rol değişiminde seçilen rol yok: sekme düşmez, hata gösterilir (önceki rolün tanımı kalır).
    if (roleMissing && prevScope) {
      patch(e, { status: prev === "ready" ? "ready" : "error", error: errorText(p) });
      return;
    }
    // Yeniden yüklemede (açılış/yenileme/uygulama sonrası) seçili rol kaybolmuş: bağlı role düşülür, bilgi notu.
    const scope = e.model().aclScope;
    if (roleMissing && scope && scope.role !== null) {
      patch(e, { aclScope: { ...scope, role: null }, aclRoleGone: scope.role });
      void loadObject(e.model().id, e);
      return;
    }
    if (markIfDown(e, p)) return;
    patch(e, { status: prev === "ready" ? "ready" : "error", error: errorText(p) });
  };

  /** Yeni temel tanımı kurar: taslak = kopya, sürüm ilerler, eski önizleme/onay/hatalar düşer. */
  const setBase = (e: Entry, original: DesignerDef | null, draft: DesignerDef) =>
    update(e, (m) => ({
      ...m, status: "ready", error: null, original, draft, dirty: false, version: m.version + 1,
      // acl: CASCADE her yeni temelde (yeniden yükleme/uygulama sonrası) güvenli varsayılana döner.
      cascade: m.variant === "acl" ? false : m.cascade,
      view: null, viewFor: -1, pending: false, previewError: null, gateError: null, applyError: null, needsReload: false,
      confirm: emptyConfirm(),
    }));

  const loadDefaults = async (tabId: string, e: Entry) => {
    const gen = ++e.gen;
    e.previewer.cancel();
    const m = e.model();
    const c = e.create!;
    const prev = m.draft ? "ready" : m.status;
    patch(e, { status: "loading", error: null, pending: false });
    try {
      const def = await api.ddlDefaults(m.serverId, m.databaseOid, c.kind, c.context, c.preset ?? null);
      if (!alive(tabId, e, gen)) return;
      e.base = cloneDef(def);
      setBase(e, null, cloneDef(def));
    } catch (err) {
      if (alive(tabId, e, gen)) loadFailed(e, err, prev);
    }
  };

  /** Varyanta göre yükleme komutu: object → `ddl_load`, acl → `ddl_load_acl`, default_acl → `ddl_load_default_acl`. */
  const loadDef = (m: DesignerModel): Promise<DesignerDef> => {
    if (m.variant === "default_acl") return api.ddlLoadDefaultAcl(m.serverId, m.databaseOid!, { ...m.aclScope! });
    if (m.variant === "acl") return api.ddlLoadAcl(m.serverId, m.databaseOid, m.object!);
    return api.ddlLoad(m.serverId, m.databaseOid, m.object!);
  };

  /** `prevScope`: rol değişimi yüklemesi; başarısızlıkta kapsam buna geri döner. */
  const loadObject = async (tabId: string, e: Entry, prevScope?: DefaultAclScope) => {
    const gen = ++e.gen;
    e.previewer.cancel();
    const m = e.model();
    const prev = m.draft ? "ready" : m.status;
    patch(e, { status: "loading", error: null, pending: false });
    try {
      const def = await loadDef(m);
      if (!alive(tabId, e, gen)) return;
      setBase(e, cloneDef(def), cloneDef(def));
    } catch (err) {
      if (alive(tabId, e, gen)) loadFailed(e, err, prev, prevScope);
    }
  };

  const openCreate = async (input: OpenCreateInput): Promise<void> => {
    if (entries.has(input.tabId)) return;
    const e = makeEntry(input.tabId, input.serverId, input.databaseOid, "create", input.create.kind, null, input.refreshNodeId, input.create);
    // İlk önizleme zamanlanmaz: boş form doğrulama hatası verir; ilk düzenleme ya da `previewNow` ister.
    await loadDefaults(input.tabId, e);
  };

  const openAlter = async (input: OpenAlterInput): Promise<void> => {
    if (entries.has(input.tabId)) return;
    const e = makeEntry(input.tabId, input.serverId, input.databaseOid, "alter", input.object.kind, { ...input.object }, input.refreshNodeId, null);
    await loadObject(input.tabId, e);
  };

  /**
   * Plan 22: nesne yetkileri sekmesi (`acl`; kip `alter`). Sunucu kapsamlı nesnede (Database) `databaseOid` null'dır.
   * Girdi EŞZAMANLI kurulur (`openAlter` gibi).
   */
  const openAcl = async (tabId: string, ctx: AclOpenContext, object: ObjectRef): Promise<void> => {
    if (entries.has(tabId)) return;
    const e = makeEntry(tabId, ctx.serverId, ctx.databaseOid, "alter", object.kind, { ...object }, ctx.refreshNodeId, null, "acl");
    await loadObject(tabId, e);
  };

  /**
   * Plan 22: varsayılan yetkiler sekmesi (`default_acl`; kip `alter`). `databaseOid` zorunludur (null → açılmaz).
   * Kapsam nesnesi şema (`schemaOid`) ya da veritabanıdır; silinince sekme `gone` olur.
   */
  const openDefaultAcl = async (tabId: string, ctx: AclOpenContext, scope: DefaultAclScope): Promise<void> => {
    if (entries.has(tabId) || ctx.databaseOid === null) return;
    const object: ObjectRef = scope.schemaOid === null
      ? { kind: "database", oid: ctx.databaseOid }
      : { kind: "schema", oid: scope.schemaOid };
    const e = makeEntry(tabId, ctx.serverId, ctx.databaseOid, "alter", object.kind, object, ctx.refreshNodeId, null, "default_acl", { ...scope });
    await loadObject(tabId, e);
  };

  /**
   * Plan 22 (default_acl): kapsamın rolünü değiştirir ve tanımı yeniden yükler. Kirli taslak atılır: çağıran
   * kullanıcıdan onay almış olmalıdır. Başka varyantta, uygulama sırasında ya da `gone` sekmede etkisizdir.
   */
  const setAclRole = (tabId: string, role: string): void => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.variant !== "default_acl" || !m.aclScope || m.applying || m.status === "gone") return;
    forgetSecret(tabId);
    const prevScope = { ...m.aclScope };
    patch(e, { aclScope: { ...m.aclScope, role }, aclRoleGone: null, postApplyWarnings: [] });
    void loadObject(tabId, e, prevScope);
  };

  /**
   * alter: `ddl_load`; create: varsayılanlar. Kirli taslakta yalnızca `force` ile (çağıran kullanıcıdan onay almış olmalı).
   * `gone` ve uygulama sırasında yapılmaz; `disconnected` sekme bununla yeniden hazır olur.
   */
  const reload = async (tabId: string, opts?: { force?: boolean }): Promise<void> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.applying || m.status === "gone") return;
    if (m.dirty && !opts?.force) return;
    forgetSecret(tabId);
    if (m.mode === "alter") await loadObject(tabId, e);
    else await loadDefaults(tabId, e);
  };

  // ---- düzenleme
  /**
   * `f` taslağın kopyasını alır (yerinde değiştirebilir ya da yeni tanım döndürebilir). Yalnızca `ready` ve uygulama
   * dışında; uygulama sırasındaki düzenlemeler bilinçli olarak yok sayılır (gönderilen taslak değişmez).
   */
  const editDef = <D extends DesignerDef>(tabId: string, variant: DesignerVariant, f: (draft: D) => D | void) => {
    const e = entries.get(tabId);
    const m = e?.model();
    // Yalnız varyantı eşleşen sekme (ör. nesne düzenleyicisi yetki sekmesinde etkisiz).
    if (!e || !m || m.variant !== variant || !editable(m) || !m.draft) return;
    const copy = cloneDef(m.draft as D);
    const next = cloneDef(f(copy) ?? copy);
    update(e, (cur) => ({
      ...cur, draft: next, version: cur.version + 1, dirty: dirtyOf(e, next, cur), outcome: null, applyError: null,
    }));
    // Parola niyeti `set` değilse (keep/clear ya da rol dışı) girilmiş parola tutulmaz.
    if (!(variant === "object" && isObjectDef(next) && next.kind === "role" && next.password === "set")) forgetSecret(tabId);
    startPreview(e, false);
  };
  const edit = (tabId: string, f: (draft: ObjectDef) => ObjectDef | void) => editDef<ObjectDef>(tabId, "object", f);
  /** Plan 22: nesne yetkileri taslağı (yalnız `acl` sekmesinde). */
  const editAcl = (tabId: string, f: (draft: AclDef) => AclDef | void) => editDef<AclDef>(tabId, "acl", f);
  /** Plan 22: varsayılan yetkiler taslağı (yalnız `default_acl` sekmesinde). */
  const editDefaultAcl = (tabId: string, f: (draft: DefaultAclDef) => DefaultAclDef | void) =>
    editDef<DefaultAclDef>(tabId, "default_acl", f);

  const previewNow = (tabId: string) => {
    const e = entries.get(tabId);
    if (e) startPreview(e, true);
  };

  const setLockTimeout = (tabId: string, ms: number) => {
    const e = entries.get(tabId);
    if (!e) return;
    patch(e, { lockTimeoutMs: Number.isFinite(ms) && ms >= 0 ? Math.floor(ms) : DEFAULT_LOCK_TIMEOUT_MS });
  };
  const setConfirm = (tabId: string, c: Partial<ConfirmState>) => {
    const e = entries.get(tabId);
    if (!e) return;
    update(e, (m) => ({ ...m, confirm: { ...m.confirm, ...c } }));
  };
  /** Yalnız index create: plan değişir → sürüm ilerler, önizleme yenilenir. */
  const setConcurrently = (tabId: string, v: boolean) => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.mode !== "create" || m.kind !== "index" || !editable(m) || m.concurrently === v) return;
    patch(e, { concurrently: v, version: m.version + 1, dirty: m.draft ? dirtyOf(e, m.draft, { ...m, concurrently: v }) : m.dirty });
    startPreview(e, false);
  };
  /**
   * Nesne sekmesi: yalnız silme isteği seçeneği. acl: `privileges` isteğinin parçasıdır → plan değişir, sürüm ilerler ve
   * önizleme yenilenir (risk/kapı/üretim adı onayı yeni görünüme göre; eski görünüm bayat sayılır).
   */
  const setCascade = (tabId: string, v: boolean) => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m) return;
    if (m.variant !== "acl") {
      patch(e, { cascade: v });
      return;
    }
    if (!editable(m) || m.cascade === v) return;
    patch(e, { cascade: v, version: m.version + 1 });
    startPreview(e, false);
  };

  // ---- uygulama
  const applied = async (tabId: string, e: Entry, mode: DesignerMode, out: Extract<ApplyOutcome, { outcome: "applied" }>) => {
    const m = e.model();
    forgetSecret(tabId);
    const warnings = out.warnings ?? [];
    if (warnings.length > 0) notify(tdes.apply.appliedWarnings(out.statements, warnings.length), "info");
    else notify(tdes.apply.applied(out.statements), "success");
    // Uyarılar sekmede kalıcı (yeniden yükleme `setBase` bunlara dokunmaz).
    patch(e, { postApplyWarnings: [...warnings] });
    try {
      deps.refreshInspector?.();
    } catch {
      /* Inspector yenilemesi uygulama sonucunu etkilemez */
    }
    if (m.variant !== "object") {
      // Yetki sekmeleri: ad/ağaç değişmez; yalnız yeniden yüklenir (yükleme başarısızsa `needsReload` kalır).
      patch(e, { dirty: false, needsReload: true });
      await loadObject(tabId, e);
      return;
    }
    // Sunucu kapsamlı nesne (rol/veritabanı) her veritabanının lookup'larını (ör. `roles`) etkiler.
    dropLookups(m.serverId, isServerScoped(m.kind) ? undefined : m.databaseOid);
    refresh(e.refreshNodeId);
    // Buraya yalnız `variant = "object"` ulaşır (yetki sekmeleri yukarıda döner); `isObjectDef` tip daraltmasıdır.
    if ((mode === "alter" || out.created) && m.draft && isObjectDef(m.draft)) {
      const before = m.original && isObjectDef(m.original) ? cloneDef(m.original) : null;
      try {
        deps.onApplied?.({ tabId, kind: m.kind, mode, before, after: cloneDef(m.draft), object: out.created ?? m.object });
      } catch {
        /* başlık/ek yenileme hatası uygulama sonucunu etkilemez */
      }
    }
    if (mode === "create") {
      if (!out.created) {
        // Yeni nesne bulunamadı: sekme "tamam" durumunda (arayüz kapat düğmesi sunar).
        patch(e, { dirty: false });
        return;
      }
      // Yeniden yükleme başarısız olursa temel bayat kalır: `needsReload` (başarıda `setBase` temizler).
      patch(e, { mode: "alter", object: { ...out.created }, kind: out.created.kind, dirty: false, needsReload: true });
    } else {
      // Kimliği değişen alter (indeks yapısal yeniden kurma: DROP+CREATE → yeni oid; yeniden adlandırma) `created`
      // döndürür: yeniden yükleme yeni nesneyle yapılır (eski oid `not_found` verirdi). Sekme anahtarı değişmez;
      // eşleme `findAlter` ile yapılır.
      patch(e, { object: out.created ? { ...out.created } : m.object, dirty: false, needsReload: true });
    }
    await loadObject(tabId, e);
  };

  const failed = (e: Entry, out: Extract<ApplyOutcome, { outcome: "failed" }>) => {
    patch(e, { applyError: failedText(out) });
    if (out.created) {
      // Kısmi uygulama: nesne oluştu/yeniden adlandırıldı (ör. CREATE DATABASE kalıcı, sonraki SET/COMMENT başarısız).
      refresh(e.refreshNodeId);
      const m = e.model();
      dropLookups(m.serverId, isServerScoped(m.kind) ? undefined : m.databaseOid);
      const created = { ...out.created };
      if (m.mode === "create") {
        // Aynı create yeniden gönderilmez (42P04/42710): kapı `partial`, arayüz "Düzenle olarak aç" sunar. Nesne
        // oluştuğu için parola bir daha gönderilmez: silinir.
        forgetSecret(m.id);
        notify(tdes.gate.partial, "error");
        patch(e, { partialCreated: created });
        return;
      }
      // Alter: kimlik değişmiş olabilir; yeniden yükleme yeni nesneyle yapılır (aşağıdaki kısmi yol).
      patch(e, { object: created });
    }
    if (out.committed > 0) {
      // Bir kısmı kalıcı: nesne değişti, `original` bayat. Taslak korunur; bayat temele karşı önizleme istenmez.
      notify(tdes.apply.committed(out.committed), "error");
      patch(e, { gateError: tdes.gate.partialChanged, needsReload: true });
      return;
    }
    // Aynı `expectedHash` ile yeniden gönderilmez: görünüm düştü, yenisi istenir.
    startPreview(e, true);
  };

  /** `ddl_apply` reddi (plan uygulanmadı): yalnızca `code`'a göre. */
  const rejected = (e: Entry, p: ErrorPayload) => {
    if (markIfDown(e, p)) return;
    switch (classifyError(p)) {
      case "planChanged":
        // Taslak korunur; eski `original`e karşı yeniden önizleme istenmez. Yeniden yükleme kullanıcıya bırakılır.
        patch(e, { gateError: tdes.gate.planChanged, needsReload: true });
        return;
      case "locked":
        patch(e, { gateError: tdes.gate.locked, locked: true });
        return;
      default:
        patch(e, { gateError: designerErrorText(p) });
        startPreview(e, true);
    }
  };

  /** Kapı → `ddl_apply` → sonuç. Kapı kapalıysa ya da uçuşta ise hiçbir şey yapmaz. */
  const apply = async (tabId: string): Promise<void> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.applying || !m.view || finished(m)) return;
    // Yerel engel: kapı zaten bayat görünüm nedeniyle kapalıdır; ikinci savunma hattı.
    if (localBlock(m.draft)) return;
    const gate = gateFor(m, true);
    if (!canSubmit(gate, m.confirm)) return;
    const req = planRequest(m);
    if (!req) return;
    const options = applyOptionsFor(newId(), m.lockTimeoutMs, m.view, gate, m.confirm, secrets.get(tabId) ?? null);
    if (!options) return;
    const gen = e.gen;
    const mode = m.mode;
    e.previewer.cancel();
    patch(e, { applying: true, applyId: options.applyId, gateError: null, applyError: null, outcome: null, postApplyWarnings: [] });
    let out: ApplyOutcome;
    try {
      out = await api.ddlApply(m.serverId, m.databaseOid, req, options);
    } catch (err) {
      if (!alive(tabId, e, gen)) return;
      patch(e, { applying: false, applyId: null, view: null, viewFor: -1, confirm: emptyConfirm() });
      rejected(e, toPayload(err));
      return;
    }
    if (!alive(tabId, e, gen)) return;
    // Yanıt geldi: görünüm (ve hash'i) tüketildi; onay sıfırlanır.
    patch(e, { applying: false, applyId: null, outcome: out, view: null, viewFor: -1, confirm: emptyConfirm() });
    if (out.outcome === "applied") await applied(tabId, e, mode, out);
    else failed(e, out);
  };

  /**
   * "Düzenle olarak aç" (kısmi create): aynı sekme, oluşturulan nesnenin alter sekmesine çevrilir (`openAlter` ile;
   * sekme kimliği ve `refreshNodeId` korunur). Yükleme başarılıysa başlık `onApplied` ile güncellenir. Kısmi durum
   * yoksa ya da uygulama sürüyorsa etkisizdir.
   */
  const openCreatedAsAlter = async (tabId: string): Promise<void> => {
    const e = entries.get(tabId);
    const m = e?.model();
    if (!e || !m || m.mode !== "create" || !m.partialCreated || m.applying) return;
    const object = { ...m.partialCreated };
    entries.delete(tabId);
    secrets.delete(tabId);
    e.gen++;
    e.previewer.dispose();
    await openAlter({ tabId, serverId: m.serverId, databaseOid: m.databaseOid, object, refreshNodeId: e.refreshNodeId });
    const n = entries.get(tabId)?.model();
    if (!n || n.status !== "ready" || !n.original || !isObjectDef(n.original)) return;
    try {
      deps.onApplied?.({ tabId, kind: n.kind, mode: "create", before: null, after: cloneDef(n.original), object: { ...object } });
    } catch {
      /* başlık/ek yenileme hatası sonucu etkilemez */
    }
  };

  /** Uçuştaki uygulamayı iptal ister; sonuç `apply`'ın yanıtıyla gelir (iptal hatası yok sayılır). */
  const cancelApply = async (tabId: string): Promise<void> => {
    const m = model(tabId);
    if (!m?.applying || !m.applyId) return;
    await api.ddlCancel(m.applyId).catch(() => {});
  };

  // ---- yaşam döngüsü
  /** Sunucu kaydı değişti: ad/kilit/üretim yenilenir; biri değiştiyse onay sıfırlanır. Kayıt yoksa dokunulmaz. */
  const serverChanged = (serverId: string) => {
    const info = deps.serverInfo(serverId);
    if (!info) return;
    drop.serverChanged(serverId);
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId) continue;
      if (m.serverName === info.name && m.locked === info.locked && m.production === info.production) continue;
      patch(e, { serverName: info.name, locked: info.locked, production: info.production, confirm: emptyConfirm() });
    }
  };

  /** Arka uç sunucunun oturumlarını kapattı (sil/kes/kilit): sekmeler `disconnected`/`gone`, lookup önbelleği düşer. */
  const serverSessionsClosed = (serverId: string, gone: boolean) => {
    dropLookups(serverId);
    drop.serverSessionsClosed(serverId);
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId || m.status === "gone") continue;
      markDown(e, gone ? "gone" : "disconnected", gone ? tdes.status.serverGone : null);
    }
  };

  /**
   * Veritabanı silindi: o veritabanına ait (veritabanı kapsamlı, `databaseOid === oid`) tüm sekmeler `gone` ("Nesne
   * artık yok"); sekmeler kapanmaz. Sunucu kapsamlı (`null`) sekmeler etkilenmez.
   */
  const databaseDropped = (serverId: string, databaseOid: number) => {
    dropLookups(serverId, databaseOid);
    for (const e of entries.values()) {
      const m = e.model();
      if (m.serverId !== serverId || m.databaseOid !== databaseOid || m.status === "gone") continue;
      markDown(e, "gone", tdes.status.gone);
    }
  };

  /** Silme diyaloğu başarıyla sildi: aynı nesnenin alter sekmeleri `gone`; Database ise o veritabanının sekmeleri de. */
  const objectDropped = (serverId: string, databaseOid: number | null, object: ObjectRef) => {
    dropLookups(serverId, isServerScoped(object.kind) ? undefined : databaseOid);
    for (const e of entries.values()) {
      const m = e.model();
      if (!matchesObject(m, { serverId, databaseOid, object }) || m.status === "gone") continue;
      markDown(e, "gone", tdes.status.dropped);
    }
    if (object.kind === "database") databaseDropped(serverId, object.oid);
  };

  /** Sekme kapandı: önizleyici kapanır, uçuştaki yanıtlar yoksayılır, uçuştaki apply için `ddl_cancel`. */
  const close = (tabId: string) => {
    const e = entries.get(tabId);
    if (!e) return;
    entries.delete(tabId);
    secrets.delete(tabId);
    bump();
    e.gen++;
    e.previewer.dispose();
    const m = e.model();
    if (m.applying && m.applyId) cancelQuietly(m.applyId);
  };

  // ---- silme diyaloğu (tek örnek; sekme açmaz). Başarıda aynı nesnenin alter sekmeleri `gone`.
  const drop = createDropController({
    api,
    notify,
    serverInfo: deps.serverInfo,
    refreshNode: (id) => refresh(id),
    objectDropped,
    lockTimeoutMs: DEFAULT_LOCK_TIMEOUT_MS,
    previewDeps: pd,
    newId,
  });

  // ---- lookup
  /** Sunucu/veritabanı başına önbellek; aynı istek tek uçuşta; hata önbelleğe alınmaz. Bilinmeyen sekme → []. */
  const lookup = (tabId: string, request: LookupRequest): Promise<LookupItem[]> => {
    const m = model(tabId);
    if (!m) return Promise.resolve([]);
    const key = lookupKey(m.serverId, m.databaseOid, request);
    const hit = lookups.get(key);
    if (hit && (hit.at === null || SESSION_LOOKUPS.has(request.kind) || now() - hit.at < LOOKUP_TTL_MS)) return hit.promise;
    // Süresi dolmuş kısa ömürlü girdiler (ör. her yazılan tip öneki, hedef tablo/kolon) eklemede atılır: harita büyümez.
    const t = now();
    for (const [k, v] of lookups) if (!v.session && v.at !== null && t - v.at >= LOOKUP_TTL_MS) lookups.delete(k);
    const slot: LookupSlot = { promise: Promise.resolve([]), at: null, session: SESSION_LOOKUPS.has(request.kind) };
    slot.promise = api.ddlLookup(m.serverId, m.databaseOid, request).then(
      (items) => {
        slot.at = now();
        return items;
      },
      (err: unknown) => {
        if (lookups.get(key) === slot) lookups.delete(key);
        throw err;
      },
    );
    lookups.set(key, slot);
    return slot.promise;
  };

  // ---- parola
  /**
   * Formun girdiği parola (iki alan eşleşince değer, aksi halde `null`). Yalnızca taslak rolün parola niyeti `set` ve
   * sekme `ready` iken saklanır; aksi halde (`null`/boş, bilinmeyen sekme, başka tür/niyet, yükleniyor/kesik) silinir.
   * `registry` yalnızca saklanan değer gerçekten değişince dürtülür (`secretReady` ve kapı tepkisel).
   */
  const setSecret = (tabId: string, value: string | null): void => {
    const m = entries.get(tabId)?.model();
    const d = m?.draft;
    const accept = !!value && !!m && m.status === "ready" && m.variant === "object" && !!d && isObjectDef(d) && d.kind === "role"
      && d.password === "set";
    if (accept) {
      if (secrets.get(tabId) === value) return;
      secrets.set(tabId, value!);
      bump();
    } else {
      forgetSecret(tabId);
    }
  };
  /** Sekmenin sunucu kaydındaki kullanıcı adı (kendini kilitleme uyarıları için; kayıt yoksa `null`). */
  const connectedUser = (tabId: string): string | null => {
    const m = model(tabId);
    return m ? (deps.serverInfo(m.serverId)?.username ?? null) : null;
  };

  // ---- odak
  /** Formdan alan odağı ister (ör. "Yeniden adlandır" → `name`); her çağrı yeni `seq` (aynı alan tekrar odaklanır). */
  const requestFocus = (tabId: string, field: string) => {
    entries.get(tabId)?.setFocus({ field, seq: ++focusSeq });
  };

  /**
   * Aynı nesnenin (`gone` olmayan) alter sekmesi — create'ten dönüşen ya da kimliği `created` ile güncellenen dahil.
   * Yalnız nesne sekmeleri (yetki sekmeleri kendi anahtarlarıyla tekilleşir). Tepkisel değil.
   */
  const findAlter = (serverId: string, databaseOid: number | null, object: ObjectRef): string | null => {
    for (const [id, e] of entries) {
      const m = e.model();
      if (m.variant === "object" && m.status !== "gone" && matchesObject(m, { serverId, databaseOid, object })) return id;
    }
    return null;
  };

  return {
    model,
    findAlter,
    ids: () => (registry(), [...entries.keys()]),
    openCreate,
    openAlter,
    openAcl,
    openDefaultAcl,
    setAclRole,
    edit,
    editAcl,
    editDefaultAcl,
    setLockTimeout,
    setConfirm,
    setConcurrently,
    setCascade,
    setSecret,
    secretReady,
    connectedUser,
    previewNow,
    apply,
    cancelApply,
    reload,
    lookup,
    /** Önbellekteki lookup girdisi sayısı (test/teşhis). */
    lookupCacheSize: () => lookups.size,
    serverChanged,
    serverSessionsClosed,
    objectDropped,
    databaseDropped,
    openCreatedAsAlter,
    drop,
    close,
    isDirty: (tabId: string) => model(tabId)?.dirty ?? false,
    peekApplyGate: (tabId: string): ApplyGate => {
      const m = model(tabId);
      return m ? gateFor(m, false) : { kind: "blocked", reason: "disconnected" };
    },
    focusRequest: (tabId: string): FocusRequest | null => (registry(), entries.get(tabId)?.focus() ?? null),
    /** Odak isteği uygulandı: yalnızca hâlâ aynı `seq` bekliyorsa temizler (yeni istek ezilmez); yeniden bağlanan form eski isteği tekrar uygulamaz. */
    consumeFocus: (tabId: string, seq: number) => {
      const e = entries.get(tabId);
      if (e?.focus()?.seq === seq) e.setFocus(null);
    },
    requestFocus,
    /** `requestFocus` ile aynı (Task 4 adı). */
    focusField: requestFocus,
  };
}

export type DesignerController = ReturnType<typeof createDesignerController>;
