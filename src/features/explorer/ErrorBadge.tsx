import { HoverCard } from "@kobalte/core/hover-card";
import { CircleAlert } from "lucide-solid";
import { Show } from "solid-js";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";

export function ErrorBadge(props: { error: ErrorPayload; onRetry: () => void; onRefreshParent?: () => void; onSelect?: () => void }) {
  const gone = () => props.error.code === "not_found";
  return (
    <HoverCard openDelay={150} closeDelay={100} placement="bottom-end">
      <HoverCard.Trigger
        as="button"
        type="button"
        aria-label={t.error.badge}
        class="ml-auto shrink-0 rounded text-danger outline-none focus-visible:ring-1 focus-visible:ring-accent"
        onClick={(e: MouseEvent) => {
          e.stopPropagation();
          props.onSelect?.();
          if (gone()) props.onRefreshParent?.();
          else props.onRetry();
        }}
        onDblClick={(e: MouseEvent) => e.stopPropagation()}
      >
        <CircleAlert size={13} />
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content class="z-50 max-w-[360px] rounded-md border border-border bg-bg p-3 text-sm shadow-xl">
          <p class="font-semibold">{gone() ? t.error.gone : props.error.message}</p>
          <Show when={props.error.detail}>
            <p class="mt-1 select-text break-words">{props.error.detail}</p>
          </Show>
          <Show when={props.error.hint}>
            <p class="mt-1 break-words text-muted">{props.error.hint}</p>
          </Show>
          <button
            type="button"
            class="mt-2 rounded border border-border px-2 py-0.5 hover:bg-hover"
            onClick={() => (gone() ? props.onRefreshParent?.() : props.onRetry())}
          >
            {gone() ? t.error.refreshParent : t.error.retry}
          </button>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard>
  );
}
