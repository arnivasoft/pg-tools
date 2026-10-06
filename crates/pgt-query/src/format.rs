//! SQL biçimlendirici (tasarım §5). Saf: oturum gerektirmez; SQL metni günlüğe yazılmaz.
//!
//! **Kurallar** (yalnız tanınan ifadeler: ilk sözcüğü `SELECT`, `WITH`, `INSERT`, `UPDATE`, `DELETE`, `VALUES` olanlar,
//! baştaki `(` atlanarak):
//! - Ana yan tümceler yeni satırda (`SELECT`, `FROM`, `WHERE`, `GROUP BY`, `HAVING`, `WINDOW`, `ORDER BY`, `LIMIT`,
//!   `OFFSET`, `FETCH`, `FOR UPDATE…`, `RETURNING`, `UNION/INTERSECT/EXCEPT`, `VALUES`, `SET`, `ON CONFLICT`,
//!   `INSERT INTO`, `UPDATE`, `DELETE FROM`, `DELETE … USING`); `JOIN` türleri yeni satırda, `ON`/`USING` aynı satırda.
//! - Liste yan tümceleri (seçim listesi, `FROM`, `GROUP BY`, `ORDER BY`, `RETURNING`, `SET`, `VALUES`, `WINDOW`) tek
//!   satıra sığmazsa (satırın girintisi + yan tümce + liste > `max_line`) her öğe ayrı satıra (bir girinti içeride).
//! - `WHERE`/`HAVING`/`JOIN … ON` koşulu sığmazsa üst düzey `AND`/`OR` satır başına (`BETWEEN … AND` hariç).
//! - Alt sorgu / CTE gövdesi (`(` + `SELECT|WITH|VALUES|INSERT|UPDATE|DELETE`) her zaman çok satırlı: içerik `(`'in
//!   bulunduğu satırın girintisi + bir girinti, `)` o satırın girintisinde.
//! - `CASE` sığmazsa `WHEN`/`ELSE` bir girinti içeride alt alta, `END` `CASE`'in satırının girintisinde.
//! - Virgül: listede sonda (`a,`) ya da başta (`, b`); satır içinde `a, b`. `(` sonrası ve `)`/`,` öncesi boşluk
//!   kaldırılır; diğer belirteç aralarında özgün bitişiklik korunur (operatörler, `::`, `.`, `->>`, `B'01'` …), boşluk
//!   tek boşluğa iner. Satır içi parantezler (fonksiyon çağrısı, `IN (…)` listesi) bölünmez.
//!
//! **Anahtar sözcük harfi** yalnızca seçilmiş listeye (`ALWAYS`; `.`'ya bitişik değilse) ve bağlamlı sözcüklere
//! (`CONTEXT`: ör. `first`/`last` yalnızca `NULLS`'tan sonra, `left`/`right` yalnızca `JOIN`/`OUTER`'dan önce, `by`
//! yalnızca `GROUP`/`ORDER`/`PARTITION`'dan sonra) uygulanır. `name`, `type`, `value`, `data`, tür adları ve fonksiyon
//! adları gibi tanımlayıcı olabilecek sözcüklere dokunulmaz. Tırnaksız sözcükler PostgreSQL'de zaten küçük harfe
//! katlandığından harf değişimi anlamı değiştirmez.
//!
//! **Yorumlar:** yeri korunur. Kod ile aynı satırdaki yorum önceki belirtecin ardında kalır; kendi satırındaki yorum
//! kendi satırında kalır; `--` yorumundan sonra her zaman satır sonu gelir.
//!
//! **Tanınmayan ifadeler** (`CREATE FUNCTION … BEGIN ATOMIC`, `COPY`, `DO`, DDL, …) ve çok derin iç içe ifadeler
//! (> `MAX_DEPTH`): yalnız boşluk sadeleştirme — satır sonları ve satır başı girintileri korunur, satır içi boşluk
//! dizileri tek boşluğa iner, satır sonu boşlukları silinir; harf değişmez.
//!
//! **Betik düzeni:** ifadeler arasında bir boş satır; ifadeler arası yorumlar özgün satır yapısıyla (çoklu boş satır
//! teke iner) ardından gelen ifadeye bağlanır; `;` ifadenin hemen ardına gelir. Baştaki boşluk atılır; çıktı yalnızca
//! girdinin son belirtecinden sonra satır sonu varsa tek bir `\n` ile biter.
//!
//! **Güvenlik sözleşmesi (§5.2):** her çağrının sonunda çıktının belirteç dizisi girdiyle karşılaştırılır (boşluk
//! hariç; yorumlar, dizeler, dolar tırnaklı gövdeler, tırnaklı tanımlayıcılar bayt bayt; yalnız harfi değiştirilebilen
//! sözcükler harfe duyarsız; anlamlı bitişiklikler — operatör parçaları, `.`, `B'…'` — korunmalı) ve
//! `format(çıktı) == çıktı` doğrulanır. Herhangi biri bozulursa `Err(Invalid(FORMAT_FAILED))`; bozuk sonuç dönmez.

use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::lex::{Kind, Token, lex};
use crate::split::split;

/// Sözleşme bozulduğunda dönen `Error::Invalid` işareti.
pub const FORMAT_FAILED: &str = "biçimlendirilemedi";

/// Biçimlendirilebilecek en büyük metin.
pub const MAX_FORMAT_BYTES: usize = 5 * 1024 * 1024;
pub const FORMAT_TOO_LARGE: &str = "biçimlendirilecek metin 5 MB'tan büyük";

