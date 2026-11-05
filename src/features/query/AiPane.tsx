// Yapay zekâ asistanı paneli (tembel parça): sekme başına çok turlu sohbet. Üretilen SQL yalnız "Editöre ekle"
// ile editöre girer; çalıştırılmaz. Sohbet durumu modül kapsamında tutulur: panel/sekme değişince kaybolmaz,
// süren yanıt arka planda tamamlanır. Sayfa yenilenince (arka uç geçmişiyle birlikte) silinir.
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on } from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";
import { Search } from "lucide-solid";
import { copyText } from "../../app/clipboard";
import { toast } from "../../app/toast";
import type { AiMode } from "../../ipc/bindings/AiMode";
import type { AiRequest } from "../../ipc/bindings/AiRequest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { applyEvent, emptyChat, isCommitted, newAssistant, type AssistantTurn, type ChatState, type UserTurn } from "./ai-chat";
import { aiIpc } from "./ai-ipc";
import { splitBlocks } from "./ai-model";
import { tai } from "./strings-ai";

type Chat = [ChatState, SetStoreFunction<ChatState>];
const chats = new Map<string, Chat>();
const drafts = new Map<string, string>();
function chatFor(tabId: string): Chat {
  let c = chats.get(tabId);
  if (!c) {
    c = createStore(emptyChat(crypto.randomUUID()));
    chats.set(tabId, c);
  }
  return c;
}

/** Anahtar durumu bir kez sorulur (tüm sekmeler için ortak). */
const [hasKey, setHasKey] = createSignal<boolean | null>(null);
let keyAsked = false;

function userLabel(t: UserTurn): string {
  if (t.mode === "explain") return t.text ? `${tai.explain}: ${t.text}` : tai.explain;
  if (t.mode === "fix") return t.text ? `${tai.fix}: ${t.text}` : tai.fix;
  return t.text;
}

