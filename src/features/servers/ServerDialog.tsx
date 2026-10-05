import { open as openFile } from "@tauri-apps/plugin-dialog";
import { For, Match, Show, Switch, createEffect, createSignal } from "solid-js";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SslMode } from "../../ipc/bindings/SslMode";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import { TabPanel, Tabs } from "../../ui/Tabs";
import { SERVER_COLORS, emptyForm, formFromConfig, suggestLock, toConfig, type FormTab, type ServerForm } from "./server-form";

const SSL_MODES: SslMode[] = ["disable", "prefer", "require", "verify-ca", "verify-full"];
type Feedback = { kind: "ok"; text: string } | { kind: "error"; error: ErrorPayload } | null;
type CertKey = "rootCert" | "clientCert" | "clientKey";

export function ServerDialog(props: {
  open: boolean;
  editing: ServerConfig | null;
  groups: string[];
  onClose: () => void;
  /** `passwordTyped`: diyalogda parola yazıldı (arka uç oturumları kapatır). */
  onSaved: (config: ServerConfig, passwordTyped: boolean) => void;
  /** Kayıttan önce (ör. açık transaction uyarısı); `false` kaydı durdurur. */
  beforeSave?: (prev: ServerConfig | null, next: ServerConfig, passwordTyped: boolean) => Promise<boolean>;
}) {
  const [form, setForm] = createSignal<ServerForm>(emptyForm());
  const [tab, setTab] = createSignal<FormTab>("general");
  const [feedback, setFeedback] = createSignal<Feedback>(null);
  const [busy, setBusy] = createSignal(false);
  const [lockTouched, setLockTouched] = createSignal(false);
  let nameEl: HTMLInputElement | undefined;
  let generation = 0;

  createEffect(() => {
    generation++;
    if (!props.open) {
      setForm(emptyForm());
      setBusy(false);
      return;
    }
    setForm(props.editing ? formFromConfig(props.editing) : emptyForm());
    setLockTouched(false);
    setTab("general");
    setFeedback(null);
    setBusy(false);
  });

  const set = <K extends keyof ServerForm>(key: K, value: ServerForm[K]) => setForm((f) => ({ ...f, [key]: value }));
  const text = (key: Exclude<keyof ServerForm, "savePassword" | "sslMode" | "color" | "readOnlyLock" | "label">) => ({
    get value() {
      return form()[key];
    },
    onInput: (e: InputEvent & { currentTarget: HTMLInputElement }) => set(key, e.currentTarget.value),
  });

  const validated = () => {
    const result = toConfig(form(), props.editing?.id ?? crypto.randomUUID());
    if (!result.ok) {
      setTab(result.tab);
      setFeedback({ kind: "error", error: { code: "invalid", message: result.error, detail: null, hint: null } });
      return null;
    }
    return result;
  };

  const runTest = async () => {
    const r = validated();
    if (!r) return;
    const gen = generation;
    setBusy(true);
    setFeedback(null);
    try {
      const info = await commands.testConnection(r.config, r.password);
      if (gen === generation) setFeedback({ kind: "ok", text: t.server.testOk(info.version) });
    } catch (e) {
      if (gen === generation) setFeedback({ kind: "error", error: toPayload(e) });
    } finally {
      if (gen === generation) setBusy(false);
    }
  };

  const save = async (e: SubmitEvent) => {
    e.preventDefault();
    if (busy()) return;
    const r = validated();
    if (!r) return;
    const gen = generation;
    setBusy(true);
    try {
      if (props.beforeSave && !(await props.beforeSave(props.editing, r.config, r.password !== null))) return;
      if (gen !== generation) return;
      const saved = await commands.saveServer(r.config, r.password);
      if (gen === generation) {
        setForm(emptyForm());
        props.onSaved(saved, r.password !== null);
      }
    } catch (err) {
      if (gen === generation) setFeedback({ kind: "error", error: toPayload(err) });
    } finally {
      if (gen === generation) setBusy(false);
    }
  };

  const pick = async (key: CertKey) => {
    try {
      const path = await openFile({ multiple: false, directory: false, filters: [{ name: "PEM", extensions: ["pem", "crt", "cer", "key"] }] });
      if (typeof path === "string") set(key, path);
    } catch (e) {
      setFeedback({ kind: "error", error: toPayload(e) });
    }
  };

  const FileField = (p: { label: string; key: CertKey }) => (
    <Field label={p.label}>
      <div class="flex gap-1">
        <input class={inputClass} {...text(p.key)} />
        <Button onClick={() => void pick(p.key)}>{t.server.browse}</Button>
      </div>
    </Field>
  );

  return (
    <Dialog
      open={props.open}
      onOpenChange={(o) => !o && props.onClose()}
      title={props.editing ? t.server.titleEdit : t.server.titleNew}
      width="520px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        nameEl?.focus();
      }}
      footer={
        <>
          <Button class="mr-auto" disabled={busy()} onClick={() => void runTest()}>
            {busy() ? t.server.testing : t.server.test}
          </Button>
          <Button onClick={props.onClose}>{t.server.cancel}</Button>
          <Button variant="primary" type="submit" form="pgt-server-form" disabled={busy()}>
            {t.server.save}
          </Button>
        </>
      }
    >
      <form id="pgt-server-form" class="flex flex-col gap-3" onSubmit={(e) => void save(e)}>
        <Tabs
          value={tab()}
          onChange={(v) => setTab(v as FormTab)}
          tabs={[
            { value: "general", label: t.server.tabs.general },
            { value: "connection", label: t.server.tabs.connection },
            { value: "ssl", label: t.server.tabs.ssl },
            { value: "security", label: t.server.tabs.security },
          ]}
        >
          <TabPanel value="general" class="flex flex-col gap-3 pt-3">
            <Field label={t.server.name}>
              <input ref={nameEl} class={inputClass} {...text("name")} />
            </Field>
            <Field label={t.server.group}>
              <input class={inputClass} list="pgt-groups" {...text("group")} />
              <datalist id="pgt-groups">
                <For each={props.groups}>{(g) => <option value={g} />}</For>
              </datalist>
            </Field>
          </TabPanel>
          <TabPanel value="connection" class="grid grid-cols-[1fr_96px] gap-3 pt-3">
            <Field label={t.server.host}>
              <input class={inputClass} {...text("host")} />
            </Field>
            <Field label={t.server.port}>
              <input class={inputClass} inputMode="numeric" {...text("port")} />
            </Field>
            <div class="col-span-2">
              <Field label={t.server.maintenanceDb}>
                <input class={inputClass} {...text("maintenanceDb")} />
              </Field>
            </div>
            <div class="col-span-2">
              <Field label={t.server.username}>
                <input class={inputClass} {...text("username")} />
              </Field>
            </div>
            <div class="col-span-2">
              <Field label={t.server.password} hint={props.editing ? t.server.passwordKeep : undefined}>
                <input type="password" class={inputClass} {...text("password")} />
              </Field>
            </div>
            <label class="col-span-2 flex items-center gap-2">
              <input type="checkbox" checked={form().savePassword} onChange={(e) => set("savePassword", e.currentTarget.checked)} />
              {t.server.savePassword}
            </label>
          </TabPanel>
          <TabPanel value="ssl" class="flex flex-col gap-3 pt-3">
            <Field label={t.server.sslMode}>
              <select class={inputClass} onChange={(e) => set("sslMode", e.currentTarget.value as SslMode)}>
                <For each={SSL_MODES}>{(m) => <option value={m} selected={form().sslMode === m}>{m}</option>}</For>
              </select>
            </Field>
            <FileField label={t.server.rootCert} key="rootCert" />
            <FileField label={t.server.clientCert} key="clientCert" />
            <FileField label={t.server.clientKey} key="clientKey" />
          </TabPanel>
          <TabPanel value="security" class="flex flex-col gap-3 pt-3">
            <Field label={t.server.label} hint={t.server.labelHint}>
              <input
                class={inputClass}
                list="pgt-labels"
                maxLength={32}
                value={form().label}
                onInput={(e) => {
                  const next = e.currentTarget.value;
                  setForm((f) => ({ ...f, label: next, readOnlyLock: suggestLock(f.label, next, f.readOnlyLock, lockTouched()) }));
                }}
              />
              <datalist id="pgt-labels">
                <For each={t.server.labelSuggestions}>{(l) => <option value={l} />}</For>
              </datalist>
            </Field>
            <div class="flex flex-col gap-1">
              <span class="text-xs text-muted">{t.server.color}</span>
              <div class="flex flex-wrap gap-1.5" role="radiogroup" aria-label={t.server.color}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={form().color === null}
                  title={t.server.colorNone}
                  aria-label={t.server.colorNone}
                  class="h-6 w-6 rounded-full border border-dashed border-border text-[11px] text-muted"
                  classList={{ "ring-2 ring-accent ring-offset-1 ring-offset-bg": form().color === null }}
                  onClick={() => set("color", null)}
                >
                  ×
                </button>
                <For each={SERVER_COLORS}>
                  {(c) => (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={form().color === c}
                      title={t.server.colors[c]}
                      aria-label={t.server.colors[c]}
                      class="h-6 w-6 rounded-full border border-border"
                      classList={{ "ring-2 ring-accent ring-offset-1 ring-offset-bg": form().color === c }}
                      style={{ background: `var(--srv-${c})` }}
                      onClick={() => set("color", c)}
                    />
                  )}
                </For>
              </div>
            </div>
            <label class="flex items-start gap-2">
              <input
                type="checkbox"
                class="mt-0.5"
                checked={form().readOnlyLock}
                onChange={(e) => {
                  setLockTouched(true);
                  set("readOnlyLock", e.currentTarget.checked);
                }}
              />
              <span class="flex flex-col">
                {t.server.readOnlyLock}
                <span class="text-xs text-muted">{t.server.readOnlyLockHint}</span>
              </span>
            </label>
          </TabPanel>
        </Tabs>
        <Show when={feedback()} keyed>
          {(fb) => (
            <Switch>
              <Match when={fb.kind === "ok"}>
                <p class="text-[13px] text-green-600 dark:text-green-400">{fb.kind === "ok" ? fb.text : ""}</p>
              </Match>
              <Match when={fb.kind === "error"}>
                <div class="text-[13px] text-danger">
                  <p>{fb.kind === "error" ? fb.error.message : ""}</p>
                  <Show when={fb.kind === "error" ? fb.error.detail : null}>
                    <p class="mt-1 text-xs opacity-80">{fb.kind === "error" ? fb.error.detail : ""}</p>
                  </Show>
                </div>
              </Match>
            </Switch>
          )}
        </Show>
      </form>
    </Dialog>
  );
}
