// Asistan paneli için saf yardımcılar: yanıtı metin/SQL bloklarına bölme ve şema bağlamı metni.
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";

export type AiBlock = { kind: "text"; text: string } | { kind: "code"; lang: string; text: string; open: boolean };

/**
 * Yanıtı ``` çitlerine göre böler. Akış sürerken kapanmamış son blok `open: true` döner (ekleme düğmesi gizlenir).
 */
export function splitBlocks(text: string): AiBlock[] {
  const out: AiBlock[] = [];
  const re = /^```([\w-]*)[ \t]*\r?\n/gm;
  let pos = 0;
  for (;;) {
    re.lastIndex = pos;
    const m = re.exec(text);
    if (!m) break;
    const before = text.slice(pos, m.index);
    if (before.trim()) out.push({ kind: "text", text: before.trim() });
    const bodyStart = m.index + m[0].length;
    const close = /^```[ \t]*$/m;
    const rest = text.slice(bodyStart);
    const c = close.exec(rest);
    if (!c) {
      out.push({ kind: "code", lang: m[1].toLowerCase(), text: rest.replace(/\s+$/, ""), open: true });
      return out;
    }
    out.push({ kind: "code", lang: m[1].toLowerCase(), text: rest.slice(0, c.index).replace(/\s+$/, ""), open: false });
    pos = bodyStart + c.index + c[0].length;
  }
  const tail = text.slice(pos);
  if (tail.trim()) out.push({ kind: "text", text: tail.trim() });
  return out;
}

/** Unicode scalar ofsetleriyle dilim (arka uç ofsetleri karakter tabanlıdır). */
export function sliceChars(text: string, start: number, len: number): string {
  return Array.from(text)
    .slice(start, start + len)
    .join("");
}

/** İstemde adı geçen ilişkiler (büyük/küçük harf duyarsız, tam sözcük), en çok `max` tane. */
export function mentionedRelations(prompt: string, relations: CompletionItem[], max = 20): CompletionItem[] {
  const words = new Set(prompt.toLocaleLowerCase("en-US").split(/[^\p{L}\p{N}_$]+/u).filter(Boolean));
  return relations.filter((r) => words.has(r.label.toLocaleLowerCase("en-US"))).slice(0, max);
}

/** Bağlam metni: arama yolundaki ilişki adları ve (varsa) seçilmiş ilişkilerin kolonları. */
export function schemaText(relations: CompletionItem[], columns: Map<string, CompletionItem[]>): string {
  const lines: string[] = [];
  if (relations.length > 0) {
    lines.push(`Relations in search_path: ${relations.map((r) => `${r.quoted} (${r.kind})`).join(", ")}`);
  }
  for (const [rel, cols] of columns) {
    if (cols.length === 0) continue;
    lines.push(`${rel}: ${cols.map((c) => (c.detail ? `${c.quoted} ${c.detail}` : c.quoted)).join(", ")}`);
  }
  return lines.join("\n");
}
