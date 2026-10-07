// Uygulama sonrası başlık ve ek ağaç yenileme kimlikleri (saf; designer-open parçasında).
// Düğüm kimlikleri ayrıştırılmaz: düğümler ağaç yapısı ve etiketleriyle bulunur.
import { tdes } from "../../app/strings-designer";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeState } from "../explorer/tree-model";
import type { AppliedInfo } from "./designer-controller";
import { kindLabel } from "./designer-model";

export interface OpenContext {
  /** Sekmenin açıldığı ağaçtaki veritabanı düğümü (yoksa null). */
  databaseNodeId: string | null;
  /** Table alter: tablo düğümü (tablo düğümünden ya da kolon/kısıt düğümünün tablo atasından); diğerleri null. */
  tableNodeId: string | null;
}

const schemaOf = (d: object | null): string | null => {
  const v = d === null ? undefined : (d as { schema?: unknown }).schema;
  return typeof v === "string" ? v : null;
};

/** Başlık güncellemesi: create→alter ya da ad/şema değişiminde `tdes.tab.edit(kindLabel, after.name)`; aksi halde null. */
export function titleAfterApply(info: AppliedInfo): string | null {
  const name = info.after.name;
  if (!name) return null;
  const title = tdes.tab.edit(kindLabel(info.kind), name);
  if (info.mode === "create" || !info.before) return title;
  return info.before.name !== name || schemaOf(info.before) !== schemaOf(info.after) ? title : null;
}

/**
 * `refreshNodeId` dışındaki yenilemeler (yalnızca yüklü — `tree.children.has(id)` — düğümler, tekrarsız, `primary` hariç):
 * (1) table: tablo düğümünün yüklü columnCollection/constraintCollection/indexCollection çocukları;
 * (2) sequence/table şema değişimi: veritabanı düğümünün schemaCollection'ındaki `after.schema` etiketli şemanın
 *     `${kind}Collection` çocuğu. Düğüm bulunamazsa hiçbir şey eklenmez.
 */
export function extraRefreshIds(tree: TreeState, ctx: OpenContext | null, info: AppliedInfo, primary: string | null): string[] {
  const out: string[] = [];
  const add = (id: string | undefined) => {
    if (id !== undefined && id !== primary && !out.includes(id) && tree.children.has(id)) out.push(id);
  };
  const kids = (id: string): string[] => tree.children.get(id) ?? [];
  const kindOf = (id: string): NodeKind | undefined => tree.nodes.get(id)?.kind;

  if (info.kind === "table" && ctx?.tableNodeId && tree.nodes.has(ctx.tableNodeId)) {
    const wanted: NodeKind[] = ["columnCollection", "constraintCollection", "indexCollection"];
    for (const c of kids(ctx.tableNodeId)) if (wanted.includes(kindOf(c) as NodeKind)) add(c);
  }

  if ((info.kind === "sequence" || info.kind === "table") && info.mode === "alter" && info.before) {
    const to = schemaOf(info.after);
    if (to !== null && to !== schemaOf(info.before) && ctx?.databaseNodeId) {
      const target: NodeKind = info.kind === "table" ? "tableCollection" : "sequenceCollection";
      for (const sc of kids(ctx.databaseNodeId)) {
        if (kindOf(sc) !== "schemaCollection") continue;
        for (const sch of kids(sc)) {
          const n = tree.nodes.get(sch);
          if (n?.kind !== "schema" || n.label !== to) continue;
          for (const c of kids(sch)) if (kindOf(c) === target) add(c);
        }
      }
    }
  }
  return out;
}
