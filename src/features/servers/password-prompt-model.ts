// Parola istemi modeli (Solid bileşeni yok → birim test edilebilir): söz tabanlı istek + tembel görünüm yükleme.
import { createSignal } from "solid-js";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { PasswordAnswer } from "../explorer/controller";

export type PasswordRequest = {
  node: TreeNode;
  retry: boolean;
  error: ErrorPayload | null;
  resolve: (answer: PasswordAnswer | null) => void;
};

/** `ensure`: görünüm parçasını yükler (başarıda true). Hata → istek iptal (`null`) ile çözülür, diyalog açılmaz. */
export function createPasswordPromptModel(ensure: () => Promise<boolean>) {
  const [request, setRequest] = createSignal<PasswordRequest | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [password, setPassword] = createSignal("");
  const [remember, setRemember] = createSignal(false);

  const ask = async (node: TreeNode, retry: boolean, error: ErrorPayload | null = null) => {
    if (!(await ensure())) return null;
    return new Promise<PasswordAnswer | null>((resolve) => {
      request()?.resolve(null);
      setPassword("");
      setRemember(false);
      setLoaded(true);
      setRequest({ node, retry, error, resolve });
    });
  };

  const finish = (answer: PasswordAnswer | null) => {
    const current = request();
    if (!current) return;
    setRequest(null);
    setPassword("");
    current.resolve(answer);
  };

  return { request, loaded, password, setPassword, remember, setRemember, ask, finish };
}
