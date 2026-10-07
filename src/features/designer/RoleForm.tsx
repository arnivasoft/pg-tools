// Role formu (tembel parça): Genel, Öznitelikler, Bağlantı (sınır + VALID UNTIL), Parola (yalnızca yazılır), Üye olduğu
// roller, Ayarlar (ortak SettingsEditor), Düzenlenmeyenler. İstemci SQL üretmez. Parola DEĞERİ yalnızca bu bileşenin yerel
// sinyallerinde ve denetleyicinin `secrets` haritasında (`setSecret`) bulunur; taslağa yalnızca niyet (`password`) girer.
// Kobalte yok; ikonlar yalnızca Trash2/Undo.
import { Trash2, Undo } from "lucide-solid";
import { For, Index, Show, createEffect, createResource, createSignal, createUniqueId, on, onCleanup, type JSX } from "solid-js";
import { trole } from "../../app/strings-role";
import type { PasswordChange } from "../../ipc/bindings/PasswordChange";
import type { RoleDef } from "../../ipc/bindings/RoleDef";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import { connectionLimitFromInput, unlimitedToggle } from "./database-model";
import { issuesFor, type FieldIssues } from "./designer-model";
import { CheckRow, CommentField, FieldRow, createFocusOnRequest, invalidClass, type FormProps } from "./FormParts";
import { exactIssues, normalizeEmpty, orphanRowIssues } from "./forms-model";
import {
  ROLE_FLAGS, addMembership, asRole, availableRoles, passwordChoices, passwordIssue, privilegedGrants, removeMembership, removedMemberships,
  restoreMembership, roleEditPatch, selfLockoutWarnings, toggleAdmin, validUntilClearValue, validUntilFromParts, validUntilParts,
} from "./role-model";
import { SettingsEditor } from "./SettingsEditor";

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

// Sınırsız kutusunun hatası yok: connectionLimit hataları sayı alanında gösterilir.
const NO_ISSUES: FieldIssues = { byPath: new Map(), general: [], generalPaths: [] };
const RESERVED = (n: string) => {
  const l = n.toLowerCase();
  return l === "public" || l === "none" || l.startsWith("pg_");
};
const join = (...ids: (string | false | null | undefined)[]) => ids.filter(Boolean).join(" ") || undefined;

