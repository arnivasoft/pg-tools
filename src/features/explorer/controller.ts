import type { Api } from "../../ipc/commands";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { toPayload } from "../../ipc/errors";
import {
  ROOT, collapse, expand, flatten, keyIntent, patchNode, select, setChildren, setError, startLoading, stopLoading,
  type Intent, type TreeState,
} from "./tree-model";

export type PasswordAnswer = { password: string; remember: boolean };

export interface ExplorerDeps {
  api: Pick<Api, "listChildren" | "connectServer">;
  requestPassword(node: TreeNode, retry: boolean): Promise<PasswordAnswer | null>;
}

export class ExplorerController {
  constructor(
    private readonly get: () => TreeState,
    private readonly set: (s: TreeState) => void,
    private readonly deps: ExplorerDeps,
  ) {}

  private update(f: (s: TreeState) => TreeState): void {
    this.set(f(this.get()));
  }

  private readonly connecting = new Map<string, Promise<boolean>>();
  private readonly loads = new Map<string, Promise<void>>();

  load(id: string): Promise<void> {
    const pending = this.loads.get(id);
    if (pending) return pending;
    const p = this.doLoad(id).finally(() => this.loads.delete(id));
    this.loads.set(id, p);
    return p;
  }

  private async doLoad(id: string): Promise<void> {
    this.update((s) => startLoading(s, id));
    try {
      const list = await this.deps.api.listChildren(id);
      this.update((s) => setChildren(s, id, list));
    } catch (e) {
      this.update((s) => setError(s, id, toPayload(e)));
    }
  }

  ensureConnected(id: string): Promise<boolean> {
    const pending = this.connecting.get(id);
    if (pending) return pending;
    const p = this.doConnect(id).finally(() => this.connecting.delete(id));
    this.connecting.set(id, p);
    return p;
  }

  private async doConnect(id: string): Promise<boolean> {
    const first = this.get().nodes.get(id);
    if (!first || first.kind !== "server" || first.flags.connected || !first.serverId) return true;
    const serverId = first.serverId;
    let password: string | null = null;
    let remember = false;
    for (;;) {
      this.update((s) => startLoading(s, id));
      try {
        const info = await this.deps.api.connectServer(serverId, password, remember);
        const cur = this.get().nodes.get(id);
        if (!cur) {
          this.update((s) => stopLoading(s, id));
          return false;
        }
        this.update((s) => {
          const n = s.nodes.get(id);
          return n ? patchNode(stopLoading(s, id), id, { detail: info.version, flags: { ...n.flags, connected: true } }) : s;
        });
        return true;
      } catch (e) {
        const error = toPayload(e);
        if (error.code === "password_required" || error.code === "auth") {
          this.update((s) => stopLoading(s, id));
          const cur = this.get().nodes.get(id);
          if (!cur) return false;
          let answer: PasswordAnswer | null;
          try {
            answer = await this.deps.requestPassword(cur, error.code === "auth");
          } catch (pe) {
            this.update((s) => (s.nodes.has(id) ? setError(s, id, toPayload(pe)) : s));
            return false;
          }
          if (!answer) return false;
          password = answer.password;
          remember = answer.remember;
          continue;
        }
        this.update((s) => (s.nodes.has(id) ? setError(s, id, error) : s));
        return false;
      }
    }
  }

  async expand(id: string): Promise<void> {
    const node = this.get().nodes.get(id);
    if (!node?.hasChildren) return;
    if (!(await this.ensureConnected(id))) return;
    if (!this.get().nodes.has(id)) return;
    this.update((s) => expand(s, id));
    if (!this.get().children.has(id)) await this.load(id);
  }

  collapse(id: string): void {
    this.update((s) => collapse(s, id));
  }

  async toggle(id: string): Promise<void> {
    if (this.get().expanded.has(id)) this.collapse(id);
    else await this.expand(id);
  }

  select(id: string): void {
    this.update((s) => select(s, id));
  }

  async refresh(id: string): Promise<void> {
    if (id === ROOT || this.get().children.has(id)) await this.load(id);
  }

  async reloadServers(): Promise<void> {
    await this.load(ROOT);
    const groups = (this.get().children.get(ROOT) ?? []).filter(
      (gid) => this.get().nodes.get(gid)?.kind === "serverGroup" && this.get().children.has(gid),
    );
    for (const gid of groups) await this.load(gid);
  }

  handleKey(key: string): boolean {
    const state = this.get();
    const intent = keyIntent(state, flatten(state), key);
    if (!intent) return false;
    this.apply(intent).catch((e) => {
      console.error("explorer key action failed", e);
      const sel = this.get().selected;
      if (sel !== null && this.get().nodes.has(sel)) this.update((s) => setError(s, sel, toPayload(e)));
    });
    return true;
  }

  private async apply(intent: Intent): Promise<void> {
    if (intent.select !== undefined) this.select(intent.select);
    if (intent.collapse !== undefined) this.collapse(intent.collapse);
    if (intent.expand !== undefined) await this.expand(intent.expand);
    if (intent.refresh !== undefined) await this.refresh(intent.refresh);
  }
}