/// Bu derinliği (parantez ya da iç içe `CASE`) aşan ifadeler yalnız boşluk sadeleştirmesi alır (yığın güvenliği).
const MAX_DEPTH: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KeywordCase {
    #[default]
    Upper,
    Lower,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CommaStyle {
    #[default]
    Trailing,
    Leading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct FormatOptions {
    pub keyword_case: KeywordCase,
    /// Girinti boşluğu: 2 ya da 4.
    pub indent: u8,
    pub comma: CommaStyle,
    /// Satır uzunluğu: 80–200.
    pub max_line: u16,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions { keyword_case: KeywordCase::Upper, indent: 2, comma: CommaStyle::Trailing, max_line: 120 }
    }
}

impl FormatOptions {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.indent, 2 | 4) {
            return Err(Error::Invalid("girinti 2 ya da 4 olmalı".into()));
        }
        if !(80..=200).contains(&self.max_line) {
            return Err(Error::Invalid("satır uzunluğu 80–200 olmalı".into()));
        }
        Ok(())
    }
}

/// `sql`'i biçimlendirir. Seçenekler geçersizse ya da güvenlik sözleşmesi (belirteç eşitliği + eşkuvvetlilik)
/// sağlanamazsa `Err(Invalid)`.
pub fn format(sql: &str, opts: &FormatOptions) -> Result<String> {
    opts.validate()?;
    if sql.len() > MAX_FORMAT_BYTES {
        return Err(Error::Invalid(FORMAT_TOO_LARGE.into()));
    }
    let out = format_unchecked(sql, opts);
    if !tokens_equivalent(sql, &out) || format_unchecked(&out, opts) != out {
        return Err(Error::Invalid(FORMAT_FAILED.into()));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------
// Anahtar sözcükler

/// Her bağlamda harfi değiştirilen sözcükler (PostgreSQL ayrılmış sözcükleri + yan tümce sözcükleri).
const ALWAYS: &[&str] = &[
    "all", "and", "any", "array", "as", "asc", "asymmetric", "between", "both", "case", "cast", "collate", "cross",
    "default", "delete", "desc", "distinct", "do", "else", "end", "except", "exists", "false", "fetch", "for", "from",
    "full", "group", "having", "ilike", "in", "inner", "insert", "intersect", "into", "is", "isnull", "join", "lateral",
    "leading", "like", "limit", "natural", "not", "notnull", "null", "offset", "on", "only", "or", "order", "outer",
    "returning", "select", "set", "similar", "some", "symmetric", "then", "to", "trailing", "true", "union", "update",
    "using", "values", "when", "where", "window", "with",
];

/// Yalnızca bağlamında harfi değiştirilen sözcükler (bkz. `Builder::recase_at`).
const CONTEXT: &[&str] = &[
    "by", "conflict", "filter", "first", "key", "last", "left", "locked", "materialized", "next", "no", "nothing",
    "nowait", "nulls", "over", "partition", "recursive", "right", "row", "rows", "share", "skip", "within",
];

/// Biçimlendiricinin harfini değiştirebileceği sözcük mü (belirteç karşılaştırmasında harfe duyarsızlık yalnız bunlara).
pub fn is_recasable(word: &str) -> bool {
    if word.len() > 12 || !word.is_ascii() {
        return false;
    }
    let w = word.to_ascii_lowercase();
    ALWAYS.contains(&w.as_str()) || CONTEXT.contains(&w.as_str())
}

// ---------------------------------------------------------------------------------------------------------------
// Güvenlik sözleşmesi

fn is_punct(t: &Token, sql: &str, set: &[&str]) -> bool {
    t.kind == Kind::Punct && set.contains(&t.text(sql))
}

/// Dize devamı için satır sonu: PostgreSQL'in sözcük çözümleyicisi yalın `\r`'yi de satır sonu sayar.
fn has_newline(gap: &str) -> bool {
    gap.contains(['\n', '\r'])
}

/// Belirteç dizileri eşdeğer mi: tür + metin (harfi değişebilen sözcükler harfe duyarsız) ve anlamlı bitişiklikler.
/// Bitişikliği serbest olanlar: `( ) , ;` komşulukları, yorumlar ve harfi değişebilen sözcükler.
pub fn tokens_equivalent(a: &str, b: &str) -> bool {
    let (ta, tb) = (lex(a), lex(b));
    if ta.len() != tb.len() {
        return false;
    }
    let mut prev_sig: Option<usize> = None;
    for i in 0..ta.len() {
        let (x, y) = (&ta[i], &tb[i]);
        if x.kind != y.kind {
            return false;
        }
        if x.kind != Kind::Comment {
            // Dize devamı: `'a'` + (satır sonu içeren boşluk) + `'b'` tek bir literaldir, yalnız boşlukla sözdizimi
            // hatasıdır. Ardışık iki dize arasındaki satır sonu varlığı iki tarafta aynı olmalı.
            if let Some(j) = prev_sig
                && x.kind == Kind::Str
                && ta[j].kind == Kind::Str
                && has_newline(&a[ta[j].end..x.start]) != has_newline(&b[tb[j].end..y.start])
            {
                return false;
            }
            prev_sig = Some(i);
        }
        let (xs, ys) = (x.text(a), y.text(b));
        let recasable = x.kind == Kind::Word && is_recasable(xs);
        if !(xs == ys || recasable && xs.eq_ignore_ascii_case(ys)) {
            return false;
        }
        if i > 0 {
            let p = &ta[i - 1];
            let free = |t: &Token, s: &str| {
                t.kind == Kind::Comment
                    || is_punct(t, s, &["(", ")", ",", ";"])
                    || t.kind == Kind::Word && is_recasable(t.text(s))
            };
            if !free(p, a) && !free(x, a) && (p.end == x.start) != (tb[i - 1].end == y.start) {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------------------------------------------
// Betik düzeni

fn is_line_comment(t: &Token, sql: &str) -> bool {
    t.kind == Kind::Comment && t.text(sql).starts_with("--")
}

fn sep(nl: usize, after_line_comment: bool) -> &'static str {
    if nl >= 2 {
        "\n\n"
    } else if nl == 1 || after_line_comment {
        "\n"
    } else {
        " "
    }
}

fn format_unchecked(sql: &str, opts: &FormatOptions) -> String {
    let toks = lex(sql);
    let Some(last_tok) = toks.last() else { return String::new() };
    let stmts = split(sql);
    let mut groups: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_stmt = false; // `cur` bir ifade içeriyor (ardından yalnız aynı satırdaki `;`/yorum eklenir)
    let mut line_end = false; // `cur` bir `--` yorumuyla bitiyor
    let mut prev_comment = false;
    let mut prev_end: Option<usize> = None;
    let (mut i, mut k) = (0usize, 0usize);
    while i < toks.len() {
        let t = toks[i];
        let nl = prev_end.map_or(0, |e| sql[e..t.start].matches('\n').count());
        if k < stmts.len() && t.start == stmts[k].start {
            let j = i + toks[i..].iter().take_while(|x| x.end <= stmts[k].end).count();
            let body = format_stmt(sql, &toks[i..j], opts);
            if has_stmt {
                groups.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push_str(if nl >= 2 { "\n\n" } else { "\n" });
            }
            cur.push_str(&body);
            has_stmt = true;
            line_end = is_line_comment(&toks[j - 1], sql);
            prev_comment = false;
            prev_end = Some(toks[j - 1].end);
            i = j;
            k += 1;
            continue;
        }
        let is_comment = t.kind == Kind::Comment;
        let attach = has_stmt && if is_comment { nl == 0 } else { !prev_comment };
        if attach {
            if line_end {
                cur.push('\n');
            } else if is_comment {
                cur.push(' ');
            }
        } else {
            if has_stmt {
                groups.push(std::mem::take(&mut cur));
                has_stmt = false;
            }
            if !cur.is_empty() {
                cur.push_str(sep(nl, line_end));
            }
        }
        cur.push_str(t.text(sql));
        line_end = is_line_comment(&t, sql);
        prev_comment = is_comment;
        prev_end = Some(t.end);
        i += 1;
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    let mut out = groups.join("\n\n");
    if sql[last_tok.end..].contains('\n') {
        out.push('\n');
    }
    out
}

fn format_stmt(sql: &str, toks: &[Token], opts: &FormatOptions) -> String {
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].kind != Kind::Comment).collect();
    let lead = sig.iter().map(|&i| toks[i]).find(|t| !(t.kind == Kind::Punct && t.text(sql) == "("));
    let known = lead.is_some_and(|t| {
        t.kind == Kind::Word
            && ["select", "with", "insert", "update", "delete", "values"].iter().any(|w| t.text(sql).eq_ignore_ascii_case(w))
    });
    if !known || too_deep(sql, toks) {
        return normalize(sql, toks);
    }
    let mut b = Builder::new(sql, toks, sig, opts);
    let n = b.sig.len();
    b.query(0, n);
    let doc = b.finish();
    let mut r = Renderer::new(opts.max_line as usize);
    for d in &doc {
        r.render(d, 0, false);
    }
    r.finish()
}

fn too_deep(sql: &str, toks: &[Token]) -> bool {
    let mut case = 0u32;
    for t in toks {
        if t.depth > MAX_DEPTH {
            return true;
        }
        if t.is_word(sql, "case") {
            case += 1;
            if case > MAX_DEPTH {
                return true;
            }
        } else if t.is_word(sql, "end") {
            case = case.saturating_sub(1);
        }
    }
    false
}

/// Tanınmayan ifade: satır sonları ve satır başı girintisi korunur, satır içi boşluk tek boşluğa iner.
fn normalize(sql: &str, toks: &[Token]) -> String {
    let mut s = String::new();
    for (n, t) in toks.iter().enumerate() {
        if n > 0 {
            let gap = &sql[toks[n - 1].end..t.start];
            if let Some(last_nl) = gap.rfind('\n') {
                for _ in 0..gap.matches('\n').count() {
                    s.push('\n');
                }
                s.extend(gap[last_nl + 1..].chars().filter(|c| matches!(c, ' ' | '\t')));
            } else if !gap.is_empty() {
                s.push(' ');
            }
        }
        s.push_str(t.text(sql));
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------
// Belge modeli ve yazıcı

enum Doc {
    Text(String),
    Space,
    /// Düz kipte `true` → boşluk, `false` → hiçbir şey; kırık kipte satır sonu.
    Line(bool),
    Hard,
    /// Sığarsa düz; düz genişlik (`None`: hiç düz olamaz).
    Group(Option<usize>, Vec<Doc>),
    /// İçindeki satır sonlarının girintisi: girişteki satırın girintisi + adım.
    Indent(usize, Option<usize>, Vec<Doc>),
    /// Her zaman kırık; içerik satır girintisi + adım, sonunda satır girintisinde satır sonu.
    Block(usize, Vec<Doc>),
}

fn width(d: &Doc) -> Option<usize> {
    match d {
        Doc::Text(s) => (!s.contains('\n')).then(|| s.chars().count()),
        Doc::Space => Some(1),
        Doc::Line(sp) => Some(*sp as usize),
        Doc::Hard | Doc::Block(..) => None,
        Doc::Group(w, _) | Doc::Indent(_, w, _) => *w,
    }
}

fn sum_width(items: &[Doc]) -> Option<usize> {
    items.iter().try_fold(0usize, |acc, d| width(d).map(|w| acc + w))
}

struct Renderer {
    out: String,
    col: usize,
    line_indent: usize,
    line_start: usize,
    at_start: bool,
    space: bool,
    max: usize,
}

impl Renderer {
    fn new(max: usize) -> Self {
        Renderer { out: String::new(), col: 0, line_indent: 0, line_start: 0, at_start: true, space: false, max }
    }

    fn newline(&mut self, indent: usize) {
        if self.at_start {
            self.out.truncate(self.line_start);
        } else {
            self.out.push('\n');
        }
        self.line_start = self.out.len();
        self.out.extend(std::iter::repeat_n(' ', indent));
        self.col = indent;
        self.line_indent = indent;
        self.at_start = true;
        self.space = false;
    }

    fn render(&mut self, d: &Doc, indent: usize, flat: bool) {
        match d {
            Doc::Text(s) => {
                if self.space && !self.at_start {
                    self.out.push(' ');
                    self.col += 1;
                }
                self.space = false;
                self.out.push_str(s);
                if let Some(nl) = s.rfind('\n') {
                    let last = &s[nl + 1..];
                    self.col = last.chars().count();
                    self.line_indent = last.len() - last.trim_start_matches(' ').len();
                    self.line_start = self.out.len() - last.len();
                } else {
                    self.col += s.chars().count();
                }
                self.at_start = false;
            }
            Doc::Space => self.space = true,
            Doc::Line(sp) => {
                if !flat {
                    self.newline(indent);
                } else if *sp {
                    self.space = true;
                }
            }
            Doc::Hard => self.newline(indent),
            Doc::Group(w, items) => {
                let fits = flat || w.is_some_and(|w| self.col + self.space as usize + w <= self.max);
                for it in items {
                    self.render(it, indent, fits);
                }
            }
            Doc::Indent(step, _, items) => {
                let ind = self.line_indent + step;
                for it in items {
                    self.render(it, ind, flat);
                }
            }
            Doc::Block(step, items) => {
                let base = self.line_indent;
                for it in items {
                    self.render(it, base + step, false);
                }
                self.newline(base);
            }
        }
    }

    fn finish(mut self) -> String {
        if self.at_start {
            self.out.truncate(self.line_start);
            if self.line_start > 0 && self.out.ends_with('\n') {
                self.out.pop();
            }
        }
        self.out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Yapı kurucu

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gap {
    /// Varsayılan kural (`default_gap`).
    Default,
    Space,
    Line,
    Soft,
    Hard,
}

enum Frame {
    Root,
    Group,
    Indent(usize),
    Block(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clause {
    Select,
    From,
    Where,
    GroupBy,
    Having,
    Window,
    OrderBy,
    Limit,
    Offset,
    Fetch,
    For,
    Returning,
    SetOp,
    Values,
    Set,
    OnConflict,
    Insert,
    Update,
    Delete,
    With,
    Using,
    Join,
}

const QUERY_HEADS: [&str; 6] = ["select", "with", "values", "insert", "update", "delete"];

struct Builder<'a> {
    sql: &'a str,
    toks: &'a [Token],
    /// Yorum olmayan belirteçlerin `toks` dizinleri; yapı işlemleri bu "sig" konumları üzerindendir.
    sig: Vec<usize>,
    /// `(` sig konumu → eşleşen `)` sig konumu.
    close: Vec<Option<usize>>,
    /// Küçük harfli sözcük (Word değilse ya da `.`'ya bitişikse boş).
    lw: Vec<String>,
    recase: Vec<bool>,
    opts: &'a FormatOptions,
    step: usize,
    frames: Vec<(Frame, Vec<Doc>)>,
    pending: Gap,
    force_hard: bool,
    /// Son `tok` çağrısı belirteci yeni satıra mı koydu.
    last_gap_hard: bool,
    /// Son yazılan belirtecin (yorum dahil) `toks` dizini.
    last: Option<usize>,
}

impl<'a> Builder<'a> {
    fn new(sql: &'a str, toks: &'a [Token], sig: Vec<usize>, opts: &'a FormatOptions) -> Self {
        let n = sig.len();
        let t = |p: usize| &toks[sig[p]];
        let mut close = vec![None; n];
        let mut stack = Vec::new();
        for p in 0..n {
            if is_punct(t(p), sql, &["("]) {
                stack.push(p);
            } else if is_punct(t(p), sql, &[")"])
                && let Some(o) = stack.pop()
            {
                close[o] = Some(p);
            }
        }
        // Nitelikli ad parçası (`t.from`, `t . select`): boşluktan bağımsız olarak anahtar sözcük sayılmaz.
        let dot_adjacent = |p: usize| {
            (p > 0 && is_punct(t(p - 1), sql, &["."])) || (p + 1 < n && is_punct(t(p + 1), sql, &["."]))
        };
        let lw: Vec<String> = (0..n)
            .map(|p| {
                if t(p).kind == Kind::Word && !dot_adjacent(p) { t(p).text(sql).to_ascii_lowercase() } else { String::new() }
            })
            .collect();
        let mut b = Builder {
            sql,
            toks,
            sig,
            close,
            lw,
            recase: Vec::new(),
            opts,
            step: opts.indent as usize,
            frames: vec![(Frame::Root, Vec::new())],
            pending: Gap::Default,
            force_hard: false,
            last_gap_hard: false,
            last: None,
        };
        b.recase = (0..n).map(|p| b.recase_at(p)).collect();
        b
    }

    fn w(&self, p: usize) -> &str {
        self.lw.get(p).map_or("", String::as_str)
    }

    fn text(&self, p: usize) -> &'a str {
        self.toks[self.sig[p]].text(self.sql)
    }

    fn recase_at(&self, p: usize) -> bool {
        let w = self.w(p);
        if w.is_empty() {
            return false;
        }
        // `set = 1`, `update = 3`: `=`'den önceki sözcük kolon adıdır (değişmez sabitler hariç).
        if p + 1 < self.sig.len() && self.text(p + 1) == "=" && !matches!(w, "true" | "false" | "null") {
            return false;
        }
        if ALWAYS.contains(&w) {
            return true;
        }
        let prev = if p > 0 { self.w(p - 1) } else { "" };
        let next = self.w(p + 1);
        match w {
            "left" | "right" => matches!(next, "join" | "outer"),
            "recursive" => prev == "with",
            "conflict" => prev == "on",
            "nothing" => prev == "do",
            "nulls" => matches!(next, "first" | "last"),
            "first" | "last" => prev == "nulls" || w == "first" && prev == "fetch",
            "next" => prev == "fetch",
            // FETCH FIRST n ROWS ONLY / OFFSET n ROWS
            "rows" | "row" => next == "only" || p >= 2 && matches!(self.w(p - 2), "first" | "next" | "offset"),
            "skip" => next == "locked",
            "locked" => prev == "skip",
            "nowait" => matches!(prev, "update" | "share"),
            "no" => next == "key" && prev == "for",
            "key" => prev == "no" && next == "update" || prev == "for" && next == "share",
            "share" => matches!(prev, "for" | "key"),
            "by" => matches!(prev, "group" | "order" | "partition"),
            "partition" => next == "by",
            "filter" => p > 0 && self.text(p - 1) == ")" && p + 1 < self.sig.len() && self.text(p + 1) == "(",
            "within" => next == "group",
            "over" => p > 0 && self.text(p - 1) == ")",
            "materialized" => matches!(prev, "as" | "not"),
            _ => false,
        }
    }

    fn is_open(&self, p: usize) -> bool {
        let t = &self.toks[self.sig[p]];
        t.kind == Kind::Punct && self.text(p) == "("
    }

    /// `(` konumundan sonra devam edilecek konum (eşleşmeyen parantez `hi`'ye kadar sürer).
    fn after_paren(&self, p: usize, hi: usize) -> usize {
        self.close[p].map_or(hi, |c| (c + 1).min(hi))
    }

    // --- belge yığını ---------------------------------------------------------------------------------------

    fn push(&mut self, d: Doc) {
        self.frames.last_mut().expect("kök çerçeve").1.push(d);
    }

    fn open(&mut self, f: Frame) {
        self.flush_trailing();
        self.frames.push((f, Vec::new()));
    }

    fn close(&mut self) {
        self.flush_trailing();
        let is_block = matches!(self.frames.last(), Some((Frame::Block(_), _)));
        if is_block {
            let next = self.next_sig_tok();
            self.flush_own_line(next);
        }
        let (f, items) = self.frames.pop().expect("açık çerçeve");
        let d = match f {
            Frame::Group => Doc::Group(sum_width(&items), items),
            Frame::Indent(s) => Doc::Indent(s, sum_width(&items), items),
            Frame::Block(s) => Doc::Block(s, items),
            Frame::Root => unreachable!("kök kapatılmaz"),
        };
        self.push(d);
        if is_block {
            // Blok kendi satır sonunu yazar; bekleyen boşluk/satır sonu geçersizdir.
            self.pending = Gap::Default;
            self.force_hard = false;
        }
    }

    fn next_sig_tok(&self) -> usize {
        let from = self.last.map_or(0, |l| l + 1);
        (from..self.toks.len()).find(|&i| self.toks[i].kind != Kind::Comment).unwrap_or(self.toks.len())
    }

    fn emit_gap(&mut self, g: Gap) {
        match g {
            Gap::Default => {}
            Gap::Space => self.push(Doc::Space),
            Gap::Line => self.push(Doc::Line(true)),
            Gap::Soft => self.push(Doc::Line(false)),
            Gap::Hard => self.push(Doc::Hard),
        }
    }

    /// Önceki belirteçle aynı satırda başlayan yorumlar (zincir halinde) önceki belirtecin ardına yazılır.
    fn flush_trailing(&mut self) {
        let Some(mut l) = self.last else { return };
        while l + 1 < self.toks.len() && self.toks[l + 1].kind == Kind::Comment {
            let c = self.toks[l + 1];
            if self.force_hard || self.sql[self.toks[l].end..c.start].contains('\n') {
                break;
            }
            self.push(Doc::Space);
            self.push(Doc::Text(c.text(self.sql).to_string()));
            l += 1;
            self.last = Some(l);
            if is_line_comment(&c, self.sql) {
                self.force_hard = true;
            }
        }
    }

    /// `upto`'dan (toks dizini) önceki kalan yorumlar: kendi satırında olan kendi satırına.
    fn flush_own_line(&mut self, upto: usize) {
        let from = self.last.map_or(0, |l| l + 1);
        let mut any = false;
        for i in from..upto.min(self.toks.len()) {
            let c = self.toks[i];
            if c.kind != Kind::Comment {
                break;
            }
            let newline_before = i > 0 && self.sql[self.toks[i - 1].end..c.start].contains('\n');
            self.emit_gap(if newline_before || self.force_hard { Gap::Hard } else { Gap::Space });
            self.push(Doc::Text(c.text(self.sql).to_string()));
            self.force_hard = is_line_comment(&c, self.sql);
            self.last = Some(i);
            any = true;
        }
        if any
            && let Some(l) = self.last
            && l + 1 < self.toks.len()
            && self.sql[self.toks[l].end..self.toks[l + 1].start].contains('\n')
        {
            self.force_hard = true;
        }
    }

    fn default_gap(&self, i: usize) -> Gap {
        if i == 0 {
            return Gap::Default;
        }
        let (prev, cur) = (&self.toks[i - 1], &self.toks[i]);
        if prev.kind == Kind::Comment {
            return Gap::Space;
        }
        let punct = |t: &Token, s: &str| t.kind == Kind::Punct && t.text(self.sql) == s;
        if punct(cur, ",") || punct(prev, "(") || punct(cur, ")") {
            return Gap::Default;
        }
        if punct(prev, ",") {
            return Gap::Space;
        }
        if prev.end == cur.start { Gap::Default } else { Gap::Space }
    }

    /// `i` bir dize ve kendisinden önceki yorum olmayan belirteç de dize ise: hemen önceki aralıkta satır sonu var mı
    /// (PostgreSQL dize devamı; bkz. `tokens_equivalent`).
    fn str_continuation(&self, i: usize) -> Option<bool> {
        if i == 0 || self.toks[i].kind != Kind::Str {
            return None;
        }
        let j = (0..i).rev().find(|&j| self.toks[j].kind != Kind::Comment)?;
        (self.toks[j].kind == Kind::Str).then(|| has_newline(&self.sql[self.toks[i - 1].end..self.toks[i].start]))
    }

    /// Sig konumu `p`'deki belirteci yazar (öncesindeki yorumlar ve bekleyen aralıkla).
    fn tok(&mut self, p: usize) {
        let i = self.sig[p];
        self.flush_trailing();
        self.flush_own_line(i);
        let mut g = if self.force_hard {
            Gap::Hard
        } else if self.pending == Gap::Default {
            self.default_gap(i)
        } else {
            self.pending
        };
        match self.str_continuation(i) {
            Some(true) => g = Gap::Hard,
            Some(false) if matches!(g, Gap::Line | Gap::Soft) => g = Gap::Space,
            _ => {}
        }
        self.last_gap_hard = g == Gap::Hard;
        self.emit_gap(g);
        let raw = self.text(p);
        let text = if self.recase[p] {
            match self.opts.keyword_case {
                KeywordCase::Upper => raw.to_ascii_uppercase(),
                KeywordCase::Lower => raw.to_ascii_lowercase(),
                KeywordCase::Preserve => raw.to_string(),
            }
        } else {
            raw.to_string()
        };
        self.push(Doc::Text(text));
        self.last = Some(i);
        self.pending = Gap::Default;
        self.force_hard = false;
    }

    fn finish(mut self) -> Vec<Doc> {
        self.flush_trailing();
        while self.frames.len() > 1 {
            self.close();
        }
        let n = self.toks.len();
        self.flush_own_line(n);
        self.frames.pop().map(|f| f.1).unwrap_or_default()
    }

    // --- sorgu düzeyi ---------------------------------------------------------------------------------------

    fn clause_at(&self, p: usize, lo: usize, hi: usize, cur: Option<Clause>) -> Option<(Clause, usize)> {
        let w = self.w(p);
        if w.is_empty() || p + 1 < hi && self.text(p + 1) == "=" {
            // `SET set = 1`, `WHERE update = 3`: anahtar sözcük adlı kolon, yan tümce başı değil.
            return None;
        }
        let prev = if p > lo { self.w(p - 1) } else { "" };
        let next = if p + 1 < hi { self.w(p + 1) } else { "" };
        let c = match w {
            "select" => (Clause::Select, 1),
            "from" if prev != "distinct" => (Clause::From, 1),
            "where" => (Clause::Where, 1),
            "group" if next == "by" => (Clause::GroupBy, 2),
            "having" => (Clause::Having, 1),
            "window" => (Clause::Window, 1),
            "order" if next == "by" => (Clause::OrderBy, 2),
            "limit" => (Clause::Limit, 1),
            "offset" => (Clause::Offset, 1),
            "fetch" => (Clause::Fetch, 1),
            "for" if matches!(next, "update" | "share" | "no" | "key") => (Clause::For, 1),
            "returning" => (Clause::Returning, 1),
            "union" | "intersect" | "except" => (Clause::SetOp, if matches!(next, "all" | "distinct") { 2 } else { 1 }),
            "values" if prev != "default" => (Clause::Values, 1),
            "set" => (Clause::Set, 1),
            "on" if next == "conflict" => (Clause::OnConflict, 2),
            "insert" | "update" | "delete"
                if !matches!(prev, "do" | "for" | "key" | "no" | "on" | "before" | "after" | "or" | "of" | "instead") =>
            {
                let c = match w {
                    "insert" => Clause::Insert,
                    "update" => Clause::Update,
                    _ => Clause::Delete,
                };
                (c, if matches!((w, next), ("insert", "into") | ("delete", "from")) { 2 } else { 1 })
            }
            "with" if p == lo => (Clause::With, if next == "recursive" { 2 } else { 1 }),
            "using" if cur == Some(Clause::Delete) => (Clause::Using, 1),
            _ => {
                // [NATURAL] [LEFT|RIGHT|FULL [OUTER] | INNER | CROSS] JOIN
                let mut q = p;
                if self.w(q) == "natural" {
                    q += 1;
                }
                match self.w(q) {
                    "left" | "right" | "full" => {
                        q += 1;
                        if self.w(q) == "outer" {
                            q += 1;
                        }
                    }
                    "inner" | "cross" => q += 1,
                    _ => {}
                }
                if q < hi && self.w(q) == "join" {
                    (Clause::Join, q - p + 1)
                } else {
                    return None;
                }
            }
        };
        Some(c)
    }

    /// `[lo, hi)` aynı parantez düzeyinde bir sorgu: yan tümcelere bölünür.
    fn query(&mut self, lo: usize, hi: usize) {
        let mut clauses: Vec<(usize, Clause, usize)> = Vec::new();
        let (mut p, mut cur) = (lo, None);
        while p < hi {
            if self.is_open(p) {
                p = self.after_paren(p, hi);
                continue;
            }
            if let Some((c, n)) = self.clause_at(p, lo, hi, cur) {
                clauses.push((p, c, n));
                cur = Some(c);
                p += n;
            } else {
                p += 1;
            }
        }
        let first = clauses.first().map_or(hi, |c| c.0);
        if first > lo {
            self.generic(lo, first, false);
        }
        for k in 0..clauses.len() {
            let (s, c, n) = clauses[k];
            let e = clauses.get(k + 1).map_or(hi, |c| c.0);
            self.clause(s, n, e, c);
        }
    }

    fn clause(&mut self, s: usize, n: usize, e: usize, c: Clause) {
        self.pending = Gap::Hard;
        for q in s..s + n {
            self.tok(q);
        }
        let mut b = s + n;
        match c {
            Clause::Select => {
                if b < e && self.w(b) == "distinct" {
                    self.tok(b);
                    b += 1;
                    if b + 1 < e && self.w(b) == "on" && self.is_open(b + 1) {
                        self.tok(b);
                        let end = self.after_paren(b + 1, e);
                        self.generic(b + 1, end, false);
                        b = end;
                    }
                } else if b < e && self.w(b) == "all" {
                    self.tok(b);
                    b += 1;
                }
                self.list(b, e);
            }
            Clause::From
            | Clause::GroupBy
            | Clause::OrderBy
            | Clause::Returning
            | Clause::Set
            | Clause::Values
            | Clause::Window => self.list(b, e),
            Clause::Where | Clause::Having => self.cond(b, e),
            Clause::Join => {
                let on = self.find_word(b, e, "on");
                match on {
                    Some(q) => {
                        self.generic(b, q, false);
                        self.tok(q);
                        self.cond(q + 1, e);
                    }
                    None => self.generic(b, e, false),
                }
            }
            Clause::With => {
                let (items, commas) = self.split_commas(b, e);
                for (k, &(is, ie)) in items.iter().enumerate() {
                    if k > 0 {
                        self.tok(commas[k - 1]);
                        self.pending = Gap::Hard;
                    }
                    self.generic(is, ie, false);
                }
            }
            Clause::SetOp => {
                if b < e {
                    self.pending = Gap::Hard;
                }
                self.generic(b, e, false);
            }
            _ => self.generic(b, e, false),
        }
    }

    /// Aynı düzeyde (parantez dışında) `word`'ün ilk konumu.
    fn find_word(&self, a: usize, b: usize, word: &str) -> Option<usize> {
        let mut p = a;
        while p < b {
            if self.is_open(p) {
                p = self.after_paren(p, b);
                continue;
            }
            if self.w(p) == word {
                return Some(p);
            }
            p += 1;
        }
        None
    }

    /// Aynı düzeydeki virgüllere göre öğeler ve virgül konumları.
    fn split_commas(&self, a: usize, b: usize) -> (Vec<(usize, usize)>, Vec<usize>) {
        let (mut items, mut commas, mut start, mut p) = (Vec::new(), Vec::new(), a, a);
        while p < b {
            if self.is_open(p) {
                p = self.after_paren(p, b);
                continue;
            }
            if self.toks[self.sig[p]].kind == Kind::Punct && self.text(p) == "," {
                items.push((start, p));
                commas.push(p);
                start = p + 1;
            }
            p += 1;
        }
        items.push((start, b));
        (items, commas)
    }

    fn list(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        let (items, commas) = self.split_commas(a, b);
        if items.len() == 1 {
            self.generic(a, b, false);
            return;
        }
        self.open(Frame::Group);
        self.open(Frame::Indent(self.step));
        self.pending = Gap::Line;
        for (k, &(s, e)) in items.iter().enumerate() {
            if k > 0 {
                let c = commas[k - 1];
                match self.opts.comma {
                    // Virgül bir yorum yüzünden satır başına düştüyse (`a -- c` / `, b`) öğe aynı satırda sürer;
                    // virgül yalnız kalmaz.
                    CommaStyle::Trailing => {
                        self.tok(c);
                        self.pending = if self.last_gap_hard { Gap::Space } else { Gap::Line };
                    }
                    // Virgülün ardındaki aynı satır yorumu (`a, -- c`) virgülle birlikte öğenin satırında kalır.
                    CommaStyle::Leading if self.comment_follows_on_same_line(c) => {
                        self.tok(c);
                        self.pending = Gap::Line;
                    }
                    CommaStyle::Leading => {
                        self.pending = Gap::Soft;
                        self.tok(c);
                        self.pending = Gap::Space;
                    }
                }
            }
            self.generic(s, e, false);
        }
        self.close();
        self.close();
    }

    fn comment_follows_on_same_line(&self, p: usize) -> bool {
        let i = self.sig[p];
        i + 1 < self.toks.len()
            && self.toks[i + 1].kind == Kind::Comment
            && !self.sql[self.toks[i].end..self.toks[i + 1].start].contains('\n')
    }

    fn cond(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        self.open(Frame::Group);
        self.open(Frame::Indent(self.step));
        self.generic(a, b, true);
        self.close();
        self.close();
    }

    fn is_query_paren(&self, a: usize, b: usize) -> bool {
        a < b && QUERY_HEADS.contains(&self.w(a))
    }

    /// Satır içi ifade; `and_or` ise bu düzeydeki `AND`/`OR` öncesi kırılabilir satır.
    fn generic(&mut self, a: usize, b: usize, and_or: bool) {
        let mut p = a;
        let mut between = false;
        while p < b {
            if self.is_open(p) {
                let closed = self.close[p].filter(|&c| c < b);
                let inner_end = closed.unwrap_or(b);
                self.tok(p);
                if self.is_query_paren(p + 1, inner_end) {
                    self.open(Frame::Block(self.step));
                    self.pending = Gap::Hard;
                    self.query(p + 1, inner_end);
                    self.close();
                } else {
                    self.generic(p + 1, inner_end, false);
                }
                if let Some(c) = closed {
                    self.tok(c);
                    p = c + 1;
                } else {
                    p = b;
                }
                continue;
            }
            let w = self.w(p);
            if w == "case"
                && let Some(end) = self.case_end(p, b)
            {
                self.case(p, end);
                p = end + 1;
                continue;
            }
            if and_or && (w == "and" || w == "or") {
                if w == "and" && between {
                    between = false;
                } else {
                    self.pending = Gap::Line;
                }
            } else if w == "between" {
                between = true;
            }
            self.tok(p);
            p += 1;
        }
    }

    /// Bu düzeyde `CASE`'i kapatan `END`.
    fn case_end(&self, p: usize, b: usize) -> Option<usize> {
        let (mut q, mut depth) = (p + 1, 0u32);
        while q < b {
            if self.is_open(q) {
                q = self.after_paren(q, b);
                continue;
            }
            match self.w(q) {
                "case" => depth += 1,
                "end" if depth == 0 => return Some(q),
                "end" => depth -= 1,
                _ => {}
            }
            q += 1;
        }
        None
    }

    fn case(&mut self, p: usize, end: usize) {
        // Bu CASE'e ait WHEN/ELSE konumları (iç içe CASE…END ve parantezler atlanır).
        let mut parts = Vec::new();
        let mut q = p + 1;
        while q < end {
            if self.is_open(q) {
                q = self.after_paren(q, end);
                continue;
            }
            match self.w(q) {
                "case" => {
                    q = self.case_end(q, end).map_or(end, |e| e + 1);
                    continue;
                }
                "when" | "else" => parts.push(q),
                _ => {}
            }
            q += 1;
        }
        self.tok(p);
        self.open(Frame::Group);
        self.open(Frame::Indent(0));
        self.open(Frame::Indent(self.step));
        let first = parts.first().copied().unwrap_or(end);
        self.generic(p + 1, first, false);
        for (k, &s) in parts.iter().enumerate() {
            let e = parts.get(k + 1).copied().unwrap_or(end);
            self.pending = Gap::Line;
            self.tok(s);
            self.generic(s + 1, e, false);
        }
        self.close();
        self.pending = Gap::Line;
        self.tok(end);
        self.close();
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(sql: &str) -> String {
        format(sql, &FormatOptions::default()).unwrap()
    }

    #[test]
    fn simple_select() {
        assert_eq!(f("select 1"), "SELECT 1");
        assert_eq!(f("select a, b from t where x = 1"), "SELECT a, b\nFROM t\nWHERE x = 1");
    }

    #[test]
    fn options_are_validated() {
        let bad = [
            FormatOptions { indent: 3, ..Default::default() },
            FormatOptions { max_line: 79, ..Default::default() },
            FormatOptions { max_line: 201, ..Default::default() },
        ];
        for o in bad {
            assert!(matches!(format("select 1", &o), Err(Error::Invalid(_))));
        }
    }

    #[test]
    fn identifiers_that_look_like_keywords_keep_their_case() {
        assert_eq!(f("select name, type, value, data, first, last from t"), "SELECT name, type, value, data, first, last\nFROM t");
        assert_eq!(f("select t.from from t"), "SELECT t.from\nFROM t");
        assert_eq!(f("select left(a, 2) from t left join u on true"), "SELECT left(a, 2)\nFROM t\nLEFT JOIN u ON TRUE");
    }

    #[test]
    fn equivalence_check_catches_corruption() {
        assert!(tokens_equivalent("select a,b from t", "SELECT a, b\nFROM t"));
        assert!(tokens_equivalent("select (1)", "select ( 1 )"));
        assert!(tokens_equivalent("select 'a'\n'b'", "SELECT 'a'\n  'b'"));
        assert!(tokens_equivalent("select 'a'\r'b'", "SELECT 'a'\n  'b'"));
        for (a, b) in [
            ("select name", "select NAME"),           // tanımlayıcı harfi
            ("select 'a  b'", "select 'a b'"),        // dize içi
            ("select a-b", "select a - b"),           // operatör bitişikliği ayrıldı
            ("select x::int", "select x: :int"),      // `::` bölündü
            ("select a - -b", "select a --b"),        // yorum oluştu
            ("select B'01'", "select B '01'"),        // bit dizesi öneki
            ("select 1 -- c\n, 2", "select 1 -- c, 2"), // satır yorumu kodu yuttu
            ("select $1", "select $ 1"),
            ("select 1.5", "select 1 . 5"),
            ("select /* a */ 1", "select /* A */ 1"),
            ("select 'a'\n'b'", "select 'a' 'b'"),    // dize devamı birleşti → sözdizimi hatası
            ("select 'a' 'b'", "select 'a'\n'b'"),    // sözdizimi hatası sessizce devama döndü
            ("select E'a'\n'b'", "select E'a' 'b'"),  // önekli dize devamı
            ("select 'a'\r'b'", "select 'a' 'b'"),    // yalın CR de satır sonudur
        ] {
            assert!(!tokens_equivalent(a, b), "{a:?} ≈ {b:?}");
        }
    }

    #[test]
    fn empty_and_comment_only() {
        assert_eq!(f(""), "");
        assert_eq!(f("   \n"), "");
        assert_eq!(f("-- a\n\n\n/* b */  -- c\n"), "-- a\n\n/* b */ -- c\n");
    }
}