export default function AiPane(props: {
  tabId: string;
  /** Seçili SQL, son hata, şema özeti ve sürüm; eksik bağlam için `Error` (Türkçe ileti) fırlatır. */
  context: (mode: AiMode, prompt: string) => Promise<AiRequest>;
  onInsert: (sql: string) => void;
  canInsert: boolean;
  /** Sekmenin veritabanı (katalog araçları için); sekme bağlı değilse alanlar `null`. */
  target: { serverId: string | null; databaseOid: number | null };
}) {
  // `tabId` bileşen ömrü boyunca sabittir (QueryTab sekme başına yeniden oluşturur).
  const [chat, setChat] = chatFor(props.tabId);
  const [keyDraft, setKeyDraft] = createSignal("");
  const [keyError, setKeyError] = createSignal<ErrorPayload | null>(null);
  const [draft, setDraftSignal] = createSignal(drafts.get(props.tabId) ?? "");
  const setDraft = (v: string) => {
    drafts.set(props.tabId, v);
    setDraftSignal(v);
  };
  /** Bağlam toplanamadı (ör. düzeltilecek hata yok): sohbete girmeyen geçici ileti. */
  const [notice, setNotice] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal<string | null>(null);
  const busy = () => chat.status !== "idle";
  let scroller: HTMLDivElement | undefined;

  if (!keyAsked) {
    keyAsked = true;
    aiIpc.keyStatus().then(setHasKey, (e) => {
      keyAsked = false;
      setKeyError(toPayload(e));
      setHasKey(false);
    });
  }

  // Yeni metin gelince, kullanıcı en alttaysa aşağı kaydır (yukarı kaydırıp okurken zıplatma).
  const lastLen = () => {
    const last = chat.turns[chat.turns.length - 1];
    return (chat.turns.length << 20) + (last?.role === "assistant" ? last.text.length : 0);
  };
  createEffect(
    on(lastLen, () => {
      const el = scroller;
      if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 80) el.scrollTop = el.scrollHeight;
    }),
  );

  const saveKey = async () => {
    try {
      await aiIpc.keySet(keyDraft());
      setKeyDraft("");
      setHasKey(true);
      setKeyError(null);
      toast.show(tai.key.saved, "success");
    } catch (e) {
      setKeyError(toPayload(e));
    }
  };
  const clearKey = async () => {
    try {
      await aiIpc.keyClear();
      setHasKey(false);
      toast.show(tai.key.cleared, "info");
    } catch (e) {
      setKeyError(toPayload(e));
    }
  };

  const ask = async (mode: AiMode) => {
    if (busy()) return;
    const id = crypto.randomUUID();
    const prompt = draft();
    setNotice(null);
    setChat({ requestId: id, status: "gathering" });
    let request: AiRequest;
    try {
      request = await props.context(mode, prompt);
    } catch (e) {
      setChat({ requestId: null, status: "idle" });
      setNotice(toPayload(e).message);
      return;
    }
    if (chat.requestId !== id) return;
    setDraft("");
    const user: UserTurn = { role: "user", mode, text: prompt.trim(), sql: mode === "generate" ? "" : request.sql };
    setChat("turns", (t) => [...t, user, newAssistant()]);
    const idx = chat.turns.length - 1;
    setChat("status", "streaming");
    const patch = (f: (t: AssistantTurn) => AssistantTurn) => setChat("turns", idx, (t) => f(t as AssistantTurn));
    try {
      await aiIpc.ask(id, chat.conversation, chat.committed, request, props.target, (ev) => patch((t) => applyEvent(t, ev)));
    } catch (e) {
      const p = toPayload(e);
      if (p.code === "ai_no_key") setHasKey(false);
      patch((t) => ({ ...t, error: p }));
    } finally {
      if (isCommitted(chat.turns[idx] as AssistantTurn)) {
        patch((t) => ({ ...t, committed: true }));
        setChat("committed", (n) => n + 1);
      }
      setChat({ requestId: null, status: "idle" });
    }
  };

  /** Akışı keser; durum, istek kapanınca (`finally`) boşa döner. */
  const stop = () => {
    const id = chat.requestId;
    if (id) void aiIpc.cancel(id);
  };

  const newChat = () => {
    if (busy()) return;
    void aiIpc.forget(chat.conversation);
    setChat(emptyChat(crypto.randomUUID()));
    setNotice(null);
  };

  const copy = async (key: string, text: string) => {
    await copyText(text);
    setCopied(key);
    setTimeout(() => setCopied((c) => (c === key ? null : c)), 1500);
  };

  const AssistantView = (p: { turn: AssistantTurn; index: number; last: boolean }) => {
    const blocks = createMemo(() => splitBlocks(p.turn.text));
    const streaming = () => p.last && chat.status === "streaming";
    return (
      <div class="space-y-2">
        <Show when={p.turn.tools.length > 0}>
          <ul class="space-y-0.5 text-[12px] text-muted" aria-label={tai.toolsAria}>
            <For each={p.turn.tools}>
              {(s) => (
                <li class="flex items-center gap-1.5">
                  <Search size={12} aria-hidden="true" />
                  <span>
                    {tai.tool[s.name] ?? s.name}
                    <Show when={s.detail}>
                      : <span class="font-mono">{s.detail}</span>
                    </Show>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <Show when={streaming() && p.turn.text === ""}>
          <p class="text-muted">{tai.thinking}</p>
        </Show>
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
                    <Button variant="ghost" class="h-6 text-[12px]" onClick={() => void copy(`${p.index}:${i()}`, b.text)}>
                      {copied() === `${p.index}:${i()}` ? tai.copied : tai.copy}
                    </Button>
                  </div>
                </Show>
              </div>
            )
          }
        </For>
        <Show when={p.turn.error}>
          {(e) => (
            <div role="alert" class="text-danger">
              {e().message}
              <Show when={e().detail}>{(d) => <div class="mt-0.5 font-mono text-[12px] text-muted">{d()}</div>}</Show>
            </div>
          )}
        </Show>
        <Show when={!streaming()}>
          <p class="text-[12px] text-muted">
            <Show when={p.turn.fallback}>{tai.fallbackNote} </Show>
            <Show when={p.turn.finish?.stopReason === "max_tokens"}>{tai.maxTokens} </Show>
            <Show when={p.turn.finish?.stopReason === "refusal"}>{tai.refused} </Show>
            <Show when={!p.turn.committed}>{tai.notCommitted} </Show>
            <Show when={p.turn.finish}>{(f) => tai.usage(f().inputTokens, f().cacheReadTokens, f().outputTokens)}</Show>
          </p>
        </Show>
      </div>
    );
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
            <Show when={keyError()}>{(e) => <p class="text-danger">{e().message}</p>}</Show>
          </div>
        </Match>
        <Match when={hasKey() === true}>
          <div ref={scroller} class="min-h-0 flex-1 overflow-auto p-3" aria-live="polite" aria-busy={busy()}>
            <Show when={chat.turns.length === 0}>
              <p class="max-w-xl text-muted">{tai.empty}</p>
              <p class="mt-2 max-w-xl text-[12px] text-muted">{tai.privacy}</p>
            </Show>
            <ol class="max-w-3xl space-y-4" aria-label={tai.historyAria}>
              <For each={chat.turns}>
                {(turn, i) =>
                  turn.role === "user" ? (
                    <li class="rounded bg-hover px-2 py-1.5" aria-label={tai.youAria}>
                      <p class="whitespace-pre-wrap font-medium">{userLabel(turn)}</p>
                      <Show when={turn.sql}>
                        <details class="mt-1">
                          <summary class="cursor-pointer text-[12px] text-muted">{tai.sentSql}</summary>
                          <pre class="mt-1 max-h-40 overflow-auto font-mono text-[12px]">{turn.sql}</pre>
                        </details>
                      </Show>
                    </li>
                  ) : (
                    <li aria-label={tai.assistantAria}>
                      <AssistantView turn={turn} index={i()} last={i() === chat.turns.length - 1} />
                    </li>
                  )
                }
              </For>
            </ol>
            <Show when={chat.status === "gathering"}>
              <p class="mt-3 text-muted">{tai.gathering}</p>
            </Show>
          </div>
          <div class="flex shrink-0 flex-col gap-2 border-t border-border p-2">
            <Show when={notice()}>{(n) => <p class="text-danger">{n()}</p>}</Show>
            <label class="sr-only" for={`ai-prompt-${props.tabId}`}>
              {tai.promptLabel}
            </label>
            <textarea
              id={`ai-prompt-${props.tabId}`}
              rows={2}
              class="min-h-[2.5rem] w-full resize-y rounded border border-border bg-bg px-2 py-1"
              placeholder={chat.committed > 0 ? tai.followUpPlaceholder : tai.promptPlaceholder}
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  if (draft().trim() !== "") void ask("generate");
                }
              }}
            />
            <div class="flex flex-wrap items-center gap-1.5">
              <Button variant="primary" disabled={busy() || draft().trim() === ""} title="Ctrl+Enter" onClick={() => void ask("generate")}>
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
              <Button variant="ghost" class="h-6 text-[12px]" disabled={busy() || chat.turns.length === 0} onClick={newChat}>
                {tai.newChat}
              </Button>
              <Button variant="ghost" class="h-6 text-[12px] text-muted" disabled={busy()} onClick={() => void clearKey()}>
                {tai.key.clear}
              </Button>
            </div>
          </div>
        </Match>
      </Switch>
    </section>
  );
}
