// Işlem kapatma / toplu işlem / ortam diyalogları (Kobalte Dialog çeker): ana pakette statik içe aktarılmaz,
// ilk gerekli olduğunda `loadTxDialogs` ile tembel iner.
import { For, Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import type { QueryController } from "./query-controller";
import { ENV_CHOICES, canCommitAny, envLabel, envLockDefault, type EnvChoice } from "./workspace-model";

export function CloseTxDialog(props: { ctl: QueryController }) {
  const g = props.ctl.closeGate;
  const c = t.query.close;
  let cancelEl: HTMLButtonElement | undefined;
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && g.answer("cancel")}
      title={c.title}
      width="420px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => g.answer("cancel")}>
            {t.server.cancel}
          </Button>
          <Show when={g.request()?.choices.includes("rollback")}>
            <Button class="!bg-danger !text-white" variant="primary" onClick={() => g.answer("rollback")}>
              {c.rollback}
            </Button>
          </Show>
          <Show when={g.request()?.choices.includes("commit")}>
            <Button variant="primary" onClick={() => g.answer("commit")}>
              {c.commit}
            </Button>
          </Show>
        </>
      }
    >
      <p>{g.request()?.failed ? c.failedMessage(g.request()?.title ?? "") : c.message(g.request()?.title ?? "")}</p>
      <Show when={g.request()?.busy}>
        <p class="mt-2 text-xs text-muted">{c.busyNote}</p>
      </Show>
    </Dialog>
  );
}

export function BulkTxDialog(props: { ctl: QueryController }) {
  const g = props.ctl.bulkGate;
  const b = t.query.bulk;
  let cancelEl: HTMLButtonElement | undefined;
  const list = () => g.request()?.list ?? [];
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && g.answer("cancel")}
      title={b.title}
      width="460px"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        cancelEl?.focus();
      }}
      footer={
        <>
          <Button ref={cancelEl} onClick={() => g.answer("cancel")}>
            {t.server.cancel}
          </Button>
          <Button class="!bg-danger !text-white" variant="primary" onClick={() => g.answer("rollback")}>
            {b.rollbackAll}
          </Button>
          <Show when={canCommitAny(list())}>
            <Button variant="primary" onClick={() => g.answer("commit")}>
              {b.commitAll}
            </Button>
          </Show>
        </>
      }
    >
      <p class="mb-2">{g.request()?.message}</p>
      <ul class="mb-2 flex max-h-48 flex-col gap-1 overflow-auto">
        <For each={list()}>
          {(p) => (
            <li class="flex items-center gap-2 rounded border border-border px-2 py-1 text-[13px]">
              <span class="min-w-0 flex-1 truncate">{p.title}</span>
              <span classList={{ "text-danger": p.tx === "failed", "text-muted": p.tx !== "failed" }}>
                {b.tx[p.tx]}
                {p.busy ? ` · ${b.busy}` : ""}
              </span>
            </li>
          )}
        </For>
      </ul>
      <Show when={list().some((p) => p.tx === "failed" || p.busy)}>
        <p class="text-xs text-muted">{b.failedNote}</p>
      </Show>
    </Dialog>
  );
}

export function EnvDialog(props: { ctl: QueryController }) {
  const g = props.ctl.envGate;
  const e = t.query.env;
  const [choice, setChoice] = createSignal<EnvChoice | null>(null);
  const [lock, setLock] = createSignal(false);
  const config = () => g.request()?.config ?? null;
  const pick = (c: EnvChoice) => {
    setChoice(c);
    const cfg = config();
    if (cfg) setLock(envLockDefault(cfg, c));
  };
  const close = (answer: Parameters<typeof g.answer>[0]) => {
    g.answer(answer);
    setChoice(null);
    setLock(false);
  };
  return (
    <Dialog
      open={g.request() !== null}
      onOpenChange={(open) => !open && close(null)}
      title={e.title}
      width="400px"
      footer={
        <>
          <Button class="mr-auto" onClick={() => close("skip")}>
            {e.skip}
          </Button>
          <Button
            variant="primary"
            disabled={choice() === null}
            onClick={() => {
              const c = choice();
              if (c) close({ choice: c, lock: lock() });
            }}
          >
            {e.save}
          </Button>
        </>
      }
    >
      <p class="mb-3">{e.message(config()?.name ?? "")}</p>
      <div role="radiogroup" aria-label={e.title} class="flex flex-col gap-1.5">
        <For each={ENV_CHOICES}>
          {(c) => (
            <label class="flex items-center gap-2 text-[13px]">
              <input type="radio" name="pgt-env" checked={choice() === c} onChange={() => pick(c)} />
              {envLabel(c)}
            </label>
          )}
        </For>
      </div>
      <Show when={choice() === "production"}>
        <label class="mt-3 flex items-center gap-2 text-[13px]">
          <input type="checkbox" checked={lock()} onChange={(ev) => setLock(ev.currentTarget.checked)} />
          {e.lock}
        </label>
      </Show>
    </Dialog>
  );
}
