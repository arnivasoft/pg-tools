// Hata rozeti (ana paket): yalnız ikon + düğme. HoverCard'lı tam bileşen tembel parçadadır; yüklenene dek fallback aynı rozettir.
import { CircleAlert } from "lucide-solid";
import { type Component, Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { toast } from "../../app/toast";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { createLoader } from "../../ui/lazy-load";
import { BADGE_CLASS, badgeLabel, cardActionLabel, cardTitle, isGone } from "./error-badge-model";
import type { ErrorBadgeCardProps } from "./ErrorBadgeCard";

/** Yükleyici (tek tanım): `lazy` ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadErrorBadgeCard = () => import("./ErrorBadgeCard");

// Parça modül düzeyinde bir kez yüklenir (ilk rozet oluşunca başlar); yüklenene dek ya da yükleme reddedilirse
// çalışan fallback rozet kalır (ret önbelleğe alınmaz: sonraki rozet yeniden dener, bildirim ret başına bir kez).
const [Card, setCard] = createSignal<Component<ErrorBadgeCardProps> | null>(null);
const ensureCard = createLoader(
  () => loadErrorBadgeCard().then((m) => void setCard(() => m.default)),
  () => toast.show(t.app.chunkLoadFailed, "error"),
);

export function ErrorBadge(props: { error: ErrorPayload; onRetry: () => void; onRefreshParent?: () => void; onSelect?: () => void }) {
  // Düğme odağı tutarsa ağacın klavye işleyicisi (e.target === currentTarget) tuşları yok sayar.
  const focusTree = (el: HTMLElement | undefined) => el?.closest<HTMLElement>('[role="tree"]')?.focus();
  const action = (el: HTMLElement | undefined) => {
    if (isGone(props.error)) props.onRefreshParent?.();
    else props.onRetry();
    focusTree(el);
  };
  const activate = (el: HTMLElement | undefined) => {
    props.onSelect?.();
    action(el);
  };
  ensureCard();
  const fallback = () => {
    let el: HTMLButtonElement | undefined;
    return (
      <button
        type="button"
        ref={el}
        aria-label={badgeLabel()}
        class={BADGE_CLASS}
        onClick={(e) => {
          e.stopPropagation();
          activate(el);
        }}
        onDblClick={(e) => e.stopPropagation()}
      >
        <CircleAlert size={13} />
      </button>
    );
  };
  return (
    <Show when={Card()} fallback={fallback()}>
      {(C) => {
        const View = C();
        return (
        <View
          label={badgeLabel()}
          badgeClass={BADGE_CLASS}
          title={cardTitle(props.error)}
          detail={props.error.detail ?? null}
          hint={props.error.hint ?? null}
          actionLabel={cardActionLabel(props.error)}
          onActivate={activate}
          onAction={action}
        />
        );
      }}
    </Show>
  );
}
