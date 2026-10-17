// Hata rozeti (ana paket): yalnız ikon + düğme. HoverCard'lı tam bileşen tembel parçadadır; yüklenene dek fallback aynı rozettir.
import { CircleAlert } from "lucide-solid";
import { Suspense, lazy } from "solid-js";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { BADGE_CLASS, badgeLabel, cardActionLabel, cardTitle, isGone } from "./error-badge-model";

/** Yükleyici (tek tanım): `lazy` ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadErrorBadgeCard = () => import("./ErrorBadgeCard");
const ErrorBadgeCard = lazy(loadErrorBadgeCard);

export function ErrorBadge(props: { error: ErrorPayload; onRetry: () => void; onRefreshParent?: () => void; onSelect?: () => void }) {
  // Düğme odağı tutarsa ağacın klavye işleyicisi (e.target === currentTarget) tuşları yok sayar.
  const focusTree = (el: HTMLElement | undefined) => el?.closest<HTMLElement>('[role="tree"]')?.focus();
  const run = () => {
    if (isGone(props.error)) props.onRefreshParent?.();
    else props.onRetry();
  };
  const activate = (el: HTMLElement | undefined) => {
    props.onSelect?.();
    run();
    focusTree(el);
  };
  const action = (el: HTMLElement | undefined) => {
    run();
    focusTree(el);
  };
  const fallback = () => {
    let el: HTMLButtonElement | undefined;
    return (
      <button
        type="button"
        ref={el}
        aria-label={badgeLabel(props.error)}
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
    <Suspense fallback={fallback()}>
      <ErrorBadgeCard
        label={badgeLabel(props.error)}
        badgeClass={BADGE_CLASS}
        title={cardTitle(props.error)}
        detail={props.error.detail ?? null}
        hint={props.error.hint ?? null}
        actionLabel={cardActionLabel(props.error)}
        onActivate={activate}
        onAction={action}
      />
    </Suspense>
  );
}
