// Ortak ayar düzenleyicisi (ALTER DATABASE|ROLE … SET; Database ve Role formları paylaşır; tembel parça). Satır listesi
// (ad + değer), kaldırılacaklar listesi + Geri al. İstemci SQL üretmez; uyarılar ön uyarıdır (hata değil), alan hataları
// arka uçtan gelir (`settings[i].name|value`, liste düzeyi `settings`).
import { Trash2, Undo } from "lucide-solid";
import { For, Index, Show, createResource, createUniqueId } from "solid-js";
import { tset } from "../../app/strings-settings";
import type { ConfigSetting } from "../../ipc/bindings/ConfigSetting";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { invalidClass, type FormProps } from "./FormParts";
import { exactIssues, orphanRowIssues } from "./forms-model";
import {
  addSetting, isListSetting, listItemCaseWarning, removeSetting, removedSettings, restoreSetting, settingNameWarning, updateSetting,
} from "./settings-model";

export function SettingsEditor(
  props: FormProps & { path: "settings"; list: ConfigSetting[]; original: ConfigSetting[] | null; onChange: (l: ConfigSetting[]) => void },
) {
  const headingId = createUniqueId();
  const listId = createUniqueId();
  let root: HTMLDivElement | undefined;
  const [catalog] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "settings" }).catch(() => []),
  );
  const editable = () => !props.readOnly;
  const removed = () => removedSettings(props.original, props.list);
  const listErrors = () => [...exactIssues(props.issues, props.path), ...orphanRowIssues(props.issues, props.path, props.list.length)];

  const focusLastName = () =>
    queueMicrotask(() => {
      const els = root?.querySelectorAll<HTMLInputElement>("[data-setting-name]");
      els?.[els.length - 1]?.focus();
    });

  return (
    <div ref={root} role="group" aria-labelledby={headingId} class="flex flex-col gap-3">
      <h3 id={headingId} class="text-[13px] font-medium">
        {tset.title}
      </h3>
      <p class="text-xs text-muted">{tset.note}</p>
      <datalist id={listId}>
        <For each={catalog() ?? []}>{(i) => <option value={i.value}>{i.detail ?? ""}</option>}</For>
      </datalist>
      <Show when={props.list.length === 0}>
        <p class="text-xs text-muted">{tset.empty}</p>
      </Show>
      <ul role="list" class="flex flex-col gap-2">
        <Index each={props.list}>
          {(s, idx) => {
            const i = () => idx;
            const id = createUniqueId();
            const nameErrors = () => issuesFor(props.issues, `${props.path}[${i()}].name`);
            const valueErrors = () => issuesFor(props.issues, `${props.path}[${i()}].value`);
            const nameWarning = () => settingNameWarning(props.list, i());
            const caseWarning = () => listItemCaseWarning(s().name, s().value);
            const nameDesc = () =>
              [nameErrors().length > 0 ? `${id}-nerr` : null, nameWarning() ? `${id}-nwarn` : null].filter(Boolean).join(" ") || undefined;
            const valueDesc = () =>
              [
                valueErrors().length > 0 ? `${id}-verr` : null,
                caseWarning() ? `${id}-vwarn` : null,
                isListSetting(s().name) ? `${id}-vhint` : null,
              ]
                .filter(Boolean)
                .join(" ") || undefined;
            return (
              <li class="flex flex-col gap-1 rounded border border-border px-2 py-2">
                <div class="flex items-start gap-2">
                  <input
                    data-setting-name
                    class={`${inputClass} min-w-0 flex-1 font-mono${invalidClass(nameErrors().length > 0)}`}
                    list={listId}
                    autocomplete="off"
                    spellcheck={false}
                    aria-label={`${tset.nameLabel} (${tset.rowLabel(i())})`}
                    placeholder={tset.namePlaceholder}
                    aria-describedby={nameDesc()}
                    aria-invalid={nameErrors().length > 0}
                    disabled={!editable()}
                    value={s().name}
                    onInput={(ev) => props.onChange(updateSetting(props.list, i(), { name: ev.currentTarget.value }))}
                  />
                  <input
                    class={`${inputClass} min-w-0 flex-1 font-mono${invalidClass(valueErrors().length > 0)}`}
                    autocomplete="off"
                    spellcheck={false}
                    aria-label={`${tset.valueLabel} (${tset.rowLabel(i())})`}
                    placeholder={tset.valuePlaceholder}
                    aria-describedby={valueDesc()}
                    aria-invalid={valueErrors().length > 0}
                    disabled={!editable()}
                    value={s().value}
                    onInput={(ev) => props.onChange(updateSetting(props.list, i(), { value: ev.currentTarget.value }))}
                  />
                  <Button
                    variant="ghost"
                    disabled={!editable()}
                    aria-label={tset.remove(s().name)}
                    title={tset.remove(s().name)}
                    onClick={() => props.onChange(removeSetting(props.list, i()))}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                  </Button>
                </div>
                <Show when={nameErrors().length > 0}>
                  <span id={`${id}-nerr`} class="text-xs text-danger">
                    {nameErrors().join(" ")}
                  </span>
                </Show>
                <Show when={nameWarning()}>
                  <span id={`${id}-nwarn`} class="text-xs text-warn-text">
                    {nameWarning()}
                  </span>
                </Show>
                <Show when={valueErrors().length > 0}>
                  <span id={`${id}-verr`} class="text-xs text-danger">
                    {valueErrors().join(" ")}
                  </span>
                </Show>
                <Show when={caseWarning()}>
                  <span id={`${id}-vwarn`} class="text-xs text-warn-text">
                    {caseWarning()}
                  </span>
                </Show>
                <Show when={isListSetting(s().name)}>
                  <span id={`${id}-vhint`} class="text-xs text-muted">
                    {tset.listHint}
                  </span>
                </Show>
              </li>
            );
          }}
        </Index>
      </ul>
      <Show when={listErrors().length > 0}>
        <span role="alert" class="text-xs text-danger">
          {listErrors().join(" ")}
        </span>
      </Show>
      <div>
        <Button
          disabled={!editable()}
          onClick={() => {
            props.onChange(addSetting(props.list));
            focusLastName();
          }}
        >
          {tset.add}
        </Button>
      </div>
      <Show when={removed().length > 0}>
        <div class="flex flex-col gap-1">
          <span class="text-xs font-medium">{tset.removed}</span>
          <ul role="list" aria-label={tset.removed} class="flex flex-col gap-1">
            <For each={removed()}>
              {(s) => (
                <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
                  <span class="font-mono font-medium line-through">{s.name}</span>
                  <span class="min-w-0 truncate font-mono text-muted">{s.value}</span>
                  <Show when={editable() && props.original}>
                    {(o) => (
                      <div class="ml-auto">
                        <Button
                          variant="ghost"
                          aria-label={tset.restore(s.name)}
                          title={tset.restore(s.name)}
                          onClick={() => props.onChange(restoreSetting(props.list, o(), s.name))}
                        >
                          <Undo size={14} aria-hidden="true" />
                        </Button>
                      </div>
                    )}
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <span class="text-xs text-warn-text">{tset.removedNote}</span>
        </div>
      </Show>
    </div>
  );
}
