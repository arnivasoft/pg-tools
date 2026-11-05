import { describe, expect, it } from "vitest";
import {
  PREVIEW_CSP,
  bytesToBase64,
  dataUrl,
  decodeByteaHex,
  formatHtml,
  formatXml,
  hexRow,
  hexRowCount,
  imageMime,
  pdfVersion,
  previewDoc,
  saveExt,
  suggestSaveName,
  utf8Bytes,
} from "./cell-format";

// Test ortamı (node) Buffer sağlar; tsconfig node türlerini içermez.
declare const Buffer: { from(b: Uint8Array): { toString(enc: string): string } };

const arr =(u: Uint8Array | null) => (u ? Array.from(u) : null);

describe("decodeByteaHex", () => {
  it("boş, büyük/küçük harf, bozuk girdi ve sınır", () => {
    expect(arr(decodeByteaHex("\\x"))).toEqual([]);
    expect(arr(decodeByteaHex("\\x0A0b"))).toEqual([10, 11]);
    expect(decodeByteaHex("\\x0a0")).toBeNull();
    expect(decodeByteaHex("\\x0a0g")).toBeNull();
    expect(decodeByteaHex("0a")).toBeNull();
    expect(arr(decodeByteaHex("\\x00010203", 2))).toEqual([0, 1]);
  });
});

describe("utf8Bytes", () => {
  it("kodlar ve keser", () => {
    expect(arr(utf8Bytes("ş"))).toEqual([0xc5, 0x9f]);
    expect(arr(utf8Bytes("abc", 2))).toEqual([0x61, 0x62]);
  });
});

describe("hexRow", () => {
  const bytes = new Uint8Array(17);
  "\x89PNG\r\n\x1a\n\0\0\0\rIHDR".split("").forEach((c, i) => (bytes[i] = c.charCodeAt(0)));
  bytes[16] = 0x78;
  it("tam satır", () => {
    expect(hexRow(bytes, 0)).toBe(
      "00000000  89 50 4E 47 0D 0A 1A 0A  00 00 00 0D 49 48 44 52  |.PNG........IHDR|",
    );
  });
  it("son satır eksik baytlarla", () => {
    expect(hexRow(bytes, 1)).toBe("00000010  78" + " ".repeat(48) + "|x|");
  });
  it("ASCII dışı nokta", () => {
    expect(hexRow(new Uint8Array([0x1f, 0x7f, 0x41]), 0).endsWith("|..A|")).toBe(true);
  });
  it("satır sayısı", () => {
    expect(hexRowCount(0)).toBe(0);
    expect(hexRowCount(16)).toBe(1);
    expect(hexRowCount(17)).toBe(2);
  });
});

describe("base64 ve görsel", () => {
  it("Buffer ile eşit", () => {
    for (const n of [0, 1, 2, 3, 0x8000 + 1]) {
      const b = new Uint8Array(n);
      for (let i = 0; i < n; i++) b[i] = (i * 31 + 7) & 255;
      expect(bytesToBase64(b)).toBe(Buffer.from(b).toString("base64"));
    }
  });
  it("data URL ve MIME", () => {
    expect(dataUrl("png", new Uint8Array([1])).startsWith("data:image/png;base64,")).toBe(true);
    expect(imageMime("svg")).toBe("image/svg+xml");
    expect(imageMime("ico")).toBe("image/x-icon");
    expect(imageMime("jpeg")).toBe("image/jpeg");
  });
});

describe("pdfVersion", () => {
  const b = (s: string) => new TextEncoder().encode(s);
  it("sürümü okur", () => {
    expect(pdfVersion(b("%PDF-1.7\n"))).toBe("1.7");
    expect(pdfVersion(b("%PDF-2.0"))).toBe("2.0");
    expect(pdfVersion(b("%PDF-"))).toBeNull();
    expect(pdfVersion(b("abc"))).toBeNull();
  });
});

describe("formatHtml", () => {
  it("girintiler, boş öğeler girinti artırmaz", () => {
    expect(formatHtml("<div><p>a</p><br><img src=x></div>")).toBe(
      "<div>\n  <p>\n    a\n  </p>\n  <br>\n  <img src=x>\n</div>",
    );
  });
  it("ham içerik korunur", () => {
    expect(formatHtml("<pre>  a\n b</pre>")).toBe("<pre>  a\n b</pre>");
    expect(formatHtml("<div><script>if (a<b) x()</script></div>")).toContain("<script>if (a<b) x()</script>");
    expect(formatHtml("<style>p>a{}</style>")).toBe("<style>p>a{}</style>");
  });
  it("öznitelikte > etiketi bitirmez", () => {
    expect(formatHtml('<a title="x>y">t</a>')).toBe('<a title="x>y">\n  t\n</a>');
  });
  it("doctype, yorum, CDATA ayrı satırda", () => {
    expect(formatHtml("<!DOCTYPE html><!-- y --><![CDATA[z]]>")).toBe("<!DOCTYPE html>\n<!-- y -->\n<![CDATA[z]]>");
  });
  it("kapanmamış yapılar", () => {
    expect(formatHtml("<!-- açık")).toBe("<!-- açık");
    expect(formatHtml('<div class="a')).toBe('<div class="a');
    expect(formatHtml("<div><pre>abc")).toBe("<div>\n  <pre>abc");
  });
  it("kenar durumlar", () => {
    expect(formatHtml("a < b")).toBe("a < b");
    expect(formatHtml("</div><p>x</p>")).toBe("</div>\n<p>\n  x\n</p>");
    expect(formatHtml("<div><br/>x</div>")).toBe("<div>\n  <br/>\n  x\n</div>");
  });
  it("1 MiB hızlı", () => {
    const big = "<p>abc</p>".repeat(Math.ceil((1 << 20) / 10));
    const t0 = performance.now();
    formatHtml(big);
    expect(performance.now() - t0).toBeLessThan(200);
  });
});

