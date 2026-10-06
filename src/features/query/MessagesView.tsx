import { For, Show } from "solid-js";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { hintFor } from "./errors";
import type { Message } from "./result-model";

const TONE: Record<Message["level"], string> = {
  notice: "text-muted",
  info: "text-accent",
  error: "text-danger",
  cancelled: "text-muted",
};

function Detail(props: { error: Pick<ErrorPayload, "detail" | "hint">; position?: number | null; extraHint?: string | null }) {
  const m = t.query.messages;
  return (
    <div class="mt-0.5 flex flex-col gap-0.5 pl-3 text-muted">
      <Show when={props.position != null}>
        <div>{m.position(props.position!)}</div>
      </Show>
      <Show when={props.error.detail}>
        <div>{m.detail}: {props.error.detail}</div>
      </Show>
      <Show when={props.error.hint}>
        <div>{m.hint}: {props.error.hint}</div>
      </Show>
      <Show when={props.extraHint}>
        <div class="text-fg">{m.hint}: {props.extraHint}</div>
      </Show>
    </div>
  );
}

/** NOTICE/WARNING, hata ayrıntıları (`detail`/`hint`/konum, `hintFor` ipucu) ve komut etiketleri. */
export function MessagesView(props: { messages: Message[] }) {
  const m = t.query.messages;
  return (
    <div role="region" aria-label={m.ariaLabel} class="h-full overflow-auto p-2 font-mono text-xs">
      <Show when={props.messages.length > 0} fallback={<p class="text-muted">{m.empty}</p>}>
        <For each={props.messages}>
          {(msg) => (
            <div class="mb-1.5">
              {msg.level === "notice" ? (
                <>
                  <div class={TONE.notice}>
                    [{m.statement(msg.stmtIndex + 1)}] {msg.notice.severity}: {msg.notice.message}
                  </div>
                  <Detail error={msg.notice} />
                </>
              ) : msg.level === "info" ? (
                <div class={TONE.info}>
                  [{m.statement(msg.stmtIndex + 1)}] {msg.tag} — {t.query.strip.ms(msg.elapsedMs)}
                </div>
              ) : (
                <>
                  <div class={TONE[msg.level]}>
                    [{m.statement(msg.stmtIndex + 1)}] {m.level[msg.level]}: {msg.error.message}
                  </div>
                  <Detail error={msg.error} position={msg.error.position} extraHint={hintFor(msg.error)} />
                </>
              )}
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}
