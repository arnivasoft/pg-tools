// Asistan ayarları: sağlayıcı (Claude API / OpenRouter), sağlayıcı başına model, effort ve sağlayıcı başına API
// anahtarı. Tembel AiPane parçasının içindedir. Anahtar arayüze hiç dönmez; yalnız "kayıtlı mı" görünür.
import { For, Show, createSignal } from "solid-js";
import { toast } from "../../app/toast";
import type { AiEffort } from "../../ipc/bindings/AiEffort";
import type { AiProvider } from "../../ipc/bindings/AiProvider";
import type { AiSettings } from "../../ipc/bindings/AiSettings";
import type { AiSettingsView } from "../../ipc/bindings/AiSettingsView";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { activeHasKey, aiIpc } from "./ai-ipc";
import { tai } from "./strings-ai";

const PROVIDERS: AiProvider[] = ["anthropic", "openRouter"];
const EFFORTS: ("" | AiEffort)[] = ["", "low", "medium", "high", "xhigh", "max"];
/** Öneri listeleri (serbest metin de girilebilir). OpenRouter adları belgelerden doğrulanmış Claude adlarıdır. */
const SUGGESTIONS: Record<AiProvider, string[]> = {
  anthropic: ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-1"],
  openRouter: ["anthropic/claude-opus-5.5", "anthropic/claude-sonnet-5.5"],
};

export function AiSettingsPanel(props: {
  view: AiSettingsView;
  onChange: (v: AiSettingsView) => void;
  /** Etkin sağlayıcının anahtarı varsa sohbete dönülebilir. */
  onClose?: () => void;
}) {
  const s = tai.settings;
  // Düzenlenen taslak; "Ayarları kaydet" ile yazılır.
  const [draft, setDraft] = createSignal<AiSettings>({ ...props.view.settings });
  const [keyDraft, setKeyDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const provider = () => draft().provider;
  const model = () => (provider() === "anthropic" ? draft().anthropicModel : draft().openrouterModel);
  const setModel = (m: string) =>
    setDraft((d) => (d.provider === "anthropic" ? { ...d, anthropicModel: m } : { ...d, openrouterModel: m }));
  const hasKey = () => (provider() === "anthropic" ? props.view.hasAnthropicKey : props.view.hasOpenrouterKey);
  const dirty = () => JSON.stringify(draft()) !== JSON.stringify(props.view.settings);
  const listId = `ai-models-${Math.random().toString(36).slice(2)}`;

  const run = async (f: () => Promise<AiSettingsView>, ok: string) => {
    setBusy(true);
    setError(null);
    try {
      const v = await f();
      props.onChange(v);
      toast.show(ok, "success");
      return v;
    } catch (e) {
      setError(toPayload(e).message);
      return null;
    } finally {
      setBusy(false);
    }
  };

  const saveSettings = () => run(() => aiIpc.settingsSet(draft()), s.saved);
  const saveKey = async () => {
    // Sağlayıcı seçimi değiştiyse önce o kaydedilir: anahtar seçilen sağlayıcıya yazılır ve hemen kullanılır.
    if (dirty() && !(await saveSettings())) return;
    if (await run(() => aiIpc.keySet(provider(), keyDraft()), tai.key.saved)) setKeyDraft("");
  };
  const clearKey = () => run(() => aiIpc.keyClear(provider()), tai.key.cleared);

  return (
    <div class="max-w-xl space-y-3 p-3 text-[13px]">
      <div class="flex items-center gap-2">
        <h3 class="font-semibold">{s.title}</h3>
        <div class="flex-1" />
        <Show when={props.onClose && activeHasKey(props.view)}>
          <Button variant="ghost" class="h-6 text-[12px]" onClick={() => props.onClose?.()}>
            {s.close}
          </Button>
        </Show>
      </div>

      <fieldset class="space-y-1">
        <legend class="mb-1 text-muted">{s.provider}</legend>
        <div class="flex gap-4">
          <For each={PROVIDERS}>
            {(p) => (
              <label class="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="ai-provider"
                  checked={provider() === p}
                  onChange={() => setDraft((d) => ({ ...d, provider: p }))}
                />
                {s.providers[p]}
              </label>
            )}
          </For>
        </div>
      </fieldset>

      <label class="block space-y-1">
        <span class="text-muted">{s.model}</span>
        <input
          class="h-7 w-full rounded border border-border bg-bg px-2 font-mono text-[12px]"
          list={listId}
          spellcheck={false}
          autocomplete="off"
          value={model()}
          onInput={(e) => setModel(e.currentTarget.value)}
        />
        <datalist id={listId}>
          <For each={SUGGESTIONS[provider()]}>{(m) => <option value={m} />}</For>
        </datalist>
        <span class="block text-[12px] text-muted">{s.modelHint[provider()]}</span>
      </label>

      <label class="block space-y-1">
        <span class="text-muted">{s.effort}</span>
        <select
          class="h-7 rounded border border-border bg-bg px-2"
          value={draft().effort ?? ""}
          onChange={(e) => {
            const v = e.currentTarget.value as "" | AiEffort;
            setDraft((d) => ({ ...d, effort: v === "" ? null : v }));
          }}
        >
          <For each={EFFORTS}>{(v) => <option value={v}>{s.efforts[v]}</option>}</For>
        </select>
        <span class="block text-[12px] text-muted">{s.effortHint}</span>
      </label>

      <div class="flex items-center gap-2">
        <Button variant="primary" disabled={busy() || !dirty()} onClick={() => void saveSettings()}>
          {s.save}
        </Button>
        <span class="text-[12px] text-muted">{s.changeNote}</span>
      </div>

      <div class="space-y-1 border-t border-border pt-3">
        <div class="flex items-center gap-2">
          <span class="text-muted">
            {tai.key.title} — {s.providers[provider()]}:
          </span>
          <span classList={{ "text-fg": hasKey(), "text-danger": !hasKey() }}>{hasKey() ? tai.key.stored : tai.key.missing}</span>
          <div class="flex-1" />
          <Show when={hasKey()}>
            <Button variant="ghost" class="h-6 text-[12px]" disabled={busy()} onClick={() => void clearKey()}>
              {tai.key.clear}
            </Button>
          </Show>
        </div>
        <form
          class="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void saveKey();
          }}
        >
          <input
            type="password"
            autocomplete="off"
            spellcheck={false}
            aria-label={`${tai.key.title} — ${s.providers[provider()]}`}
            placeholder={s.keyPlaceholder[provider()]}
            class="h-7 min-w-0 flex-1 rounded border border-border bg-bg px-2"
            value={keyDraft()}
            onInput={(e) => setKeyDraft(e.currentTarget.value)}
          />
          <Button type="submit" variant={hasKey() ? "secondary" : "primary"} disabled={busy() || keyDraft().trim() === ""}>
            {hasKey() ? tai.key.replace : tai.key.save}
          </Button>
        </form>
        <p class="text-[12px] text-muted">{tai.key.note}</p>
      </div>

      <Show when={provider() === "openRouter"}>
        <p class="text-[12px] text-muted">{s.privacyOpenRouter}</p>
      </Show>
      <p class="text-[12px] text-muted">{tai.privacy}</p>
      <Show when={error()}>{(e) => <p role="alert" class="text-danger">{e()}</p>}</Show>
    </div>
  );
}
