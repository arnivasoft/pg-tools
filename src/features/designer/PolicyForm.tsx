// Policy formu (tembel parça): Genel (tablo salt okunur, ad, yorum), Komut ve tür (FOR, PERMISSIVE/RESTRICTIVE), Roller
// (lookup'tan seçilen kaldırılabilir çipler; boş = PUBLIC), İfadeler (USING / WITH CHECK, komuta göre kapanır), RLS durumu
// (salt okunur) ve yeniden kurma notu (alter). İstemci SQL üretmez: ifadeler olduğu gibi gider. `tableRls`, `editable`,
// `unsupported` taslakta dokunulmadan kalır. `editable = false` tanım (ya da `readOnly` sekme) tamamen salt okunurdur.
// Kobalte yok; metinler tembel `strings-policy`.
import { For, Show, createMemo, createResource, createUniqueId, type JSX } from "solid-js";
import { tpol } from "../../app/strings-policy";
import type { PolicyCommand } from "../../ipc/bindings/PolicyCommand";
import type { PolicyDef } from "../../ipc/bindings/PolicyDef";
import { inputClass } from "../../ui/Field";
import { issuesFor } from "./designer-model";
import { CommentField, FieldRow, FreeSqlNote, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { normalizeEmpty } from "./forms-model";
import {
  addRole, asPolicy, expressionAvailability, policyCommandPatch, policyPatch, policyRebuildReasons, removeRole, roleChoices,
} from "./policy-model";
import { ReadOnlyReasons } from "./ReadOnlyReasons";

function Section(props: { title: string; children: JSX.Element }) {
  const id = createUniqueId();
  return (
    <section aria-labelledby={id} class="flex flex-col gap-3">
      <h3 id={id} class="text-[13px] font-medium">
        {props.title}
      </h3>
      {props.children}
    </section>
  );
}

function ListErrors(props: { messages: string[]; id?: string }) {
  return (
    <Show when={props.messages.length > 0}>
      <p id={props.id} role="alert" class="text-xs text-danger">
        {props.messages.join(" ")}
      </p>
    </Show>
  );
}

const COMMANDS: PolicyCommand[] = ["all", "select", "insert", "update", "delete"];

export default function PolicyForm(props: FormProps) {
  const draft = () => asPolicy(props.model.draft);
  const original = () => asPolicy(props.model.original);
  const alter = () => props.model.mode === "alter";
  /** Sekme salt okunur ya da tanım düzenlenemez: her denetim kapalı. */
  const locked = () => props.readOnly || !(draft()?.editable ?? true);

  const patch = (p: Partial<PolicyDef>) => props.designer.edit(props.tabId, (x) => policyPatch(x, p));
  /** Taslağın güncel hâli üzerinde işlem yapan düzenleme (çip ekle/kaldır, komut değişimi). */
  const update = (f: (d: PolicyDef) => PolicyDef) =>
    props.designer.edit(props.tabId, (x) => {
      const prev = asPolicy(x);
      if (!prev) return x;
      return policyPatch(x, f(prev));
    });

  const typeName = createUniqueId();
  const typeErrId = createUniqueId();
  const rolesErrId = createUniqueId();
  const rolesHintId = createUniqueId();
  const rolePickId = createUniqueId();
  const usingNoteId = createUniqueId();
  const usingWhyId = createUniqueId();
  const checkNoteId = createUniqueId();
  const checkWhyId = createUniqueId();
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  const [roleItems] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "roles" }).catch(() => null),
  );

  return (
    <Show when={draft()}>
      {(d) => {
        const a = createMemo(() => expressionAvailability(d().command));
        const reasons = () => (alter() ? policyRebuildReasons(original(), d()) : []);
        const choices = () => roleChoices((roleItems() ?? []).map((r) => r.value), d().roles);
        const topErrors = () => [...issuesFor(props.issues, "schema"), ...issuesFor(props.issues, "object")];
        // Çip başına hata (`roles[i]`) rol adıyla; listenin kendi hataları olduğu gibi.
        const roleErrors = () => {
          const out: string[] = [];
          for (const [p, msgs] of props.issues.byPath) {
            const m = /^roles\[(\d+)\]$/.exec(p);
            if (m) {
              const i = Number(m[1]);
              const name = d().roles[i];
              for (const msg of msgs) out.push(`${name ?? "#" + (i + 1)}: ${msg}`);
            } else if (p === "roles" || p.startsWith("roles.") || p.startsWith("roles[")) {
              out.push(...msgs);
            }
          }
          return out;
        };
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <ReadOnlyReasons def={d()} />
            <ListErrors messages={topErrors()} />

            <Section title={tpol.sections.general}>
              <FieldRow label={tpol.general.table} path="table" issues={props.issues} hint={tpol.general.tableHint}>
                {(f) => (
                  <input
                    id={f.id}
                    class={`${inputClass} font-mono text-muted${invalidClass(f.invalid)}`}
                    readOnly
                    aria-readonly="true"
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    value={`${d().schema}.${d().table}`}
                  />
                )}
              </FieldRow>
              <FieldRow label={alter() ? tpol.general.rename : tpol.general.name} path="name" issues={props.issues}>
                {(f) => (
                  <input
                    id={f.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(f.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={tpol.general.namePlaceholder}
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().name}
                    onInput={(ev) => patch({ name: ev.currentTarget.value })}
                  />
                )}
              </FieldRow>
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={locked()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
            </Section>

            <Section title={tpol.sections.command}>
              <FieldRow label={tpol.command.label} path="command" issues={props.issues}>
                {(f) => (
                  <select
                    id={f.id}
                    class={inputClass + invalidClass(f.invalid)}
                    aria-describedby={f.describedBy}
                    aria-invalid={f.invalid}
                    disabled={locked()}
                    value={d().command}
                    onChange={(ev) => {
                      const c = ev.currentTarget.value as PolicyCommand;
                      update((x) => policyCommandPatch(x, c));
                    }}
                  >
                    <For each={COMMANDS}>
                      {(c) => (
                        <option value={c} selected={c === d().command}>
                          {tpol.command[c]}
                        </option>
                      )}
                    </For>
                  </select>
                )}
              </FieldRow>
              <fieldset class="flex flex-col gap-1" aria-describedby={issuesFor(props.issues, "permissive").length > 0 ? typeErrId : undefined}>
                <legend class="text-xs text-muted">{tpol.command.kind}</legend>
                <label class="flex items-center gap-2 text-[13px]" classList={{ "text-muted": locked() }}>
                  <input
                    type="radio"
                    name={typeName}
                    disabled={locked()}
                    checked={d().permissive}
                    onChange={(ev) => ev.currentTarget.checked && patch({ permissive: true })}
                  />
                  <span>
                    <span class="font-mono">{tpol.command.permissive}</span> — {tpol.command.permissiveHint}
                  </span>
                </label>
                <label class="flex items-center gap-2 text-[13px]" classList={{ "text-muted": locked() }}>
                  <input
                    type="radio"
                    name={typeName}
                    disabled={locked()}
                    checked={!d().permissive}
                    onChange={(ev) => ev.currentTarget.checked && patch({ permissive: false })}
                  />
                  <span>
                    <span class="font-mono">{tpol.command.restrictive}</span> — {tpol.command.restrictiveHint}
                  </span>
                </label>
                <ListErrors id={typeErrId} messages={issuesFor(props.issues, "permissive")} />
              </fieldset>
            </Section>

            <Section title={tpol.sections.roles}>
              <div class="flex flex-col gap-2" role="group" aria-label={tpol.roles.label} aria-describedby={rolesHintId}>
                <Show
                  when={d().roles.length > 0}
                  fallback={<p class="text-[13px] text-muted">{tpol.roles.publicNote}</p>}
                >
                  <ul role="list" aria-label={tpol.roles.list} class="flex flex-wrap gap-1.5">
                    <For each={d().roles}>
                      {(r, i) => (
                        <li class="inline-flex h-6 items-center gap-1 rounded-full border border-border px-2 font-mono text-[12px]">
                          {r}
                          <button
                            type="button"
                            class="text-muted hover:text-danger focus-visible:text-danger disabled:opacity-50"
                            aria-label={tpol.roles.removeRole(r)}
                            disabled={locked()}
                            onClick={() => update((x) => removeRole(x, i()))}
                          >
                            ×
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
                <Show when={choices().length > 0}>
                  <div class="flex flex-col gap-1">
                    <label for={rolePickId} class="text-xs text-muted">
                      {tpol.roles.add}
                    </label>
                    <select
                      id={rolePickId}
                      class={inputClass}
                      disabled={locked()}
                      value=""
                      onChange={(ev) => {
                        const v = ev.currentTarget.value;
                        ev.currentTarget.value = "";
                        if (v !== "") update((x) => addRole(x, v));
                      }}
                    >
                      <option value="" selected>
                        {tpol.roles.addPick}
                      </option>
                      <For each={choices()}>{(r) => <option value={r}>{r}</option>}</For>
                    </select>
                  </div>
                </Show>
                <p id={rolesHintId} class="text-xs text-muted">
                  {tpol.roles.hint}
                </p>
                <ListErrors id={rolesErrId} messages={roleErrors()} />
              </div>
            </Section>

            <Section title={tpol.sections.expressions}>
              <FieldRow label={tpol.expr.using} path="using" issues={props.issues} hint={a().using ? tpol.expr.hint : undefined}>
                {(f) => (
                  <textarea
                    id={f.id}
                    rows={4}
                    class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(f.invalid)}`}
                    spellcheck={false}
                    placeholder={tpol.expr.usingPlaceholder}
                    aria-describedby={[f.describedBy, a().using ? usingNoteId : usingWhyId].filter(Boolean).join(" ")}
                    aria-invalid={f.invalid}
                    disabled={locked() || !a().using}
                    value={d().using ?? ""}
                    onInput={(ev) => patch({ using: normalizeEmpty(ev.currentTarget.value) })}
                  />
                )}
              </FieldRow>
              <Show
                when={a().using}
                fallback={
                  <p id={usingWhyId} class="text-xs text-muted">
                    {tpol.expr.usingClosed}
                  </p>
                }
              >
                <FreeSqlNote id={usingNoteId} />
              </Show>
              <FieldRow label={tpol.expr.withCheck} path="withCheck" issues={props.issues}>
                {(f) => (
                  <textarea
                    id={f.id}
                    rows={4}
                    class={`${inputClass} !h-auto resize-y py-1 font-mono${invalidClass(f.invalid)}`}
                    spellcheck={false}
                    placeholder={tpol.expr.withCheckPlaceholder}
                    aria-describedby={[f.describedBy, a().withCheck ? checkNoteId : checkWhyId].filter(Boolean).join(" ")}
                    aria-invalid={f.invalid}
                    disabled={locked() || !a().withCheck}
                    value={d().withCheck ?? ""}
                    onInput={(ev) => patch({ withCheck: normalizeEmpty(ev.currentTarget.value) })}
                  />
                )}
              </FieldRow>
              <Show
                when={a().withCheck}
                fallback={
                  <p id={checkWhyId} class="text-xs text-muted">
                    {tpol.expr.withCheckClosed}
                  </p>
                }
              >
                <FreeSqlNote id={checkNoteId} />
              </Show>
            </Section>

            <Show when={d().tableRls}>
              {(rls) => (
                <Section title={tpol.sections.rls}>
                  <p class="text-[13px] text-muted">{tpol.rls.line(rls().enabled, rls().forced)}</p>
                  <Show when={!rls().enabled}>
                    <p role="note" class="text-xs text-warn-text">
                      {tpol.rls.offWarning}
                    </p>
                  </Show>
                </Section>
              )}
            </Show>

            <Show when={reasons().length > 0}>
              <div role="note" class="flex flex-col gap-1 rounded border border-border px-3 py-2 text-xs">
                <p class="font-medium">{tpol.rebuild.title}</p>
                <ul role="list" class="list-disc pl-4 text-warn-text">
                  <For each={reasons()}>{(r) => <li>{tpol.rebuild.reasons[r]}</li>}</For>
                </ul>
              </div>
            </Show>
          </form>
        );
      }}
    </Show>
  );
}
