// Parola istemi (ana paket): yalnız model + tembel görünüm. `Dialog`/Kobalte burada statik içe aktarılmaz.
import { Show, Suspense, lazy } from "solid-js";
import { t } from "../../app/strings";
import { toast } from "../../app/toast";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { createLoader } from "../../ui/lazy-load";
import { createPasswordPromptModel } from "./password-prompt-model";

/** Görünüm yükleyicisi (tek tanım): `lazy` ve ileride önyükleme aynı içe aktarmayı paylaşır. */
export const loadPasswordPromptView = () => import("./PasswordPromptView");
const PasswordPromptView = lazy(loadPasswordPromptView);

export function createPasswordPrompt() {
  const m = createPasswordPromptModel(
    createLoader(loadPasswordPromptView, () => toast.show(t.app.chunkLoadFailed, "error")),
  );
  const ask = (node: TreeNode, retry: boolean, error: ErrorPayload | null = null) => m.ask(node, retry, error);
  const View = () => (
    <Show when={m.loaded()}>
      <Suspense>
        <PasswordPromptView
          request={m.request()}
          password={m.password()}
          remember={m.remember()}
          onPassword={m.setPassword}
          onRemember={m.setRemember}
          finish={m.finish}
        />
      </Suspense>
    </Show>
  );
  return { ask, View };
}
