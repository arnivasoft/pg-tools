// "Yetkiler…" formu (Plan 22, spec §8; tembel parça): üst bilgi (sahip, bağlı rol, `explicit = false` notu), salt okunur
// gerekçeleri, yetki ızgarası + "Alıcı ekle", CASCADE onay kutusu ve salt okunur listeler (başka verenler, kolonlar).
// Model daraltılmadan gelir (`AclFormProps`); tanım `isAclDef` ile okunur. Tüm metinler `strings-privileges.ts`'tedir.
// Hata yolları DesignerView'da `aclGridPaths` ile ayrılır: ızgara satır/hücre hatalarını gösterir, gerisi üst şerittedir.
import { For, Show, createMemo, createResource, createSignal, createUniqueId } from "solid-js";
import { tpriv } from "../../app/strings-privileges";
import { inputClass } from "../../ui/Field";
import { Button } from "../../ui/Button";
import { AclGrid } from "./AclGrid";
import { addGranteeInput, planHasRevoke, removeGrantee } from "./acl-grid-model";
import { AclReadOnlyLists } from "./AclReadOnlyLists";
import { isAclDef } from "./designer-model";
import { CheckRow, invalidClass, type AclFormProps } from "./FormParts";
import { ReadOnlyReasons } from "./ReadOnlyReasons";

export default function PrivilegesForm(props: AclFormProps) {
  const def = createMemo(() => {
    const d = props.model.draft;
    return d && isAclDef(d) ? d : null;
  });
  return (
    <Show when={def()}>
      {(d) => {
        const inputId = createUniqueId();
        const listId = createUniqueId();
        const msgId = createUniqueId();
        const [text, setText] = createSignal("");
        const [addError, setAddError] = createSignal<string | null>(null);
        const [roles] = createResource(
          () => props.tabId,
          (tabId) => props.designer.lookup(tabId, { kind: "roles" }).catch(() => []),
        );
        // Salt okunur tanımda (`editable = false`) hiçbir denetim çalışmaz.
        const ro = () => props.readOnly || !d().editable;
        const hasPublic = () => d().grants.some((g) => g.grantee === null);
        // Öneriler: listede olmayan roller; PUBLIC satırı yoksa "PUBLIC".
        const options = () => {
          const used = new Set(d().grants.map((g) => g.grantee));
          const names = (roles() ?? []).map((r) => r.value).filter((v) => !used.has(v));
          return hasPublic() ? names : [tpriv.publicName, ...names];
        };
        const add = () => {
          if (ro()) return;
          const r = addGranteeInput(d().grants, text());
          if ("error" in r) {
            setAddError(r.error === "empty" ? tpriv.add.empty : tpriv.add.duplicate);
            return;
          }
          setAddError(null);
          setText("");
          props.designer.editAcl(props.tabId, (x) => ({ ...x, grants: r.grants }));
        };
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
                  <div class="flex flex-col gap-1">
                    <label for={inputId} class="text-xs text-muted">
                      {tpriv.add.label}
                    </label>
                    <div class="flex items-center gap-2">
                      <input
                        id={inputId}
                        data-acl-add
                        class={`${inputClass} max-w-xs${invalidClass(addError() !== null)}`}
                        list={listId}
                        autocomplete="off"
                        spellcheck={false}
                        placeholder={tpriv.add.placeholder}
                        aria-invalid={addError() !== null}
                        aria-describedby={addError() !== null ? msgId : undefined}
                        value={text()}
                        onInput={(ev) => {
                          setText(ev.currentTarget.value);
                          setAddError(null);
                        }}
                        onKeyDown={(ev) => {
                          if (ev.isComposing || ev.keyCode === 229 || ev.key !== "Enter") return;
                          ev.preventDefault();
                          add();
                        }}
                      />
                      <datalist id={listId}>
                        <For each={options()}>{(o) => <option value={o} />}</For>
                      </datalist>
                      <Button onClick={add}>{tpriv.add.button}</Button>
                    </div>
                    <Show when={addError()}>
                      <span id={msgId} role="alert" class="text-xs text-danger">
                        {addError()}
                      </span>
                    </Show>
                  </div>
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
