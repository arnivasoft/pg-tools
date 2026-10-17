// Salt okunur yetki listeleri (Plan 22, spec §8; tembel parça): "Başka verenlerin yetkileri" (`foreign`) ve "Kolon
// yetkileri" (`columns`). Yalnız doluysa görünür; hiçbir denetim yoktur. Metinler `labels` ile gelir.
import { For, Show, createUniqueId } from "solid-js";
import type { ColumnGrant } from "../../ipc/bindings/ColumnGrant";
import type { ForeignGrant } from "../../ipc/bindings/ForeignGrant";
import type { Privilege } from "../../ipc/bindings/Privilege";

export interface AclReadOnlyLabels {
  foreignTitle: string;
  columnsTitle: string;
  grantor: string;
  grantee: string;
  privilege: string;
  grantOption: string;
  column: string;
  yes: string;
  publicName: string;
  privilegeName: (p: Privilege) => string;
}

export function AclReadOnlyLists(props: { foreign: ForeignGrant[]; columns: ColumnGrant[]; labels: AclReadOnlyLabels }) {
  const fid = createUniqueId();
  const cid = createUniqueId();
  const who = (g: string | null) => g ?? props.labels.publicName;
  const th = "px-2 py-1 text-left text-xs font-medium text-muted";
  const td = "px-2 py-1";
  return (
    <>
      <Show when={props.foreign.length > 0}>
        <section aria-labelledby={fid} class="flex flex-col gap-1">
          <h3 id={fid} class="text-[13px] font-medium">
            {props.labels.foreignTitle}
          </h3>
          <table class="border-collapse text-[13px]">
            <thead>
              <tr>
                <th scope="col" class={th}>{props.labels.grantor}</th>
                <th scope="col" class={th}>{props.labels.grantee}</th>
                <th scope="col" class={th}>{props.labels.privilege}</th>
                <th scope="col" class={th}>{props.labels.grantOption}</th>
              </tr>
            </thead>
            <tbody>
              <For each={props.foreign}>
                {(f) => (
                  <tr class="border-t border-border">
                    <td class={`${td} font-mono`}>{f.grantor}</td>
                    <td class={`${td} font-mono`}>{who(f.grantee)}</td>
                    <td class={td}>{props.labels.privilegeName(f.privilege)}</td>
                    <td class={td}>{f.grantOption ? props.labels.yes : ""}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </section>
      </Show>
      <Show when={props.columns.length > 0}>
        <section aria-labelledby={cid} class="flex flex-col gap-1">
          <h3 id={cid} class="text-[13px] font-medium">
            {props.labels.columnsTitle}
          </h3>
          <table class="border-collapse text-[13px]">
            <thead>
              <tr>
                <th scope="col" class={th}>{props.labels.column}</th>
                <th scope="col" class={th}>{props.labels.grantor}</th>
                <th scope="col" class={th}>{props.labels.grantee}</th>
                <th scope="col" class={th}>{props.labels.privilege}</th>
              </tr>
            </thead>
            <tbody>
              <For each={props.columns}>
                {(c) => (
                  <tr class="border-t border-border">
                    <td class={`${td} font-mono`}>{c.column}</td>
                    <td class={`${td} font-mono`}>{c.grantor}</td>
                    <td class={`${td} font-mono`}>{who(c.grantee)}</td>
                    <td class={td}>
                      {props.labels.privilegeName(c.privilege)}
                      {c.grantOption ? ` (${props.labels.grantOption})` : ""}
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </section>
      </Show>
    </>
  );
}
