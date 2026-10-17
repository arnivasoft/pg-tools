// "Yetkiler…" formu (Plan 22, spec §8; tembel parça): üst bilgi (sahip, bağlı rol, `explicit = false` notu), salt okunur
// gerekçeleri, yetki ızgarası + "Alıcı ekle", CASCADE onay kutusu ve salt okunur listeler (başka verenler, kolonlar).
// Model daraltılmadan gelir (`AclFormProps`); tanım `isAclDef` ile okunur. Tüm metinler `strings-privileges.ts`'tedir.
// Hata yolları DesignerView'da `aclGridPaths` ile ayrılır: ızgara satır/hücre hatalarını gösterir, gerisi üst şerittedir.
import { Show, createMemo, createResource } from "solid-js";
import { tpriv } from "../../app/strings-privileges";
import { AclAddGrantee } from "./AclAddGrantee";
import { AclGrid } from "./AclGrid";
import { planHasRevoke, removeGrantee } from "./acl-grid-model";
import { AclReadOnlyLists } from "./AclReadOnlyLists";
import { isAclDef } from "./designer-model";
import { CheckRow, type AclFormProps } from "./FormParts";
import { ReadOnlyReasons } from "./ReadOnlyReasons";

export default function PrivilegesForm(props: AclFormProps) {
  const def = createMemo(() => {
    const d = props.model.draft;
    return d && isAclDef(d) ? d : null;
  });
  return (
    <Show when={def()}>
      {(d) => {
        const [roles] = createResource(
          () => props.tabId,
          (tabId) => props.designer.lookup(tabId, { kind: "roles" }).catch(() => []),
        );
        const roleNames = () => (roles() ?? []).map((r) => r.value);
        // Salt okunur tanımda (`editable = false`) hiçbir denetim çalışmaz.
        const ro = () => props.readOnly || !d().editable;
        const revoke = () => planHasRevoke(props.model.view);
        return (
          <div class="flex max-w-4xl flex-col gap-3">
            <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[13px]">
              <dt class="text-muted">{tpriv.owner}</dt>
              <dd class="font-mono">{d().owner}</dd>
              <dt class="text-muted">{tpriv.viewer}</dt>
              <dd class="font-mono">{d().viewer}</dd>
            </dl>
            <Show when={!d().explicit}>
              <p role="note" class="rounded border border-border bg-panel px-3 py-2 text-xs text-muted">
                {tpriv.implicit}
              </p>
            </Show>
            <ReadOnlyReasons def={d()} />
            <p class="text-xs text-muted">{tpriv.legend}</p>
            <AclGrid
              grants={d().grants}
              available={d().available}
              owner={d().owner}
              basePath="grants"
              issues={props.issues}
              readOnly={ro()}
              labels={tpriv.grid}
              onChange={(grants) => props.designer.editAcl(props.tabId, (x) => ({ ...x, grants }))}
              onRemove={(row) => props.designer.editAcl(props.tabId, (x) => ({ ...x, grants: removeGrantee(x.grants, row) }))}
              footer={
                <Show when={!ro()}>
                  <AclAddGrantee
                    grants={d().grants}
                    roles={roleNames()}
                    labels={tpriv.add}
                    onAdd={(grants) => props.designer.editAcl(props.tabId, (x) => ({ ...x, grants }))}
                  />
                </Show>
              }
            />
            <CheckRow
              label={tpriv.cascade}
              path="cascade"
              issues={props.issues}
              hint={revoke() || props.model.cascade ? tpriv.cascadeHint : tpriv.cascadeIdle}
              checked={props.model.cascade}
              // REVOKE yokken devre dışı; ama işaretliyse kaldırılabilsin.
              disabled={ro() || (!revoke() && !props.model.cascade)}
              onChange={(v) => props.designer.setCascade(props.tabId, v)}
            />
            <AclReadOnlyLists foreign={d().foreign} columns={d().columns} labels={tpriv.readOnly} />
          </div>
        );
      }}
    </Show>
  );
}
