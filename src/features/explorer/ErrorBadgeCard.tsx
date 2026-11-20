// HoverCard'lı tam rozet (Kobalte HoverCard'ı çeker): ana pakette statik içe aktarılmaz, `loadErrorBadgeCard` ile tembel iner.
// Eylemler ana paketten prop olarak gelir; başlık/eylem metinleri bu parçadadır (error-badge-card-text.ts; Plan 37 telafisi) (ana paket modülleri bu parçaya taşınmasın diye).
import { HoverCard } from "@kobalte/core/hover-card";
import { CircleAlert } from "lucide-solid";
import { Show } from "solid-js";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { cardActionLabel, cardTitle } from "./error-badge-card-text";

export type ErrorBadgeCardProps = {
  label: string;
  badgeClass: string;
  error: ErrorPayload;
  detail: string | null;
  hint: string | null;
  onActivate: (trigger: HTMLElement | undefined) => void;
  onAction: (trigger: HTMLElement | undefined) => void;
};

export default function ErrorBadgeCard(props: ErrorBadgeCardProps) {
  let trigger: HTMLElement | undefined;
  return (
    <HoverCard openDelay={150} closeDelay={100} placement="bottom-end">
      <HoverCard.Trigger
        as="button"
        type="button"
        ref={(el: HTMLButtonElement) => { trigger = el; }}
        aria-label={props.label}
        class={props.badgeClass}
        onClick={(e: MouseEvent) => {
          e.stopPropagation();
          props.onActivate(trigger);
        }}
        onDblClick={(e: MouseEvent) => e.stopPropagation()}
      >
        <CircleAlert size={13} />
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content class="z-50 max-w-[360px] rounded-md border border-border bg-bg p-3 text-sm shadow-xl">
          <p class="font-semibold">{cardTitle(props.error)}</p>
          <Show when={props.detail}>
            <p class="mt-1 select-text break-words">{props.detail}</p>
          </Show>
          <Show when={props.hint}>
            <p class="mt-1 break-words text-muted">{props.hint}</p>
          </Show>
          <button
            type="button"
            class="mt-2 rounded border border-border px-2 py-0.5 hover:bg-hover"
            onClick={() => props.onAction(trigger)}
          >
            {cardActionLabel(props.error)}
          </button>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard>
  );
}