/** Yalnızca yazılır parola girdileri. Değerler yerel sinyallerde; eşleşince `setSecret(değer)`, aksi halde `setSecret(null)`. */
function PasswordInputs(props: FormProps & { mode: "create" | "alter"; intent: PasswordChange; disabled: boolean; onIntent: (c: PasswordChange) => void }) {
  const id = createUniqueId();
  const [a, setA] = createSignal("");
  const [b, setB] = createSignal("");
  // Denetleyicide bu girdilerden saklanan parola var mı (yalnızca bayrak; değerin kopyası tutulmaz).
  let sentReady = false;
  const clearInputs = () => {
    setA("");
    setB("");
  };
  // Tek bir girdi olayı eşleşen iki alandan birini değiştirince eşleşme bozulur; bu yüzden `ok` dalı yalnızca eşleşme
  // durumu false→true geçtiğinde (yeni değerle) çalışır. Denetleyici aynı değeri yine de yok sayar (dürtme yok).
  const sync = () => {
    if (props.intent !== "set") return;
    const ok = passwordIssue(a(), b()) === null;
    if (ok) {
      props.designer.setSecret(props.tabId, a());
      sentReady = props.designer.secretReady(props.tabId);
    } else if (sentReady) {
      sentReady = false;
      props.designer.setSecret(props.tabId, null);
    }
  };
  // Denetleyici parolayı kendisi sildiyse (başarılı uygulama, yeniden yükleme, bağlantı kaybı): girdiler temizlenir.
  createEffect(
    on(
      () => props.designer.secretReady(props.tabId),
      (ready) => {
        if (!ready && sentReady) {
          sentReady = false;
          clearInputs();
        }
      },
      { defer: true },
    ),
  );
  // Niyet `set` dışına çıktı ya da sekme düzenlenemez duruma (yükleniyor/kesik/yok) geçti: girdiler temizlenir.
  createEffect(
    on(
      () => props.intent !== "set" || props.model.status !== "ready",
      (leave) => {
        if (!leave) return;
        clearInputs();
        if (sentReady) {
          sentReady = false;
          props.designer.setSecret(props.tabId, null);
        }
      },
    ),
  );
  onCleanup(() => {
    clearInputs();
    if (sentReady) {
      sentReady = false;
      props.designer.setSecret(props.tabId, null);
    }
  });

  const errors = () => issuesFor(props.issues, "password");
  const local = () => (props.intent === "set" ? passwordIssue(a(), b()) : null);
  const choices = () => passwordChoices(props.mode);
  return (
    <fieldset class="flex flex-col gap-2" aria-describedby={join(errors().length > 0 && `${id}-err`, `${id}-privacy`)}>
      <legend class="mb-1 text-xs text-muted">{trole.password.legend}</legend>
      <div role="radiogroup" aria-label={trole.password.legend} class="flex flex-wrap gap-4">
        <For each={choices()}>
          {(c) => (
            <label class="flex items-center gap-2 text-[13px]">
              <input
                type="radio"
                name={`${id}-intent`}
                value={c}
                disabled={props.disabled}
                checked={props.intent === c}
                aria-invalid={errors().length > 0}
                onChange={(ev) => {
                  if (ev.currentTarget.checked) props.onIntent(c);
                }}
              />
              {trole.password.choice(c, props.mode)}
            </label>
          )}
        </For>
      </div>
      <Show when={props.intent === "set"}>
        <div class="flex flex-col gap-1">
          <label for={`${id}-a`} class="text-xs text-muted">
            {trole.password.next}
          </label>
          <input
            id={`${id}-a`}
            type="password"
            autocomplete="new-password"
            spellcheck={false}
            class={inputClass + invalidClass(local() === "empty" || errors().length > 0)}
            aria-describedby={join(local() === "empty" && `${id}-local`, errors().length > 0 && `${id}-err`, `${id}-privacy`)}
            aria-invalid={local() === "empty" || errors().length > 0}
            disabled={props.disabled}
            value={a()}
            onInput={(ev) => {
              setA(ev.currentTarget.value);
              sync();
            }}
          />
        </div>
        <div class="flex flex-col gap-1">
          <label for={`${id}-b`} class="text-xs text-muted">
            {trole.password.repeat}
          </label>
          <input
            id={`${id}-b`}
            type="password"
            autocomplete="new-password"
            spellcheck={false}
            class={inputClass + invalidClass(local() === "mismatch")}
            aria-describedby={join(local() === "mismatch" && `${id}-local`, `${id}-privacy`)}
            aria-invalid={local() === "mismatch"}
            disabled={props.disabled}
            value={b()}
            onInput={(ev) => {
              setB(ev.currentTarget.value);
              sync();
            }}
          />
        </div>
        <Show when={local()}>
          {(k) => (
            <span id={`${id}-local`} class="text-xs text-danger">
              {k() === "empty" ? trole.password.empty : trole.password.mismatch}
            </span>
          )}
        </Show>
      </Show>
      <Show when={props.intent === "clear"}>
        <span class="text-xs text-warn-text">{trole.password.clearNote}</span>
      </Show>
      <Show when={errors().length > 0}>
        <span id={`${id}-err`} class="text-xs text-danger">
          {errors().join(" ")}
        </span>
      </Show>
      <p id={`${id}-privacy`} class="text-xs text-muted">
        {trole.password.privacy}
      </p>
    </fieldset>
  );
}

