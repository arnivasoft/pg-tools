// Yapay zekâ asistanı paneli (tembel parça). Üretilen SQL yalnız "Editöre ekle" ile editöre girer; çalıştırılmaz.
import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { copyText } from "../../app/clipboard";
import { toast } from "../../app/toast";
import type { AiMode } from "../../ipc/bindings/AiMode";
import type { AiRequest } from "../../ipc/bindings/AiRequest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { aiIpc } from "./ai-ipc";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { splitBlocks } from "./ai-model";
import { tai } from "./strings-ai";

type Status = "idle" | "gathering" | "streaming";
type Finish = { stopReason: string; inputTokens: number; outputTokens: number };

export default function AiPane(props: {
  /** Seçili SQL, son hata, şema özeti ve sürüm; eksik bağlam için `Error` (Türkçe ileti) fırlatır. */
  context: (mode: AiMode, prompt: string) => Promise<AiRequest>;
  onInsert: (sql: string) => void;
  canInsert: boolean;
}) {
  const [hasKey, setHasKey] = createSignal<boolean | null>(null);
  const [keyDraft, setKeyDraft] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [answer, setAnswer] = createSignal("");
  const [status, setStatus] = createSignal<Status>("idle");
  const [finish, setFinish] = createSignal<Finish | null>(null);
  const [error, setError] = createSignal<ErrorPayload | null>(null);
  const [copied, setCopied] = createSignal<number | null>(null);
  let current: string | null = null;

  const blocks = createMemo(() => splitBlocks(answer()));
  const busy = () => status() !== "idle";

  onMount(async () => {
    try {
      setHasKey(await aiIpc.keyStatus());
    } catch (e) {
      setError(toPayload(e));
      setHasKey(false);
    }
  });
  onCleanup(() => {
    if (current) void aiIpc.cancel(current);
  });

  const saveKey = async () => {
    try {
      await aiIpc.keySet(keyDraft());
      setKeyDraft("");
      setHasKey(true);
      setError(null);
      toast.show(tai.key.saved, "success");
    } catch (e) {
      setError(toPayload(e));
    }
  };
  const clearKey = async () => {
    try {
      await aiIpc.keyClear();
      setHasKey(false);
      toast.show(tai.key.cleared, "info");
    } catch (e) {
      setError(toPayload(e));
    }
  };

  const ask = async (mode: AiMode) => {
    if (busy()) return;
    const id = crypto.randomUUID();
    current = id;
    setError(null);
    setFinish(null);
    setAnswer("");
    setStatus("gathering");
    try {
      const request = await props.context(mode, prompt());
      if (current !== id) return;
      setStatus("streaming");
      await aiIpc.ask(id, request, (ev) => {
        if (current !== id) return;
        if (ev.type === "delta") setAnswer((a) => a + ev.text);
        else if (ev.type === "reset") setAnswer("");
        else setFinish({ stopReason: ev.stopReason, inputTokens: ev.inputTokens, outputTokens: ev.outputTokens });
      });
    } catch (e) {
      if (current !== id) return;
      const p = toPayload(e);
      if (p.code === "ai_no_key") setHasKey(false);
      setError(p);
    } finally {
      if (current === id) {
        current = null;
        setStatus("idle");
      }
    }
  };

  const stop = () => {
    const id = current;
    if (!id) return;
    current = null;
    setStatus("idle");
    void aiIpc.cancel(id);
  };

  const copy = async (i: number, text: string) => {
    await copyText(text);
    setCopied(i);
    setTimeout(() => setCopied((c) => (c === i ? null : c)), 1500);
  };

  return (
    <section aria-label={tai.ariaLabel} class="flex h-full min-h-0 flex-col text-[13px]">
      <Switch>
        <Match when={hasKey() === false}>
          <div class="max-w-xl space-y-2 p-3">
            <h3 class="font-semibold">{tai.key.title}</h3>
            <p class="text-muted">{tai.key.intro}</p>
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
                aria-label={tai.key.title}
                placeholder={tai.key.placeholder}
                class="h-7 min-w-0 flex-1 rounded border border-border bg-bg px-2"
                value={keyDraft()}
                onInput={(e) => setKeyDraft(e.currentTarget.value)}
              />
              <Button type="submit" variant="primary" disabled={keyDraft().trim() === ""}>
                {tai.key.save}
              </Button>
            </form>
            <p class="text-[12px] text-muted">{tai.privacy}</p>
            <Show when={error()}>{(e) => <p class="text-danger">{e().message}</p>}</Show>
          </div>
        </Match>
        <Match when={hasKey() === true}>
          <div class="flex shrink-0 flex-col gap-2 border-b border-border p-2">
            <label class="sr-only" for="ai-prompt">
              {tai.promptLabel}
            </label>
            <textarea
              id="ai-prompt"
              rows={2}
              class="min-h-[2.5rem] w-full resize-y rounded border border-border bg-bg px-2 py-1"
              placeholder={tai.promptPlaceholder}
              value={prompt()}
              onInput={(e) => setPrompt(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  void ask("generate");
                }
              }}
            />
            <div class="flex flex-wrap items-center gap-1.5">
              <Button variant="primary" disabled={busy() || prompt().trim() === ""} title="Ctrl+Enter" onClick={() => void ask("generate")}>
                {tai.generate}
              </Button>
              <Button disabled={busy()} onClick={() => void ask("explain")}>
                {tai.explain}
              </Button>
              <Button disabled={busy()} onClick={() => void ask("fix")}>
                {tai.fix}
              </Button>
              <Show when={busy()}>
                <Button variant="ghost" onClick={stop}>
                  {tai.cancel}
                </Button>
              </Show>
              <div class="flex-1" />
              <Button variant="ghost" class="h-6 text-[12px] text-muted" disabled={busy()} onClick={() => void clearKey()}>
                {tai.key.clear}
              </Button>
            </div>
          </div>
          <div class="min-h-0 flex-1 overflow-auto p-3" aria-live="polite" aria-busy={busy()}>
            <Show when={error()}>
              {(e) => (
                <div role="alert" class="mb-2 text-danger">
                  {e().message}
                  <Show when={e().detail}>{(d) => <div class="mt-0.5 font-mono text-[12px] text-muted">{d()}</div>}</Show>
                </div>
              )}
            </Show>
            <Show when={status() === "gathering"}>
              <p class="text-muted">{tai.gathering}</p>
            </Show>
            <Show when={status() === "streaming" && answer() === ""}>
              <p class="text-muted">{tai.thinking}</p>
            </Show>
            <Show when={status() === "idle" && answer() === "" && !error()}>
              <p class="max-w-xl text-muted">{tai.empty}</p>
              <p class="mt-2 max-w-xl text-[12px] text-muted">{tai.privacy}</p>
            </Show>
            <div class="max-w-3xl space-y-2">
              <For each={blocks()}>
                {(b, i) =>
                  b.kind === "text" ? (
                    <p class="whitespace-pre-wrap">{b.text}</p>
                  ) : (
                    <div class="rounded border border-border bg-panel">
                      <pre class="overflow-x-auto p-2 font-mono text-[12px]">{b.text}</pre>
                      <Show when={!b.open}>
                        <div class="flex gap-1 border-t border-border p-1">
                          <Show when={b.lang === "" || b.lang === "sql" || b.lang === "postgresql"}>
                            <Button variant="primary" class="h-6 text-[12px]" disabled={!props.canInsert} onClick={() => props.onInsert(b.text)}>
                              {tai.insert}
                            </Button>
                          </Show>
                          <Button variant="ghost" class="h-6 text-[12px]" onClick={() => void copy(i(), b.text)}>
                            {copied() === i() ? tai.copied : tai.copy}
                          </Button>
                        </div>
                      </Show>
                    </div>
                  )
                }
              </For>
            </div>
            <Show when={finish()}>
              {(f) => (
                <p class="mt-3 text-[12px] text-muted">
                  <Show when={f().stopReason === "max_tokens"}>{tai.maxTokens} </Show>
                  <Show when={f().stopReason === "refusal"}>{tai.refused} </Show>
                  {tai.usage(f().inputTokens, f().outputTokens)}
                </p>
              )}
            </Show>
          </div>
        </Match>
      </Switch>
    </section>
  );
}
