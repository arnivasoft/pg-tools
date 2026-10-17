// "Alıcı ekle" denetimi (Plan 22; tembel parça): yetki ve varsayılan yetki formlarının ortak girişi. Datalist + serbest
// yazım; Enter ya da "Ekle". Ad çözümü saf `addGranteeInput`'tadır: rol listesiyle birebir eşleşen ad gerçek roldür
// ("PUBLIC"/"Public" adlı roller dahil), aksi hâlde `public` ya da sözde seçenek PUBLIC satırıdır. Kök öğe
// `data-acl-add` taşır (AclGrid satır kaldırınca odağı buraya taşır).
import { For, Show, createSignal, createUniqueId } from "solid-js";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { addGranteeInput, granteeOptions } from "./acl-grid-model";
import { invalidClass } from "./FormParts";

export interface AclAddGranteeLabels {
  label: string;
  placeholder: string;
  button: string;
  empty: string;
  duplicate: string;
  /** PUBLIC sözde seçeneğinin datalist değeri (gerçek "PUBLIC" rolünden ayrı). */
  publicOption: string;
}

export function AclAddGrantee(props: {
  grants: AclGrant[];
  roles: readonly string[];
  labels: AclAddGranteeLabels;
  /** Erişilebilir ad eki (ör. bölüm adı); verilmezse yalnız `labels.label`. */
  context?: string;
  onAdd: (grants: AclGrant[]) => void;
}) {
  const inputId = createUniqueId();
  const listId = createUniqueId();
  const msgId = createUniqueId();
  const [text, setText] = createSignal("");
  const [addError, setAddError] = createSignal<string | null>(null);
  const add = () => {
    const r = addGranteeInput(props.grants, text(), { roles: props.roles, publicToken: props.labels.publicOption });
    if ("error" in r) {
      setAddError(r.error === "empty" ? props.labels.empty : props.labels.duplicate);
      return;
    }
    setAddError(null);
    setText("");
    props.onAdd(r.grants);
  };
  return (
    <div class="flex flex-col gap-1">
      <label for={inputId} class="text-xs text-muted">
        {props.labels.label}
        <Show when={props.context}>
          <span class="sr-only"> ({props.context})</span>
        </Show>
      </label>
      <div class="flex items-center gap-2">
        <input
          id={inputId}
          data-acl-add
          class={`${inputClass} max-w-xs${invalidClass(addError() !== null)}`}
          list={listId}
          autocomplete="off"
          spellcheck={false}
          placeholder={props.labels.placeholder}
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
          <For each={granteeOptions(props.grants, props.roles, props.labels.publicOption)}>{(o) => <option value={o} />}</For>
        </datalist>
        <Button onClick={add}>{props.labels.button}</Button>
      </div>
      <Show when={addError()}>
        <span id={msgId} role="alert" class="text-xs text-danger">
          {addError()}
        </span>
      </Show>
    </div>
  );
}