/** VALID UNTIL: "Süresiz" kutusu + yerel tarih/saat (UTC'ye çevrilir). Yüklü değer temizlenemez ("infinity" açık seçim). */
function ValidUntilField(props: { d: RoleDef; original: RoleDef | null; issues: FieldIssues; disabled: boolean; onChange: (v: string | null) => void }) {
  const id = createUniqueId();
  const parts = () => validUntilParts(props.d.validUntil);
  const [editing, setEditing] = createSignal(false);
  const [date, setDate] = createSignal(parts()?.date ?? "");
  const [time, setTime] = createSignal(parts()?.time ?? "");
  const [invalid, setInvalid] = createSignal(false);
  // Bu bileşenin yazdığı son değer: dışarıdan gelen değişimde (yeniden yükleme) yerel girdiler eşitlenir.
  let written: string | null | undefined;
  createEffect(
    on(
      () => props.d.validUntil,
      (v) => {
        if (v === written) return;
        written = undefined;
        const p = validUntilParts(v);
        setDate(p && !p.never ? p.date : "");
        setTime(p && !p.never ? p.time : "");
        setEditing(false);
        setInvalid(false);
      },
      { defer: true },
    ),
  );
  const write = (v: string | null) => {
    written = v;
    props.onChange(v);
  };
  const never = () => parts()?.never ?? false;
  const showInputs = () => !never() || editing();
  const loaded = () => (props.original?.validUntil ?? null) !== null;
  const errors = () => issuesFor(props.issues, "validUntil");
  const commit = () => {
    if (date() === "") {
      setInvalid(false);
      // Yüklü değer temizlenemez: önceki değer korunur (ipucu gösterilir); aksi halde belirtilmemiş (null).
      if (!loaded()) write(null);
      return;
    }
    const v = validUntilFromParts({ never: false, date: date(), time: time() });
    setInvalid(v === null);
    if (v !== null) write(v);
  };
  const desc = () => join(errors().length > 0 && `${id}-err`, `${id}-tz`, invalid() && `${id}-inv`);
  return (
    <fieldset class="flex flex-col gap-2" aria-describedby={desc()}>
      <legend class="mb-1 text-xs text-muted">{trole.validUntil.legend}</legend>
      <label class="flex items-center gap-2 text-[13px]">
        <input
          type="checkbox"
          aria-describedby={desc()}
          aria-invalid={errors().length > 0}
          disabled={props.disabled}
          checked={!showInputs()}
          onChange={(ev) => {
            if (ev.currentTarget.checked) {
              setEditing(false);
              setInvalid(false);
              write(validUntilClearValue(props.original));
            } else {
              setEditing(true);
            }
          }}
        />
        {trole.validUntil.never}
      </label>
      <Show when={showInputs()}>
        <div class="flex flex-wrap items-end gap-3">
          <div class="flex flex-col gap-1">
            <label for={`${id}-date`} class="text-xs text-muted">
              {trole.validUntil.date}
            </label>
            <input
              id={`${id}-date`}
              type="date"
              class={inputClass + invalidClass(errors().length > 0 || invalid())}
              aria-describedby={desc()}
              aria-invalid={errors().length > 0 || invalid()}
              disabled={props.disabled}
              value={date()}
              onInput={(ev) => {
                setDate(ev.currentTarget.value);
                commit();
              }}
            />
          </div>
          <div class="flex flex-col gap-1">
            <label for={`${id}-time`} class="text-xs text-muted">
              {trole.validUntil.time}
            </label>
            <input
              id={`${id}-time`}
              type="time"
              class={inputClass + invalidClass(errors().length > 0 || invalid())}
              aria-describedby={desc()}
              aria-invalid={errors().length > 0 || invalid()}
              disabled={props.disabled}
              value={time()}
              onInput={(ev) => {
                setTime(ev.currentTarget.value);
                commit();
              }}
            />
          </div>
          <span class="pb-1 text-xs text-muted">{trole.validUntil.utc}</span>
        </div>
      </Show>
      <Show when={props.d.validUntil}>
        {(v) => (
          <span class="font-mono text-xs text-muted">
            {parts() === null ? trole.validUntil.unparsed(v()) : trole.validUntil.stored(v())}
          </span>
        )}
      </Show>
      <Show when={invalid()}>
        <span id={`${id}-inv`} class="text-xs text-warn-text">
          {trole.validUntil.invalid}
        </span>
      </Show>
      <Show when={errors().length > 0}>
        <span id={`${id}-err`} class="text-xs text-danger">
          {errors().join(" ")}
        </span>
      </Show>
      <span id={`${id}-tz`} class="text-xs text-muted">
        {loaded() ? `${trole.validUntil.tzHelp} ${trole.validUntil.noClear}` : trole.validUntil.tzHelp}
      </span>
    </fieldset>
  );
}