describe("formatXml", () => {
  it("örnek belge", () => {
    expect(
      formatXml('<?xml version="1.0"?><kisi><ad>Ayşe</ad><adres><il>İzmir</il></adres><bos/></kisi>'),
    ).toBe('<?xml version="1.0"?>\n<kisi>\n  <ad>Ayşe</ad>\n  <adres>\n    <il>İzmir</il>\n  </adres>\n  <bos/>\n</kisi>');
  });
  it("karma içerik", () => {
    expect(formatXml("<p>a<b>x</b> c</p>")).toBe("<p>\n  a\n  <b>x</b>\n  c\n</p>");
  });
  it("HTML özel durumları uygulanmaz", () => {
    expect(formatXml("<script><a>1</a></script>")).toBe("<script>\n  <a>1</a>\n</script>");
    expect(formatXml("<r><br><x/></r>")).toBe("<r>\n  <br>\n    <x/>\n  </r>");
  });
  it("xml:space=preserve içeriği birebir", () => {
    const a = '<t xml:space="preserve">  a <i>b</i>\n</t>';
    expect(formatXml(a)).toBe(a);
    const b = '<r><t xml:space="preserve"><t>x</t> </t></r>';
    expect(formatXml(b)).toBe('<r>\n  <t xml:space="preserve"><t>x</t> </t>\n</r>');
  });
  it("tek parça yapılar", () => {
    expect(formatXml("<a><![CDATA[a<b>c]]></a>")).toBe("<a>\n  <![CDATA[a<b>c]]>\n</a>");
    expect(formatXml("<!-- <x> -->")).toBe("<!-- <x> -->");
    expect(formatXml("<?pi a>b?>")).toBe("<?pi a>b?>");
    expect(formatXml('<!DOCTYPE k [<!ENTITY e "v">]><k>&amp;&e;</k>')).toBe(
      '<!DOCTYPE k [<!ENTITY e "v">]>\n<k>&amp;&e;</k>',
    );
  });
  it("uyuşmazlık ve kapanmamış", () => {
    expect(formatXml("<Ad>x</ad>")).toBe("<Ad>\n  x\n</ad>");
    expect(formatXml("<a><b>")).toBe("<a>\n  <b>");
    expect(formatXml("</a></b><c/>")).toBe("</a>\n</b>\n<c/>");
  });
  it("kök öğesiz parça", () => {
    expect(formatXml("metin<a/>son")).toBe("metin\n<a/>\nson");
    expect(formatXml("düz")).toBe("düz");
  });
  it("1 MiB hızlı", () => {
    const big = "<a>abc</a><b x=\"1\"/>".repeat(Math.ceil((1 << 20) / 20));
    const t0 = performance.now();
    formatXml(big);
    expect(performance.now() - t0).toBeLessThan(200);
  });
});

describe("previewDoc", () => {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}">`;
  it("başa ekler", () => {
    expect(previewDoc("<p>x</p>")).toBe(meta + "<p>x</p>");
  });
  it("doctype'tan sonra", () => {
    expect(previewDoc("  <!DOCTYPE html><html>x</html>")).toBe("  <!DOCTYPE html>" + meta + "<html>x</html>");
    expect(previewDoc("<!doctype html><p>")).toBe("<!doctype html>" + meta + "<p>");
  });
});

describe("kaydetme adı", () => {
  it("temizler", () => {
    expect(suggestSaveName("foto:1", "png")).toBe("foto_1.png");
    expect(suggestSaveName("", "pdf")).toBe("hucre.pdf");
    expect(suggestSaveName("a".repeat(150), "bin").length).toBe(104);
  });
  it("uzantı", () => {
    expect(saveExt({ pdf: true, image: null })).toBe("pdf");
    expect(saveExt({ pdf: false, image: "jpeg" })).toBe("jpg");
    expect(saveExt({ pdf: false, image: "png" })).toBe("png");
    expect(saveExt({ pdf: false, image: null })).toBe("bin");
  });
});
