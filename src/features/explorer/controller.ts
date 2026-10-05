import type { Api } from "../../ipc/commands";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { toPayload } from "../../ipc/errors";
import {
  ROOT, collapse, expand, flatten, invalidate, keyIntent, patchNode, select, serverOf, setChildren, setError, startLoading, stopLoading,
  type Intent, type TreeState,
} from "./tree-model";

/** Arka uç oturumu yok/düşürdü: sunucu bağlantısız sayılmalı, sonraki açılış yeniden bağlanır. */
const SESSION_GONE = new Set(["not_connected", "connection_lost"]);

export function isSessionGone(code: string): boolean {
  return SESSION_GONE.has(code);
}

/** Sunucuyu bağlantısız işaretler, alt ağacını boşaltır ve hatayı sunucu düğümüne koyar. */
function markDisconnected(s: TreeState, loadingId: string, serverId: string, error: ErrorPayload): TreeState {
  const server = s.nodes.get(serverId);
  if (!server) return s;
  let next = patchNode(s, serverId, { detail: null, flags: { ...server.flags, connected: false } });
  next = invalidate(stopLoading(next, loadingId), serverId);
  return setError(next, serverId, error);
}

export type PasswordAnswer = { password: string; remember: boolean };

export interface ExplorerDeps {
  api: Pick<Api, "listChildren" | "connectServer" | "disconnectServer">;
  /** `retry`: önceki parola reddedildi; `error` o reddin ayrıntısını (sunucunun mesajı) taşır. */
  requestPassword(node: TreeNode, retry: boolean, error: ErrorPayload | null): Promise<PasswordAnswer | null>;
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

  /**
   * Arka uç oturumunun düştüğü başka bir yoldan (ör. Inspector çağrısı) öğrenildiğinde çağrılır.
   * `id` bir sunucunun altındaysa sunucuyu bağlantısız işaretler; aksi halde hiçbir şey yapmaz.
   */
  sessionLost(id: string, error: ErrorPayload): boolean {
    if (!isSessionGone(error.code)) return false;
    const server = serverOf(this.get(), id);
    if (!server) return false;
    this.update((s) => markDisconnected(s, id, server.id, error));
    return true;
  }

  load(id: string): Promise<void> {
    const pending = this.loads.get(id);
    if (pending) return pending;
    const p = this.doLoad(id).finally(() => this.loads.delete(id));
    this.loads.set(id, p);
    return p;
  }

  /** Süren bir yüklemeyi bekler ve listeyi yeniden okur (süren yükleme değişiklikten önce başlamış olabilir). */
  async reload(id: string): Promise<void> {
    await this.loads.get(id)?.catch(() => {});
    return this.load(id);
  }

  private async doLoad(id: string): Promise<void> {
    this.update((s) => startLoading(s, id));
    try {
      const list = await this.deps.api.listChildren(id);
      this.update((s) => setChildren(s, id, list));
    } catch (e) {
      const error = toPayload(e);
      const server = SESSION_GONE.has(error.code) ? serverOf(this.get(), id) : null;
      if (server) this.update((s) => markDisconnected(s, id, server.id, error));
      else this.update((s) => setError(s, id, error));
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
            const retry = error.code === "auth";
            answer = await this.deps.requestPassword(cur, retry, retry ? error : null);
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

  setFilter(query: string): void {
    this.update((s) => ({ ...s, filter: query }));
  }

  select(id: string): void {
    this.update((s) => select(s, id));
  }

  async refresh(id: string): Promise<void> {
    if (id === ROOT || this.get().children.has(id)) await this.load(id);
  }

  /** Yüklü tüm listeleri yeniden okur: önce ROOT, sonra derinliğe göre; sıralı, hata yutulur, bağlantısız sunucular atlanır. */
  async reloadAll(): Promise<void> {
    const depthOf = (id: string): number => {
      let d = 0;
      for (let cur = this.get().parent.get(id); cur !== undefined && cur !== ROOT; cur = this.get().parent.get(cur)) d++;
      return d;
    };
    const ids = [...this.get().children.keys()].sort((a, b) => (a === ROOT ? -1 : b === ROOT ? 1 : depthOf(a) - depthOf(b)));
    for (const id of ids) {
      try {
        if (id !== ROOT) {
          if (!this.get().nodes.has(id)) continue; // önceki yenilemede kalktı
          const server = serverOf(this.get(), id);
          if (server && !server.flags.connected) continue;
        }
        if (this.get().children.has(id)) await this.reload(id);
      } catch (e) {
        console.error("reloadAll failed for", id, e);
      }
    }
  }

  async retry(id: string): Promise<void> {
    const node = this.get().nodes.get(id);
    if (!node) return;
    if (node.kind === "server" && !node.flags.connected) {
      if (!(await this.ensureConnected(id))) return;
      if (!this.get().nodes.has(id)) return;
      this.update((s) => expand(s, id));
    }
    await this.load(id);
  }

  /** Düğümün artık var olmadığı (not_found) başka bir yoldan öğrenildiğinde satırı hatalı işaretler. */
  nodeGone(id: string, error: ErrorPayload): boolean {
    if (error.code !== "not_found" || !this.get().nodes.has(id)) return false;
    this.update((s) => setError(s, id, error));
    return true;
  }

  disconnect(id: string): void {
    const node = this.get().nodes.get(id);
    if (node?.kind !== "server" || !node.serverId) return;
    void Promise.resolve(this.deps.api.disconnectServer(node.serverId)).catch((e) => console.error("disconnect failed", e));
    this.update((s) => {
      const n = s.nodes.get(id);
      return n ? invalidate(patchNode(s, id, { detail: null, flags: { ...n.flags, connected: false } }), id) : s;
    });
  }

  /** Sunucu kaydı silindi: düğümü ve alt ağacını kaldırmak için sunucu listesini yeniden yükler. */
  async serverRemoved(): Promise<void> {
    await this.reloadServers();
  }

  async reloadServers(): Promise<void> {
    await this.reload(ROOT);
    const groups = (this.get().children.get(ROOT) ?? []).filter(
      (gid) => this.get().nodes.get(gid)?.kind === "serverGroup" && this.get().children.has(gid),
    );
    for (const gid of groups) await this.reload(gid);
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
