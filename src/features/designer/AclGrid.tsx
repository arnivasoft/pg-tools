// Yetki ızgarası (Plan 22; tembel parça). Satır = alıcı, sütun = `available` yetkileri; hücre üç durumlu düğmedir
// (yok → var → var + GRANT OPTION → yok; PUBLIC ve sahip satırında son adım atlanır). Mantık saf `acl-grid-model`'dedir.
// Erişilebilirlik: her hücre "<alıcı> — <yetki>: <durum>" adlı düğmedir (`labels.cell`; `aria-pressed` yok; durum
// metindir); ızgarada ok tuşları/Home/End ile gezinilir (gezgin tabindex: yalnız etkin hücre sekme durağı); Space/Enter
// düğmenin tıklamasıyla durumu değiştirir; IME bileşimi sırasında tuşlar yok sayılır. Salt okunur ızgarada hücreler
// `disabled` değil `aria-disabled`dır: sekme durağı ve gezinme kalır, döngü etkisizdir (`cycleCell` → null). PUBLIC
// satırı `labels.publicName` sözde adıyla gösterilir ("PUBLIC" adlı gerçek rolden ayrı). Hücre hatası `cellPath` ile `issues`'tan okunur
// (`aria-invalid` + satır altı hata metni). `available` dışı yetkiler satırda salt okunur gösterilir.
// Metinler `labels` ile gelir (Task 4: `strings-privileges.ts`); alıcı ekleme denetimini form sağlar (`footer`).
import { Trash2 } from "lucide-solid";
import { For, Index, Show, createSignal, createUniqueId, type JSX } from "solid-js";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { Privilege } from "../../ipc/bindings/Privilege";
import { Button } from "../../ui/Button";
import {
  cellPath, cellState, cycleCell, granteeLabel, nextCellFocus, unknownPrivileges, type CellState, type FocusKey,
} from "./acl-grid-model";
import { issuesFor, type FieldIssues } from "./designer-model";

export interface AclGridLabels {
  /** Izgara adı (`aria-label`). */
  grid: string;
  /** Yetki adı (sütun başlığı ve hücre adı). */
  privilege: (p: Privilege) => string;
  /** Durum metinleri: yok / var / devredilebilir. */
  state: Record<CellState, string>;
  /** PUBLIC sözde satırının adı ("PUBLIC" adlı gerçek rolden ayırt edilir). */
  publicName: string;
  /** Hücrenin erişilebilir adı: alıcı, yetki adı, durum metni. */
  cell: (grantee: string, privilege: string, state: string) => string;
  /** Sahip rozeti. */
  owner: string;
  /** Alıcı sütun başlığı. */
  grantee: string;
  /** Satır kaldırma düğmesi adı ("<alıcı> kaldır"). */
  remove: (grantee: string) => string;
  /** Bilinmeyen yetki gösterimi ("Bilinmeyen yetki: x"). */
  unknown: (name: string) => string;
  /** Satır kaldırma sütununun başlığı (görünmez). */
  actions: string;
  empty: string;
}

export interface AclGridProps {
  grants: AclGrant[];
  available: Privilege[];
  owner: string;
  /** Hata yolu kökü: `"grants"` ya da `"sections[i].grants"`. */
  basePath: string;
  issues: FieldIssues;
  readOnly: boolean;
  labels: AclGridLabels;
  onChange: (grants: AclGrant[]) => void;
  /**
   * Satır kaldırma (tüm yetkileri kaldırmak demektir); verilmezse düğme yok. Alıcı ekleme denetimi `footer`'dadır.
   * Kaldırma sonrası odak aynı sıradaki (yoksa önceki) satırın ilk hücresine, satır kalmadıysa `footer` içindeki
   * `data-acl-add` öğesine gider.
   */
  onRemove?: (row: number) => void;
  footer?: JSX.Element;
}

const NAV_KEYS = new Set<string>(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]);

