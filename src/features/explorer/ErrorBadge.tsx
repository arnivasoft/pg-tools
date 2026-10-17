// Hata rozeti (ana paket): yalnız ikon + düğme. HoverCard'lı tam bileşen tembel parçadadır; yüklenene dek fallback aynı rozettir.
import { CircleAlert } from "lucide-solid";
import { type Component, Show, createSignal } from "solid-js";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { BADGE_CLASS, badgeLabel, createBadgeCardLoader, cardActionLabel, cardTitle, isGone } from "./error-badge-model";
import type { ErrorBadgeCardProps } from "./ErrorBadgeCard";

/** Yükleyici (tek tanım): `lazy` ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadErrorBadgeCard = () => import("./ErrorBadgeCard");

// Parça modül düzeyinde bir kez yüklenir (ilk rozet oluşunca başlar); yüklenene dek ya da yükleme reddedilirse
// çalışan fallback rozet kalır. Ret sessizdir (kart süstür); yeniden deneme yalnız rozete gelme/odaklanma ile olur.
const [Card, setCard] = createSignal<Component<ErrorBadgeCardProps> | null>(null);
const cardLoader = createBadgeCardLoader(() => loadErrorBadgeCard().then((m) => void setCard(() => m.default)));

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
  cardLoader.start();
  let el: HTMLButtonElement | undefined;
  return (
    <Show when={Card()} fallback={
        // JSX prop'u Solid'de getter'a derlenir: düğme yalnız fallback gerektiğinde (tembel) oluşur.
        <button
          type="button"
          ref={el}
          aria-label={badgeLabel()}
          class={BADGE_CLASS}
          onClick={(e) => {
            e.stopPropagation();
            activate(el);
          }}
          onPointerEnter={cardLoader.retry}
          onFocus={cardLoader.retry}
          onDblClick={(e) => e.stopPropagation()}
        >
          <CircleAlert size={13} />
        </button>
      }>
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
