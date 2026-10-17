// "Varsayılan yetkiler…" formu (Plan 22 Task 5, spec §8; tembel parça). Üstte kapsam satırı: rol seçici (`roles` lookup;
// varsayılan bağlı rol) ve "Veritabanı geneli" / "Şema: <ad>". Rol değişimi tanımı yeniden yükler; taslak kirliyse önce
// "değişiklikleri at" onayı (karar saf `roleSwitchDecision`). Seçici yükleme/uygulama sırasında `aria-disabled`dır
// (odaklanabilir kalır; `disabled` ok tuşuyla değişimde odağı düşürürdü) ve değişimler yok sayılır (art arda değişimlerde
// yüklenmemiş kapsama dönüş yarışını önler). Rol değişimi yüklemesi başarısızsa (ör. rol yok, `role_not_found`)
// denetleyici eski kapsama döner; hata seçicinin altında satır içi gösterilir (ayrıntı üst şeritte). Yeniden yüklemede
// seçili rol kaybolduysa bağlı role düşülür ve bilgi notu gösterilir (`aclRoleGone`). Her nesne türü için bir ızgara
// (sahip = rol); şema kapsamında altında genel kapsamın salt okunur değeri. Bölümler `<Index>` ile çizilir: düzenleme
// bölüm/ızgara bileşenlerini yeniden kurmaz (odak, etkin hücre ve "Alıcı ekle" girdisi korunur). Hata yolları
// DesignerView'da `defaultAclGridPaths` ile ayrılır (baseline yolları sahiplenilmez → üst şerit).
import { For, Index, Show, createMemo, createResource, createSignal, createUniqueId } from "solid-js";
import { tdes } from "../../app/strings-designer";
import { tpriv } from "../../app/strings-privileges";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import { ConfirmDialog } from "../../ui/ConfirmDialogView";
import { inputClass } from "../../ui/Field";
import { AclAddGrantee } from "./AclAddGrantee";
import { AclGrid } from "./AclGrid";
import { removeGrantee } from "./acl-grid-model";
import { defaultAclSections, roleSwitchDecision } from "./default-acl-model";
import { isDefaultAclDef } from "./designer-model";
import type { AclFormProps } from "./FormParts";
import { ReadOnlyReasons } from "./ReadOnlyReasons";

