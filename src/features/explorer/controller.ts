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

  async load(id: string): Promise<void> {
    this.update((s) => startLoading(s, id));
    try {
      const list = await this.deps.api.listChildren(id);
      this.update((s) => setChildren(s, id, list));
    } catch (e) {
      this.update((s) => setError(s, id, toPayload(e)));
    }
  }

  async ensureConnected(id: string): Promise<boolean> {
    const node = this.get().nodes.get(id);
    if (!node || node.kind !== "server" || node.flags.connected || !node.serverId) return true;
    let password: string | null = null;
    let remember = false;
    for (;;) {
      this.update((s) => startLoading(s, id));
      try {
        const info = await this.deps.api.connectServer(node.serverId, password, remember);
        this.update((s) => patchNode(stopLoading(s, id), id, { detail: info.version, flags: { ...node.flags, connected: true } }));
        return true;
      } catch (e) {
        const error = toPayload(e);
        if (error.code === "password_required" || error.code === "auth") {
          this.update((s) => stopLoading(s, id));
          const answer = await this.deps.requestPassword(node, error.code === "auth");
          if (!answer) return false;
          password = answer.password;
          remember = answer.remember;
          continue;
        }
        this.update((s) => setError(s, id, error));
        return false;
      }
    }
  }

  async expand(id: string): Promise<void> {
    const node = this.get().nodes.get(id);
    if (!node?.hasChildren) return;
    if (!(await this.ensureConnected(id))) return;
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
    void this.apply(intent);
    return true;
  }

  private async apply(intent: Intent): Promise<void> {
    if (intent.select !== undefined) this.select(intent.select);
    if (intent.collapse !== undefined) this.collapse(intent.collapse);
    if (intent.expand !== undefined) await this.expand(intent.expand);
    if (intent.refresh !== undefined) await this.refresh(intent.refresh);
  }
}
