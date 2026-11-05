import { HighlightStyle } from "@codemirror/language";
import type { Tree } from "@lezer/common";
import { describe, expect, it } from "vitest";
import { loadLanguage, viewerHighlight } from "./cell-code";

function names(tree: Tree): string[] {
  const out: string[] = [];
  tree.iterate({ enter: (n) => void out.push(n.name) });
  return out;
}

describe("loadLanguage", () => {
  it("json: JsonText kökü, hata düğümü yok", async () => {
    const { language } = await loadLanguage("json");
    const tree = language.parser.parse('{"a":[1,true,null]}');
    expect(tree.topNode.name).toBe("JsonText");
    expect(names(tree)).not.toContain("⚠");
  });

  it("xml: Element var, hata düğümü yok", async () => {
    const { language } = await loadLanguage("xml");
    const tree = language.parser.parse('<?xml version="1.0"?><a b="1"><![CDATA[x<y]]></a>');
    expect(names(tree)).toContain("Element");
    expect(names(tree)).not.toContain("⚠");
  });

  it("html: Element var, gömülü script ayrıştırması istisnasız", async () => {
    const { language } = await loadLanguage("html");
    expect(names(language.parser.parse('<div a="1"><p>x</p></div>'))).toContain("Element");
    expect(() => language.parser.parse("<script>let a=1</script>")).not.toThrow();
  });

  it("aynı dil için aynı Promise döner", () => {
    expect(loadLanguage("json")).toBe(loadLanguage("json"));
  });
});

describe("viewerHighlight", () => {
  it("HighlightStyle örneğidir", () => {
    expect(viewerHighlight).toBeInstanceOf(HighlightStyle);
    expect(viewerHighlight.module).toBeDefined();
  });
});