export function AclGrid(props: AclGridProps) {
  const gid = createUniqueId();
  let root: HTMLTableElement | undefined;
  let box: HTMLDivElement | undefined;
  const [active, setActive] = createSignal<[number, number]>([0, 0]);
  const cols = () => props.available.length;
  // Satır/sütun azalınca etkin hücre ızgara içinde kalır (yoksa hiçbir hücre sekme durağı olmazdı).
  const act = (): [number, number] => {
    const [r, c] = active();
    return [Math.min(r, Math.max(props.grants.length - 1, 0)), Math.min(c, Math.max(cols() - 1, 0))];
  };
  const name = (g: AclGrant) => granteeLabel(g, props.labels.publicName);
  const cellIssues = (r: number, p: Privilege) => {
    const path = cellPath(props.basePath, r, props.grants, p);
    return path === null ? [] : issuesFor(props.issues, path);
  };
  const rowIssues = (r: number) => issuesFor(props.issues, `${props.basePath}[${r}].grantee`);
  const errId = (r: number) => `${gid}-err-${r}`;

  const cycle = (r: number, p: Privilege) => {
    const next = cycleCell(props.grants, r, p, props.owner, props.available, props.readOnly);
    if (next) props.onChange(next);
  };

  const remove = (r: number) => {
    props.onRemove?.(r);
    // Kaldırılan düğme DOM'dan gidince odak kaybolmasın: komşu satır ya da ekleme denetimi (çizim sonrası).
    queueMicrotask(() => {
      const nr = Math.min(r, props.grants.length - 1);
      const cell = nr < 0 ? null : root?.querySelector<HTMLElement>(`[data-r="${nr}"][data-c="0"]`);
      if (cell) setActive([nr, 0]);
      (cell ?? box?.querySelector<HTMLElement>("[data-acl-add]"))?.focus();
    });
  };
  const removable = () => !!props.onRemove && !props.readOnly;

  const onKeyDown = (ev: KeyboardEvent, r: number, c: number) => {
    if (ev.isComposing || ev.keyCode === 229) return;
    if (!NAV_KEYS.has(ev.key) || ev.ctrlKey || ev.altKey || ev.metaKey) return;
    ev.preventDefault();
    const [nr, nc] = nextCellFocus(r, c, props.grants.length, cols(), ev.key as FocusKey);
    setActive([nr, nc]);
    root?.querySelector<HTMLElement>(`[data-r="${nr}"][data-c="${nc}"]`)?.focus();
  };

  return (
    <div ref={box} class="flex flex-col gap-2">
      <Show when={props.grants.length === 0}>
        <p class="text-xs text-muted">{props.labels.empty}</p>
      </Show>
      <Show when={props.grants.length > 0}>
        <div class="overflow-x-auto">
          <table ref={root} aria-label={props.labels.grid} class="border-collapse text-[13px]">
            <thead>
              <tr>
                <th scope="col" class="px-2 py-1 text-left text-xs font-medium text-muted">
                  {props.labels.grantee}
                </th>
                <For each={props.available}>
                  {(p) => (
                    <th scope="col" class="px-2 py-1 text-center text-xs font-medium text-muted">
                      {props.labels.privilege(p)}
                    </th>
                  )}
                </For>
                <Show when={removable()}>
                  <th scope="col" class="px-2 py-1">
                    <span class="sr-only">{props.labels.actions}</span>
                  </th>
                </Show>
              </tr>
            </thead>
            <tbody>
              <Index each={props.grants}>
                {(g, r) => (
                  <>
                    <tr class="border-t border-border">
                      <th scope="row" class="px-2 py-1 text-left font-normal">
                        <span class="flex flex-wrap items-center gap-1.5">
                          <span class="font-mono">{name(g())}</span>
                          <Show when={g().grantee !== null && g().grantee === props.owner}>
                            <span class="rounded border border-border px-1 text-[11px] text-muted">
                              {props.labels.owner}
                            </span>
                          </Show>
                        </span>
                        <Show when={unknownPrivileges(g(), props.available).length > 0}>
                          <ul role="list" class="mt-0.5 flex flex-wrap gap-1">
                            <For each={unknownPrivileges(g(), props.available)}>
                              {(u) => (
                                <li class="rounded border border-border px-1 text-[11px] text-muted">
                                  {props.labels.unknown(u)}
                                </li>
                              )}
                            </For>
                          </ul>
                        </Show>
                      </th>
                      <Index each={props.available}>
                        {(p, c) => {
                          const st = () => cellState(g(), p());
                          const errs = () => cellIssues(r, p());
                          return (
                            <td class="px-1 py-1 text-center">
                              <button
                                type="button"
                                data-r={r}
                                data-c={c}
                                tabIndex={act()[0] === r && act()[1] === c ? 0 : -1}
                                aria-disabled={props.readOnly ? true : undefined}
                                aria-label={props.labels.cell(name(g()), props.labels.privilege(p()), props.labels.state[st()])}
                                aria-invalid={errs().length > 0 ? true : undefined}
                                aria-describedby={errs().length > 0 || rowIssues(r).length > 0 ? errId(r) : undefined}
                                class={`inline-flex h-6 min-w-8 items-center justify-center rounded border px-1 text-[12px] outline-none focus:border-accent aria-disabled:opacity-60 ${
                                  st() === "none"
                                    ? "border-border text-muted"
                                    : st() === "granted"
                                      ? "border-accent bg-accent text-accent-fg"
                                      : "border-accent bg-accent text-accent-fg font-semibold"
                                }${errs().length > 0 ? " !border-danger" : ""}`}
                                onFocus={() => setActive([r, c])}
                                onKeyDown={(ev) => onKeyDown(ev, r, c)}
                                onClick={() => cycle(r, p())}
                              >
                                <span aria-hidden="true">{st() === "none" ? "·" : st() === "granted" ? "✓" : "✓+"}</span>
                              </button>
                            </td>
                          );
                        }}
                      </Index>
                      <Show when={removable()}>
                        <td class="px-1 py-1">
                          <Button
                            variant="ghost"
                            aria-label={props.labels.remove(name(g()))}
                            title={props.labels.remove(name(g()))}
                            onClick={() => remove(r)}
                          >
                            <Trash2 size={14} aria-hidden="true" />
                          </Button>
                        </td>
                      </Show>
                    </tr>
                    <Show when={rowIssues(r).length > 0 || props.available.some((p) => cellIssues(r, p).length > 0)}>
                      <tr>
                        <td id={errId(r)} colSpan={cols() + (removable() ? 2 : 1)} class="px-2 pb-1 text-xs text-danger">
                          <For each={rowIssues(r)}>{(m) => <div>{m}</div>}</For>
                          <For each={props.available}>
                            {(p) => (
                              <For each={cellIssues(r, p)}>
                                {(m) => (
                                  <div>
                                    {props.labels.privilege(p)}: {m}
                                  </div>
                                )}
                              </For>
                            )}
                          </For>
                        </td>
                      </tr>
                    </Show>
                  </>
                )}
              </Index>
            </tbody>
          </table>
        </div>
      </Show>
      {props.footer}
    </div>
  );
}
