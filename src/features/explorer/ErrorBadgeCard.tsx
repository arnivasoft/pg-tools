// HoverCard'lı tam rozet (Kobalte HoverCard'ı çeker): ana pakette statik içe aktarılmaz, `loadErrorBadgeCard` ile tembel iner.
// Metinler ve eylemler ana paketten prop olarak gelir (ana paket modülleri bu parçaya taşınmasın diye).
import { HoverCard } from "@kobalte/core/hover-card";
import { CircleAlert } from "lucide-solid";
import { Show } from "solid-js";

export type ErrorBadgeCardProps = {
  label: string;
  badgeClass: string;
  title: string;
  detail: string | null;
  hint: string | null;
  actionLabel: string;
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
          <p class="font-semibold">{props.title}</p>
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
            {props.actionLabel}
          </button>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard>
  );
}