function Memberships(props: FormProps & { d: RoleDef; original: RoleDef | null; disabled: boolean; onChange: (d: RoleDef) => void }) {
  const selectId = createUniqueId();
  const [pick, setPick] = createSignal("");
  const [roles] = createResource(
    () => props.tabId,
    (tabId) => props.designer.lookup(tabId, { kind: "grantable_roles" }).catch(() => []),
  );
  const choices = () => availableRoles(roles() ?? [], props.d);
  const removed = () => removedMemberships(props.original, props.d);
  const listErrors = () => [...exactIssues(props.issues, "memberOf"), ...orphanRowIssues(props.issues, "memberOf", props.d.memberOf.length)];
  const add = () => {
    const r = pick();
    if (r === "") return;
    props.onChange(addMembership(props.d, r));
    setPick("");
  };
  return (
    <Section title={trole.sections.memberships}>
      <Show when={props.d.memberOf.length === 0}>
        <p class="text-xs text-muted">{trole.memberships.empty}</p>
      </Show>
      <ul role="list" class="flex flex-col gap-2">
        <Index each={props.d.memberOf}>
          {(m, i) => {
            const id = createUniqueId();
            const rowErrors = () => issuesFor(props.issues, `memberOf[${i}]`);
            return (
              <li class="flex flex-col gap-1 rounded border border-border px-2 py-1.5">
                <div class="flex flex-wrap items-center gap-3 text-[13px]">
                  <span class="min-w-0 truncate font-mono font-medium">{m().role}</span>
                  <label class="flex items-center gap-2">
                    <input
                      type="checkbox"
                      aria-label={trole.memberships.adminLabel(m().role)}
                      aria-describedby={rowErrors().length > 0 ? `${id}-err` : undefined}
                      aria-invalid={rowErrors().length > 0}
                      disabled={props.disabled}
                      checked={m().admin}
                      onChange={(ev) => props.onChange(toggleAdmin(props.d, i, ev.currentTarget.checked))}
                    />
                    {trole.memberships.admin}
                  </label>
                  <Show when={m().inheritOption !== null}>
                    <span class="rounded bg-panel px-1.5 text-[11px] text-muted" title={trole.memberships.pg16Hint}>
                      {trole.memberships.inherit(m().inheritOption === true)}
                    </span>
                  </Show>
                  <Show when={m().setOption !== null}>
                    <span class="rounded bg-panel px-1.5 text-[11px] text-muted" title={trole.memberships.pg16Hint}>
                      {trole.memberships.set(m().setOption === true)}
                    </span>
                  </Show>
                  <div class="ml-auto">
                    <Button
                      variant="ghost"
                      disabled={props.disabled}
                      aria-label={trole.memberships.remove(m().role)}
                      title={trole.memberships.remove(m().role)}
                      onClick={() => props.onChange(removeMembership(props.d, i))}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                    </Button>
                  </div>
                </div>
                <Show when={rowErrors().length > 0}>
                  <span id={`${id}-err`} class="text-xs text-danger">
                    {rowErrors().join(" ")}
                  </span>
                </Show>
              </li>
            );
          }}
        </Index>
      </ul>
      <Show when={props.d.memberOf.some((m) => m.inheritOption !== null || m.setOption !== null)}>
        <p class="text-xs text-muted">{trole.memberships.pg16Hint}</p>
      </Show>
      <Show when={listErrors().length > 0}>
        <span role="alert" class="text-xs text-danger">
          {listErrors().join(" ")}
        </span>
      </Show>
      <div class="flex items-end gap-2">
        <div class="flex min-w-0 flex-1 flex-col gap-1">
          <label for={selectId} class="text-xs text-muted">
            {trole.memberships.addLabel}
          </label>
          <select
            id={selectId}
            class={inputClass}
            disabled={props.disabled || choices().length === 0}
            value={pick()}
            onChange={(ev) => setPick(ev.currentTarget.value)}
          >
            <option value="">{choices().length === 0 ? trole.memberships.none : trole.memberships.choose}</option>
            <For each={choices()}>{(r) => <option value={r.value}>{r.detail ? `${r.value} — ${r.detail}` : r.value}</option>}</For>
          </select>
        </div>
        <Button disabled={props.disabled || pick() === ""} onClick={add}>
          {trole.memberships.add}
        </Button>
      </div>
      <Show when={removed().length > 0}>
        <div class="flex flex-col gap-1">
          <span class="text-xs font-medium">{trole.memberships.removed}</span>
          <ul role="list" aria-label={trole.memberships.removed} class="flex flex-col gap-1">
            <For each={removed()}>
              {(m) => (
                <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
                  <span class="font-mono font-medium line-through">{m.role}</span>
                  <Show when={!props.disabled && props.original}>
                    {(o) => (
                      <div class="ml-auto">
                        <Button
                          variant="ghost"
                          aria-label={trole.memberships.restore(m.role)}
                          title={trole.memberships.restore(m.role)}
                          onClick={() => props.onChange(restoreMembership(props.d, o(), m.role))}
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
          <span class="text-xs text-warn-text">{trole.memberships.revokeNote}</span>
        </div>
      </Show>
    </Section>
  );
}

export default function RoleForm(props: FormProps) {
  const draft = () => asRole(props.model.draft);
  const original = (): RoleDef | null => asRole(props.model.original);
  const alter = () => props.model.mode === "alter";
  const off = () => props.readOnly;
  const patch = (p: Partial<RoleDef>) => props.designer.edit(props.tabId, (d) => roleEditPatch(d, p));
  const replace = (next: RoleDef) =>
    props.designer.edit(props.tabId, (d) => (d.kind === "role" ? { ...next, kind: "role" } : d));
  let nameEl: HTMLInputElement | undefined;
  createFocusOnRequest(props, "name", () => nameEl);

  return (
    <Show when={draft()}>
      {(d) => {
        const privileged = () => privilegedGrants(original(), d());
        const lockouts = () => selfLockoutWarnings(props.designer.connectedUser(props.tabId), original(), d(), new Date());
        const unlimited = () => d().connectionLimit < 0;
        const renamed = () => alter() && original() !== null && original()!.name !== d().name;
        return (
          <form class="flex max-w-3xl flex-col gap-5" onSubmit={(ev) => ev.preventDefault()}>
            <Show when={lockouts().length > 0}>
              <div role="alert" class="rounded border border-border px-3 py-2 text-xs text-warn-text">
                <p class="font-medium">{trole.selfLockoutTitle}</p>
                <ul class="list-disc pl-4">
                  <For each={lockouts()}>{(k) => <li>{trole.selfLockout(k)}</li>}</For>
                </ul>
              </div>
            </Show>

            <Section title={trole.sections.general}>
              <FieldRow
                label={alter() ? trole.general.rename : trole.general.name}
                path="name"
                issues={props.issues}
                hint={trole.general.reservedHint}
                warning={RESERVED(d().name) ? trole.general.reservedWarn : renamed() ? trole.general.md5Rename : null}
              >
                {(a) => (
                  <input
                    id={a.id}
                    ref={nameEl}
                    class={inputClass + invalidClass(a.invalid)}
                    autocomplete="off"
                    spellcheck={false}
                    required
                    aria-required="true"
                    placeholder={trole.general.namePlaceholder}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={off()}
                    value={d().name}
                    onInput={(ev) => patch({ name: ev.currentTarget.value })}
                  />
                )}
              </FieldRow>
              <CommentField
                value={d().comment}
                issues={props.issues}
                disabled={off()}
                onChange={(v) => patch({ comment: v === null ? null : normalizeEmpty(v) })}
              />
            </Section>

            <Section title={trole.sections.attributes}>
              <For each={ROLE_FLAGS}>
                {(f) => (
                  <CheckRow
                    label={trole.flags.label(f)}
                    path={f}
                    issues={props.issues}
                    hint={trole.flags.hint[f]}
                    disabled={off()}
                    checked={d()[f]}
                    onChange={(c) => patch({ [f]: c })}
                  />
                )}
              </For>
              <Show when={privileged().length > 0}>
                <p role="note" class="text-xs text-warn-text">
                  {trole.privileged(privileged())}
                </p>
              </Show>
            </Section>

            <Section title={trole.sections.connection}>
              <FieldRow label={trole.connection.limit} path="connectionLimit" issues={props.issues} hint={trole.connection.unlimitedHint}>
                {(a) => (
                  <input
                    id={a.id}
                    type="number"
                    min={0}
                    step={1}
                    class={inputClass + invalidClass(a.invalid)}
                    aria-describedby={a.describedBy}
                    aria-invalid={a.invalid}
                    disabled={off() || unlimited()}
                    value={unlimited() ? "" : d().connectionLimit}
                    onInput={(ev) => {
                      const v = connectionLimitFromInput(ev.currentTarget.value, ev.currentTarget.validity.badInput);
                      if (v !== undefined) patch({ connectionLimit: v });
                    }}
                  />
                )}
              </FieldRow>
              <CheckRow
                label={trole.connection.unlimited}
                path="connectionLimit"
                issues={NO_ISSUES}
                disabled={off()}
                checked={unlimited()}
                onChange={(c) => patch({ connectionLimit: unlimitedToggle(original(), c) })}
              />
              <ValidUntilField d={d()} original={original()} issues={props.issues} disabled={off()} onChange={(v) => patch({ validUntil: v })} />
            </Section>

            <Section title={trole.sections.password}>
              <PasswordInputs
                {...props}
                mode={props.model.mode}
                intent={d().password}
                disabled={off()}
                onIntent={(c) => patch({ password: c })}
              />
            </Section>

            <Memberships {...props} d={d()} original={original()} disabled={off()} onChange={replace} />

            <SettingsEditor
              {...props}
              path="settings"
              list={d().settings}
              original={original()?.settings ?? null}
              onChange={(l) => patch({ settings: l })}
            />

            <Show when={alter() && d().unsupported.length > 0}>
              <Section title={trole.sections.unsupported}>
                <p class="text-xs text-muted">{trole.unsupported.note}</p>
                <ul role="list" aria-label={trole.sections.unsupported} class="flex flex-col gap-1">
                  <For each={d().unsupported}>
                    {(u) => (
                      <li class="rounded border border-border px-2 py-1 font-mono text-[12px]">
                        <span class="sr-only">
                          {u.startsWith("IN DATABASE") ? trole.unsupported.perDatabase : trole.unsupported.multiGrantor}:{" "}
                        </span>
                        {u}
                      </li>
                    )}
                  </For>
                </ul>
              </Section>
            </Show>

            <Show when={alter()}>
              <p class="text-xs text-muted">{trole.dropNote}</p>
            </Show>
          </form>
        );
      }}
    </Show>
  );
}
