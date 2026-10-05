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
import { emptyForm, formFromConfig, toConfig, type FormTab, type ServerForm } from "./server-form";

const SSL_MODES: SslMode[] = ["disable", "prefer", "require", "verify-ca", "verify-full"];
type Feedback = { kind: "ok"; text: string } | { kind: "error"; error: ErrorPayload } | null;
type CertKey = "rootCert" | "clientCert" | "clientKey";

export function ServerDialog(props: {
  open: boolean;
  editing: ServerConfig | null;
  groups: string[];
  onClose: () => void;
  onSaved: (config: ServerConfig) => void;
}) {
  const [form, setForm] = createSignal<ServerForm>(emptyForm());
  const [tab, setTab] = createSignal<FormTab>("general");
  const [feedback, setFeedback] = createSignal<Feedback>(null);
  const [busy, setBusy] = createSignal(false);
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
    setTab("general");
    setFeedback(null);
    setBusy(false);
  });

  const set = <K extends keyof ServerForm>(key: K, value: ServerForm[K]) => setForm((f) => ({ ...f, [key]: value }));
  const text = (key: Exclude<keyof ServerForm, "savePassword" | "sslMode" | "color" | "readOnlyLock">) => ({
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
      const saved = await commands.saveServer(r.config, r.password);
      if (gen === generation) {
        setForm(emptyForm());
        props.onSaved(saved);
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