export default function DefaultPrivilegesForm(props: AclFormProps) {
  const def = createMemo(() => {
    const d = props.model.draft;
    return d && isDefaultAclDef(d) ? d : null;
  });
  return (
    <Show when={def()}>
      {(d) => {
        const L = tpriv.defaults;
        const roleId = createUniqueId();
        const roleErrId = createUniqueId();
        const [roles] = createResource(
          () => props.tabId,
          (tabId) => props.designer.lookup(tabId, { kind: "roles" }).catch(() => []),
        );
        const roleNames = () => (roles() ?? []).map((r) => r.value);
        // Seçicide gösterilen rol: istek kapsamı (yükleme sırasında yeni rol), yoksa çözülmüş ad.
        const current = () => props.model.aclScope?.role ?? d().role;
        const roleOptions = () => [...new Set([current(), d().role, ...roleNames()])];
        const busy = () =>
          props.model.status === "loading" ||
          props.model.applying ||
          props.model.status === "gone" ||
          props.model.status === "disconnected";
        const [pending, setPending] = createSignal<string | null>(null);
        const [attempted, setAttempted] = createSignal<string | null>(null);
        const switchTo = (role: string) => {
          setAttempted(role);
          props.designer.setAclRole(props.tabId, role);
        };
        const onRole = (el: HTMLSelectElement) => {
          const next = el.value;
          const decision = roleSwitchDecision({ current: current(), next, dirty: props.model.dirty, busy: busy() });
          // Seçici kapsamı izler; onay/geçiş kapsamı değiştirene dek eski değer görünür.
          el.value = current();
          if (decision === "confirm") setPending(next);
          else if (decision === "switch") switchTo(next);
        };
        // Son rol değişimi başarısız (kapsam eski role döndü) → satır içi hata.
        const roleFailed = () => {
          const a = attempted();
          return a !== null && props.model.status === "ready" && !!props.model.error && current() !== a ? a : null;
        };
        const sections = createMemo(() => defaultAclSections(d(), props.readOnly));
        const editSection = (i: number, grants: AclGrant[]) =>
          props.designer.editDefaultAcl(props.tabId, (x) => {
            const s = x.sections[i];
            if (s) s.grants = grants;
          });
        // Düzenlenebilir tanımdaki `unsupported` notları (ör. PG 18 LARGE OBJECTS) bilgi satırıdır.
        const info = () => (d().editable ? d().unsupported : []);
        return (
          <div class="flex max-w-4xl flex-col gap-3">
            <div class="flex flex-col gap-1">
              <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-[13px]">
                <label for={roleId} class="text-muted">
                  {L.role}
                </label>
                <select
                  id={roleId}
                  class={`${inputClass} max-w-xs aria-disabled:opacity-60`}
                  value={current()}
                  aria-disabled={busy() ? true : undefined}
                  aria-invalid={roleFailed() !== null ? true : undefined}
                  aria-describedby={roleFailed() !== null ? roleErrId : undefined}
                  onChange={(ev) => onRole(ev.currentTarget)}
                >
                  <For each={roleOptions()}>{(r) => <option value={r}>{r}</option>}</For>
                </select>
                <span>
                  <span class="text-muted">{L.scope}: </span>
                  <span class="font-mono">{d().schema === null ? L.database : L.schema(d().schema ?? "")}</span>
                </span>
                <Show when={props.model.status === "loading"}>
                  <span role="status" class="text-xs text-muted">
                    {L.loading}
                  </span>
                </Show>
              </div>
              <Show when={roleFailed()}>
                {(r) => (
                  <p id={roleErrId} role="alert" class="text-xs text-danger">
                    <span class="font-mono">{r()}</span>: {L.loadError}
                  </p>
                )}
              </Show>
            </div>
            <Show when={props.model.aclRoleGone}>
              {(r) => (
                <p role="status" class="rounded border border-border px-3 py-2 text-xs text-muted">
                  {L.roleGone(r())}
                </p>
              )}
            </Show>
            <p role="note" class="rounded border border-border bg-panel px-3 py-2 text-xs text-muted">
              {L.note}
            </p>
            <ReadOnlyReasons def={d()} />
            <Show when={info().length > 0}>
              <ul role="list" aria-label={L.info} class="flex flex-col gap-1">
                <For each={info()}>
                  {(u) => <li class="rounded border border-border px-2 py-1 text-xs text-muted">{u}</li>}
                </For>
              </ul>
            </Show>
            <p class="text-xs text-muted">{tpriv.legend}</p>
            <Index each={sections()}>
              {(s, i) => {
                // `s` erişimcidir: düzenlemede bölüm nesnesi yenilenir ama bileşen (ve odak) aynı indekste kalır.
                const titleId = createUniqueId();
                const title = () => L.section[s().objectType];
                return (
                  <section aria-labelledby={titleId} class="flex flex-col gap-2 border-t border-border pt-2">
                    <h3 id={titleId} class="text-[13px] font-medium">
                      {title()}
                    </h3>
                    <AclGrid
                      grants={s().grants}
                      available={s().available}
                      owner={s().owner}
                      basePath={s().basePath}
                      issues={props.issues}
                      readOnly={s().readOnly}
                      labels={{ ...tpriv.grid, grid: tpriv.gridName(title(), tpriv.grid.grid), empty: L.empty }}
                      onChange={(grants) => editSection(i, grants)}
                      onRemove={(row) => editSection(i, removeGrantee(s().grants, row))}
                      footer={
                        <Show when={!s().readOnly}>
                          <AclAddGrantee
                            grants={s().grants}
                            roles={roleNames()}
                            labels={tpriv.add}
                            context={title()}
                            onAdd={(grants) => editSection(i, grants)}
                          />
                        </Show>
                      }
                    />
                    <Show when={s().baseline}>
                      {(b) => (
                        <div class="ml-3 flex flex-col gap-1 border-l border-border pl-3">
                          <h4 class="text-xs text-muted">{L.baseline}</h4>
                          <AclGrid
                            grants={b().grants}
                            available={s().available}
                            owner={s().owner}
                            basePath={b().basePath}
                            issues={props.issues}
                            readOnly={b().readOnly}
                            labels={{ ...tpriv.grid, grid: tpriv.gridName(title(), L.baseline), empty: L.baselineEmpty }}
                            onChange={() => {}}
                          />
                        </div>
                      )}
                    </Show>
                  </section>
                );
              }}
            </Index>
            <ConfirmDialog
              open={pending() !== null}
              title={tdes.discard.title}
              message={L.discardMessage}
              confirmLabel={L.discardConfirm}
              danger
              onCancel={() => setPending(null)}
              onConfirm={() => {
                const r = pending();
                setPending(null);
                if (r !== null && !busy()) switchTo(r);
              }}
            />
          </div>
        );
      }}
    </Show>
  );
}
