//! Sözcüksel kapsam çözümleyici: imlecin bulunduğu sorgu bloğunda görünür ilişki öğelerini
//! (tablo, CTE, alt sorgu, fonksiyon kaynağı) bulur. Tam bir ayrıştırıcı değildir; bozuk
//! sözdiziminde o ana kadar toplanan öğeler döner, hiçbir girdide panik etmez.
//! Tüm ofsetler UTF-8 bayt ofsetidir (çağıran Unicode scalar ofsetinden dönüştürür).
//! Yalnızca imleci içeren ifadeye bakılır: derinlik 0'daki `;` ifade sınırıdır.
//! SQL metni günlüğe yazılmaz.
//!
//! Tanımlayıcı katlama: tırnaksız adlar yalnızca ASCII A-Z için küçük harfe katlanır
//! (PostgreSQL `downcase_identifier`, UTF-8 veritabanında çok baytlı harfleri katlamaz);
//! tırnaklı adlarda tırnaklar atılır ve `""` tek `"` olur, büyük/küçük harf korunur.

use std::ops::Range;

use crate::lex::{lex, Kind, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeItem {
    /// `columns`: alias kolon listesi (`t x(a, b)`); PostgreSQL gibi katalog kolonlarının ilk N'ini yeniden adlandırır.
    Relation { alias: Option<String>, schema: Option<String>, name: String, columns: Option<Vec<String>> },
    Cte { name: String, body: Range<usize>, columns: Option<Vec<String>> },
    Subquery { alias: String, body: Range<usize>, columns: Option<Vec<String>> },
    /// `columns`: alias kolon listesi (`unnest(a) as u(v, n)`, `f() as x(a int, b text)` → `a`, `b`).
    Function { alias: Option<String>, name: String, columns: Option<Vec<String>> },
}

impl ScopeItem {
    fn alias(&self) -> Option<&str> {
        match self {
            ScopeItem::Relation { alias, .. } | ScopeItem::Function { alias, .. } => alias.as_deref(),
            ScopeItem::Subquery { alias, .. } => Some(alias),
            ScopeItem::Cte { .. } => None,
        }
    }

    fn name(&self) -> Option<&str> {
        match self {
            ScopeItem::Relation { name, .. }
            | ScopeItem::Function { name, .. }
            | ScopeItem::Cte { name, .. } => Some(name),
            ScopeItem::Subquery { .. } => None,
        }
    }
}

/// Belirteç metnini (tırnaklı ya da tırnaksız) karşılaştırma biçimine çevirir.
pub fn ident_norm(text: &str) -> String {
    match text.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"').unwrap_or(rest).replace("\"\"", "\""),
        None => text.to_ascii_lowercase(),
    }
}

/// `qualifier` (ham belirteç metni) ile eşleşen öğe: önce alias, yoksa ad.
pub fn resolve<'a>(items: &'a [ScopeItem], qualifier: &str) -> Option<&'a ScopeItem> {
    let q = ident_norm(qualifier);
    items
        .iter()
        .find(|i| i.alias() == Some(q.as_str()))
        .or_else(|| items.iter().find(|i| i.name() == Some(q.as_str())))
}

/// Alias olamayacak anahtar sözcükler (küçük harfle): PostgreSQL'in ayrılmış ve `type_func_name` sözcükleri
/// (AS'siz tablo alias'ı olamazlar) ile ilişki adından sonra bağlamsal olarak gelen birkaç ayrılmamış sözcük
/// (`set`, `values`, `overriding`, `ordinality`, `repeatable`, `conflict`).
fn is_kw(lower: &str) -> bool {
    matches!(
        lower,
        // ayrılmış
        "all" | "analyse" | "analyze" | "and" | "any" | "array" | "as" | "asc" | "asymmetric" | "both"
            | "case" | "cast" | "check" | "collate" | "column" | "constraint" | "create" | "current_catalog"
            | "current_date" | "current_role" | "current_time" | "current_timestamp" | "current_user"
            | "default" | "deferrable" | "desc" | "distinct" | "do" | "else" | "end" | "except" | "false"
            | "fetch" | "for" | "foreign" | "from" | "grant" | "group" | "having" | "in" | "initially"
            | "intersect" | "into" | "lateral" | "leading" | "limit" | "localtime" | "localtimestamp"
            | "not" | "null" | "offset" | "on" | "only" | "or" | "order" | "placing" | "primary"
            | "references" | "returning" | "select" | "session_user" | "some" | "symmetric"
            | "system_user" | "table" | "then" | "to" | "trailing" | "true" | "union" | "unique" | "user"
            | "using" | "variadic" | "when" | "where" | "window" | "with"
            // type_func_name
            | "authorization" | "binary" | "collation" | "concurrently" | "cross" | "current_schema"
            | "freeze" | "full" | "ilike" | "inner" | "is" | "isnull" | "join" | "left" | "like"
            | "natural" | "notnull" | "outer" | "overlaps" | "right" | "similar" | "tablesample"
            | "verbose"
            // bağlamsal
            | "set" | "values" | "overriding" | "ordinality" | "repeatable" | "conflict" | "between"
    )
}

/// Bir düzeydeki öğe: belirteç ya da (dengeli) parantez grubu.
#[derive(Clone, Copy)]
enum El {
    Tok(usize),
    /// `close` kapanış belirteci dizini; kapanmamışsa düzeyin sonu.
    Group { open: usize, close: usize },
}

struct Cx<'a> {
    sql: &'a str,
    toks: Vec<Token>,
}

impl Cx<'_> {
    fn tok(&self, e: &El) -> Option<&Token> {
        match e {
            El::Tok(i) => self.toks.get(*i),
            El::Group { .. } => None,
        }
    }

    fn word_is(&self, e: &El, w: &str) -> bool {
        self.tok(e).is_some_and(|t| t.is_word(self.sql, w))
    }

    fn punct_is(&self, e: &El, c: &str) -> bool {
        self.tok(e).is_some_and(|t| t.kind == Kind::Punct && t.text(self.sql) == c)
    }

    /// Ad belirteci: tırnaklı ad ya da rakamla başlamayan sözcük.
    fn is_name(&self, t: &Token) -> bool {
        match t.kind {
            Kind::Ident => true,
            Kind::Word => !t.text(self.sql).as_bytes()[0].is_ascii_digit(),
            _ => false,
        }
    }

    fn name_at(&self, els: &[El], p: usize) -> Option<(String, &Token)> {
        let t = self.tok(els.get(p)?)?;
        self.is_name(t).then(|| (ident_norm(t.text(self.sql)), t))
    }

    fn stmt_start(&self, prev: Option<&El>) -> bool {
        match prev {
            None | Some(El::Group { .. }) => true,
            Some(e) => self.punct_is(e, ";"),
        }
    }

    fn elems(&self, lo: usize, hi: usize) -> Vec<El> {
        let hi = hi.min(self.toks.len());
        let mut out = Vec::new();
        let mut i = lo;
        while i < hi {
            let t = &self.toks[i];
            if t.kind == Kind::Punct && t.text(self.sql) == "(" {
                let close = self.matching_close(i, hi);
                out.push(El::Group { open: i, close });
                i = close + 1;
            } else {
                out.push(El::Tok(i));
                i += 1;
            }
        }
        out
    }

    /// `(` belirteci `open` için eşleşen `)` dizini; yoksa `hi`.
    fn matching_close(&self, open: usize, hi: usize) -> usize {
        let d = self.toks[open].depth;
        (open + 1..hi.min(self.toks.len()))
            .find(|&j| {
                let t = &self.toks[j];
                t.kind == Kind::Punct && t.depth == d && t.text(self.sql) == ")"
            })
            .unwrap_or(hi)
    }

    fn body(&self, open: usize, close: usize) -> Range<usize> {
        let start = self.toks[open].end;
        let end = self.toks.get(close).map_or(self.sql.len(), |t| t.start);
        start..end.max(start)
    }

    /// Parantez içindeki kolon adları: her virgülden sonraki ilk ad (`(a int, b text)` → `a`, `b`).
    fn group_cols(&self, open: usize, close: usize) -> Vec<String> {
        let d = self.toks[open].depth + 1;
        let mut expect = true;
        let mut out = Vec::new();
        for t in self.toks.iter().take(close.min(self.toks.len())).skip(open + 1).filter(|t| t.depth == d) {
            if t.kind == Kind::Punct && t.text(self.sql) == "," {
                expect = true;
            } else if expect {
                if self.is_name(t) {
                    out.push(ident_norm(t.text(self.sql)));
                }
                expect = false;
            }
        }
        out
    }

    fn group_is_query(&self, open: usize) -> bool {
        self.toks.get(open + 1).is_some_and(|t| {
            ["select", "with", "values", "table"].iter().any(|w| t.is_word(self.sql, w))
        })
    }

    /// `[AS] alias [(kolonlar)]`; (alias, kolonlar, sonraki konum).
    fn parse_alias(&self, els: &[El], mut p: usize) -> (Option<String>, Option<Vec<String>>, usize) {
        let mut alias = None;
        if els.get(p).is_some_and(|e| self.word_is(e, "as")) {
            if let Some((a, _)) = self.name_at(els, p + 1) {
                alias = Some(a);
                p += 2;
            } else {
                return (None, None, p + 1);
            }
        } else if let Some((a, t)) = self.name_at(els, p) {
            let kw = t.kind == Kind::Word && is_kw(&t.text(self.sql).to_ascii_lowercase());
            if !kw {
                alias = Some(a);
                p += 1;
            }
        }
        let mut cols = None;
        if let (Some(_), Some(El::Group { open, close })) = (&alias, els.get(p)) {
            cols = Some(self.group_cols(*open, *close));
            p += 1;
        }
        (alias, cols, p)
    }

    /// `FROM`/`JOIN`/... ardından tek öğe; sonraki konumu döner.
    fn parse_item(&self, els: &[El], mut p: usize, allow_fn: bool, out: &mut Vec<ScopeItem>) -> usize {
        while els.get(p).is_some_and(|e| self.word_is(e, "lateral") || self.word_is(e, "only")) {
            p += 1;
        }
        match els.get(p) {
            Some(El::Group { open, close }) => {
                if self.group_is_query(*open) {
                    let body = self.body(*open, *close);
                    let (alias, columns, np) = self.parse_alias(els, p + 1);
                    if let Some(alias) = alias {
                        out.push(ScopeItem::Subquery { alias, body, columns });
                    }
                    np
                } else {
                    // Parantezli birleşim: `from (a join b on ...)`.
                    let inner = self.elems(open + 1, *close);
                    self.scan(&inner, true, out);
                    p + 1
                }
            }
            Some(El::Tok(i)) => {
                let t = &self.toks[*i];
                // `ROWS FROM (f(...), g(...)) [WITH ORDINALITY] [AS] alias [(kolonlar)]`
                if allow_fn
                    && t.is_word(self.sql, "rows")
                    && els.get(p + 1).is_some_and(|e| self.word_is(e, "from"))
                    && let Some(El::Group { open, .. }) = els.get(p + 2)
                {
                    let name = self
                        .toks
                        .get(open + 1)
                        .filter(|f| self.is_name(f))
                        .map_or_else(|| "rows".to_string(), |f| ident_norm(f.text(self.sql)));
                    let mut q = p + 3;
                    if els.get(q).is_some_and(|e| self.word_is(e, "with"))
                        && els.get(q + 1).is_some_and(|e| self.word_is(e, "ordinality"))
                    {
                        q += 2;
                    }
                    let (alias, columns, nq) = self.parse_alias(els, q);
                    out.push(ScopeItem::Function { alias, name, columns });
                    return nq;
                }
                if !self.is_name(t) || (t.kind == Kind::Word && is_kw(&t.text(self.sql).to_ascii_lowercase())) {
                    return p;
                }
                let mut parts = vec![ident_norm(t.text(self.sql))];
                let mut q = p + 1;
                while els.get(q).is_some_and(|e| self.punct_is(e, ".")) {
                    match self.name_at(els, q + 1) {
                        Some((n, _)) => {
                            parts.push(n);
                            q += 2;
                        }
                        None => break,
                    }
                }
                let name = parts.pop().unwrap_or_default();
                if allow_fn && matches!(els.get(q), Some(El::Group { .. })) {
                    q += 1;
                    if els.get(q).is_some_and(|e| self.word_is(e, "with"))
                        && els.get(q + 1).is_some_and(|e| self.word_is(e, "ordinality"))
                    {
                        q += 2;
                    }
                    let (alias, columns, nq) = self.parse_alias(els, q);
                    out.push(ScopeItem::Function { alias, name, columns });
                    return nq;
                }
                // Kalıtım yıldızı: `from t * x`.
                if els.get(q).is_some_and(|e| self.punct_is(e, "*")) {
                    q += 1;
                }
                let (mut alias, mut columns, mut nq) = self.parse_alias(els, q);
                // `TABLESAMPLE yöntem (argümanlar) [REPEATABLE (tohum)]` alias'tan sonra gelir; yanlış sırada
                // (`t tablesample system (1) s`) yazılmış alias da kabul edilir.
                if els.get(nq).is_some_and(|e| self.word_is(e, "tablesample")) {
                    nq += 1;
                    if self.name_at(els, nq).is_some() {
                        nq += 1;
                    }
                    if matches!(els.get(nq), Some(El::Group { .. })) {
                        nq += 1;
                    }
                    if els.get(nq).is_some_and(|e| self.word_is(e, "repeatable"))
                        && matches!(els.get(nq + 1), Some(El::Group { .. }))
                    {
                        nq += 2;
                    }
                    if alias.is_none() {
                        (alias, columns, nq) = self.parse_alias(els, nq);
                    }
                }
                out.push(ScopeItem::Relation { alias, schema: parts.pop(), name, columns });
                nq
            }
            None => p,
        }
    }

    /// Bir sorgu bloğu parçasındaki (küme işleci sınırları arası) ilişki öğeleri.
    fn scan(&self, els: &[El], nested: bool, out: &mut Vec<ScopeItem>) {
        let del_merge = els.iter().enumerate().any(|(i, e)| {
            (self.word_is(e, "delete") || self.word_is(e, "merge"))
                && self.stmt_start(i.checked_sub(1).map(|j| &els[j]))
        });
        let mut in_from = nested;
        let mut i = if nested { self.parse_item(els, 0, true, out) } else { 0 };
        while i < els.len() {
            let prev = i.checked_sub(1).map(|j| &els[j]);
            let next = match self.tok(&els[i]) {
                Some(t) if t.kind == Kind::Word => {
                    match t.text(self.sql).to_ascii_lowercase().as_str() {
                        "from" if !prev.is_some_and(|p| self.word_is(p, "distinct")) => {
                            in_from = true;
                            self.parse_item(els, i + 1, true, out)
                        }
                        "join" => self.parse_item(els, i + 1, true, out),
                        "using" if del_merge && !matches!(els.get(i + 1), Some(El::Group { .. })) => {
                            in_from = true;
                            self.parse_item(els, i + 1, true, out)
                        }
                        "update" if self.stmt_start(prev) => {
                            in_from = false;
                            self.parse_item(els, i + 1, false, out)
                        }
                        "into" if prev.is_some_and(|p| self.word_is(p, "insert") || self.word_is(p, "merge")) => {
                            self.parse_item(els, i + 1, false, out)
                        }
                        "where" | "group" | "order" | "having" | "limit" | "offset" | "window" | "fetch"
                        | "for" | "returning" | "set" | "values" | "select" => {
                            in_from = false;
                            i + 1
                        }
                        _ => i + 1,
                    }
                }
                Some(_) if in_from && self.punct_is(&els[i], ",") => self.parse_item(els, i + 1, true, out),
                _ => i + 1,
            };
            i = next.max(i + 1);
        }
    }

    /// `WITH [RECURSIVE] ad [(kolonlar)] AS [[NOT] MATERIALIZED] (gövde) [, ...]` CTE'leri.
    fn parse_ctes(&self, els: &[El], out: &mut Vec<ScopeItem>) {
        if !els.first().is_some_and(|e| self.word_is(e, "with")) {
            return;
        }
        let mut p = 1;
        if els.get(p).is_some_and(|e| self.word_is(e, "recursive")) {
            p += 1;
        }
        loop {
            let Some((name, _)) = self.name_at(els, p) else { return };
            p += 1;
            let mut columns = None;
            if let Some(El::Group { open, close }) = els.get(p) {
                columns = Some(self.group_cols(*open, *close));
                p += 1;
            }
            if !els.get(p).is_some_and(|e| self.word_is(e, "as")) {
                return;
            }
            p += 1;
            while els.get(p).is_some_and(|e| self.word_is(e, "not") || self.word_is(e, "materialized")) {
                p += 1;
            }
            let Some(El::Group { open, close }) = els.get(p) else { return };
            out.push(ScopeItem::Cte { name, body: self.body(*open, *close), columns });
            p += 1;
            if els.get(p).is_some_and(|e| self.punct_is(e, ",")) {
                p += 1;
            } else {
                return;
            }
        }
    }
}

/// İmlecin (`offset`, bayt) bulunduğu bloğun görünür öğeleri: iç blok önce, sonra dış bloklar,
/// en sonda (iç→dış) CTE'ler.
pub fn scope_at(sql: &str, offset: usize) -> Vec<ScopeItem> {
    let toks: Vec<Token> = lex(sql).into_iter().filter(|t| t.kind != Kind::Comment).collect();
    let cx = Cx { sql, toks };

    // İmleci içeren ifade: derinlik 0'daki `;` belirteçleri arası [stmt_lo, n).
    let is_semi = |t: &Token| t.kind == Kind::Punct && t.depth == 0 && t.text(sql) == ";";
    let stmt_lo = cx.toks.iter().rposition(|t| is_semi(t) && t.end <= offset).map_or(0, |i| i + 1);
    let n = cx.toks.iter().skip(stmt_lo).position(is_semi).map_or(cx.toks.len(), |i| stmt_lo + i);

    // İmleci çevreleyen açık parantezler (dıştan içe).
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in cx.toks.iter().enumerate().take(n).skip(stmt_lo) {
        if t.start >= offset {
            break;
        }
        if t.kind == Kind::Punct && t.end <= offset {
            match t.text(sql) {
                "(" => stack.push(i),
                ")" => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }

    // Düzeyler (dıştan içe): (token aralığı, imleç/alt grup konumu).
    let mut levels: Vec<(usize, usize, usize)> = Vec::new();
    let first_ref = stack.first().map_or(offset, |&o| cx.toks[o].start);
    levels.push((stmt_lo, n, first_ref));
    for (k, &open) in stack.iter().enumerate() {
        let r = stack.get(k + 1).map_or(offset, |&o| cx.toks[o].start);
        levels.push((open + 1, cx.matching_close(open, n), r));
    }

    let mut blocks = Vec::new();
    let mut ctes = Vec::new();
    for &(lo, hi, pos) in levels.iter().rev() {
        let els = cx.elems(lo, hi);
        cx.parse_ctes(&els, &mut ctes);
        let is_sep = |e: &El| ["union", "intersect", "except"].iter().any(|w| cx.word_is(e, w));
        let start_of = |e: &El| match e {
            El::Tok(i) | El::Group { open: i, .. } => cx.toks[*i].start,
        };
        let seg_lo = els.iter().rposition(|e| is_sep(e) && start_of(e) < pos).map_or(0, |i| i + 1);
        let seg_hi = els
            .iter()
            .enumerate()
            .skip(seg_lo)
            .find(|(_, e)| is_sep(e) && start_of(e) >= pos)
            .map_or(els.len(), |(i, _)| i);
        cx.scan(&els[seg_lo..seg_hi], false, &mut blocks);
    }
    blocks.extend(ctes);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Vec<ScopeItem> {
        let off = s.find('|').expect("imleç");
        let sql = format!("{}{}", &s[..off], &s[off + 1..]);
        scope_at(&sql, off)
    }

    fn rel(alias: Option<&str>, schema: Option<&str>, name: &str) -> ScopeItem {
        ScopeItem::Relation {
            alias: alias.map(Into::into),
            schema: schema.map(Into::into),
            name: name.into(),
            columns: None,
        }
    }

    fn cols(c: &[&str]) -> Option<Vec<String>> {
        Some(c.iter().map(|s| s.to_string()).collect())
    }

    fn relc(alias: &str, name: &str, c: &[&str]) -> ScopeItem {
        ScopeItem::Relation { alias: Some(alias.into()), schema: None, name: name.into(), columns: cols(c) }
    }

    fn func(alias: Option<&str>, name: &str) -> ScopeItem {
        ScopeItem::Function { alias: alias.map(Into::into), name: name.into(), columns: None }
    }

    fn funcc(alias: &str, name: &str, c: &[&str]) -> ScopeItem {
        ScopeItem::Function { alias: Some(alias.into()), name: name.into(), columns: cols(c) }
    }

    #[test]
    fn simple_and_schema() {
        assert_eq!(at("select o.| from app.orders o"), [rel(Some("o"), Some("app"), "orders")]);
        assert_eq!(at("select | from orders"), [rel(None, None, "orders")]);
        assert_eq!(at("select | from db.app.orders o"), [rel(Some("o"), Some("app"), "orders")]);
    }

    #[test]
    fn as_and_joins() {
        assert_eq!(
            at("select | from orders as o join customers c on c.id = o.cid"),
            [rel(Some("o"), None, "orders"), rel(Some("c"), None, "customers")]
        );
        assert_eq!(
            at("select | from a left outer join b on true cross join c natural join d"),
            [rel(None, None, "a"), rel(None, None, "b"), rel(None, None, "c"), rel(None, None, "d")]
        );
    }

    #[test]
    fn quoted_and_case_folding() {
        assert_eq!(
            at("select | from \"Order Items\" \"OI\", Foo F, \"a\"\"b\""),
            [
                rel(Some("OI"), None, "Order Items"),
                rel(Some("f"), None, "foo"),
                rel(None, None, "a\"b")
            ]
        );
    }

    #[test]
    fn comma_list_and_using() {
        assert_eq!(at("select | from a, b x"), [rel(None, None, "a"), rel(Some("x"), None, "b")]);
        assert_eq!(
            at("select | from a join t using (id)"),
            [rel(None, None, "a"), rel(None, None, "t")]
        );
        // ON ifadesinden sonra gelen virgül FROM listesini sürdürür.
        assert_eq!(
            at("select | from a join b on a.x = b.x, c d"),
            [rel(None, None, "a"), rel(None, None, "b"), rel(Some("d"), None, "c")]
        );
    }

    #[test]
    fn lateral_only_and_functions() {
        assert_eq!(
            at("select | from t, lateral fn(x) f"),
            [rel(None, None, "t"), func(Some("f"), "fn")]
        );
        assert_eq!(at("select | from only t"), [rel(None, None, "t")]);
        assert_eq!(at("select | from only t a"), [rel(Some("a"), None, "t")]);
        assert_eq!(at("select g.| from generate_series(1,3) g"), [func(Some("g"), "generate_series")]);
        assert_eq!(at("select | from generate_series(1,3)"), [func(None, "generate_series")]);
        assert_eq!(
            at("select | from unnest(a) with ordinality as u(x, n)"),
            [funcc("u", "unnest", &["x", "n"])]
        );
        assert_eq!(at("select | from pg_catalog.pg_get_keywords() k"), [func(Some("k"), "pg_get_keywords")]);
    }

    #[test]
    fn alias_never_a_keyword() {
        for tail in [
            "where x", "on x", "join u", "inner join u", "left join u", "right join u", "full join u",
            "cross join u", "natural join u", "group by x", "order by x", "having x", "limit 1",
            "offset 1", "union select 1", "intersect select 1", "except select 1", "window w as ()",
            "fetch first 1 row only", "for update", "tablesample system (1)",
        ] {
            let items = at(&format!("select | from t {tail}"));
            assert_eq!(items.first(), Some(&rel(None, None, "t")), "{tail}");
        }
    }

    #[test]
    fn statement_boundary() {
        assert_eq!(at("select | from t1; select 1 from t2"), [rel(None, None, "t1")]);
        assert_eq!(at("select 1 from t1; select | from t2"), [rel(None, None, "t2")]);
        assert_eq!(at("with c as (select 1) select 1; select | from t"), [rel(None, None, "t")]);
        // Parantez içindeki `;` sınır değildir (bozuk metin); dolar-tırnak içindeki de.
        assert_eq!(at("select | from t where x = $$;$$"), [rel(None, None, "t")]);
    }

    #[test]
    fn non_alias_keywords_after_insert_target() {
        for tail in [
            "default values returning |",
            "overriding system value values (1) returning |",
            "table u returning |",
            "select 1 returning |",
        ] {
            assert_eq!(at(&format!("insert into t {tail}")).first(), Some(&rel(None, None, "t")), "{tail}");
        }
        assert_eq!(at("insert into t as a default values returning a.|"), [rel(Some("a"), None, "t")]);
    }

    #[test]
    fn tablesample_and_rows_from() {
        assert_eq!(at("select | from t s tablesample system (10)"), [rel(Some("s"), None, "t")]);
        assert_eq!(at("select | from t tablesample system (10) s"), [rel(Some("s"), None, "t")]);
        assert_eq!(
            at("select | from t tablesample bernoulli (5) repeatable (1) s, u"),
            [rel(Some("s"), None, "t"), rel(None, None, "u")]
        );
        assert_eq!(at("select | from t s tablesample system (1) where x"), [rel(Some("s"), None, "t")]);
        assert_eq!(
            at("select | from rows from (f(1), g(2)) with ordinality as r(a, b, n)"),
            [funcc("r", "f", &["a", "b", "n"])]
        );
        assert_eq!(at("select | from rows from (f(1)) r"), [func(Some("r"), "f")]);
        // `rows` adlı tablo hâlâ ilişkidir.
        assert_eq!(at("select | from rows r"), [rel(Some("r"), None, "rows")]);
    }

    #[test]
    fn column_alias_lists() {
        assert_eq!(at("select x.| from t x(a, b)"), [relc("x", "t", &["a", "b"])]);
        assert_eq!(at("select x.| from t as x (\"A\", B)"), [relc("x", "t", &["A", "b"])]);
        assert_eq!(at("select u.| from unnest(a) as u(v, n)"), [funcc("u", "unnest", &["v", "n"])]);
        // Kolon tanım listesi: yalnızca adlar.
        assert_eq!(
            at("select x.| from f() as x(a int, b character varying(3), c text[])"),
            [funcc("x", "f", &["a", "b", "c"])]
        );
        assert_eq!(at("select | from t * x"), [rel(Some("x"), None, "t")]);
    }

    #[test]
    fn nested_subquery_and_correlation() {
        assert_eq!(
            at("select (select o2.| from orders o2 where o2.id = o.id) from orders o"),
            [rel(Some("o2"), None, "orders"), rel(Some("o"), None, "orders")]
        );
        // Dış blok, imleç alt sorguda değilken alt sorgunun öğelerini görmez.
        assert_eq!(
            at("select | , (select 1 from orders o2) from orders o"),
            [rel(Some("o"), None, "orders")]
        );
    }

    #[test]
    fn from_subquery() {
        let sql = "select s.| from (select a, b from t) s(x, y)";
        let off = sql.find('|').unwrap();
        let sql = sql.replace('|', "");
        let items = scope_at(&sql, off);
        match items.as_slice() {
            [ScopeItem::Subquery { alias, body, columns }] => {
                assert_eq!(alias, "s");
                assert_eq!(&sql[body.clone()], "select a, b from t");
                assert_eq!(columns, &Some(vec!["x".to_string(), "y".to_string()]));
            }
            other => panic!("{other:?}"),
        }
        // İmleç alt sorgunun içinde: iç blok önce, dış alias öğesi sonra.
        let items = at("select * from (select x.| from t x) s");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], rel(Some("x"), None, "t"));
        assert!(matches!(&items[1], ScopeItem::Subquery { alias, .. } if alias == "s"));
        // Alias'sız alt sorgu çözümlenemez, öğe üretilmez.
        assert_eq!(at("select | from (select 1), t"), [rel(None, None, "t")]);
    }

    #[test]
    fn with_recursive_and_cte_chain() {
        let sql = "with recursive r(a,b) as (select 1,2 union all select a+1,b from r where a<3) select r.| from r";
        let off = sql.find('|').unwrap();
        let sql = sql.replace('|', "");
        let items = scope_at(&sql, off);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], rel(None, None, "r"));
        match &items[1] {
            ScopeItem::Cte { name, body, columns } => {
                assert_eq!(name, "r");
                assert_eq!(&sql[body.clone()], "select 1,2 union all select a+1,b from r where a<3");
                assert_eq!(columns, &Some(vec!["a".to_string(), "b".to_string()]));
            }
            other => panic!("{other:?}"),
        }
        // İmleç CTE gövdesinde, UNION'ın ikinci kolunda: yalnızca o kolun öğeleri + CTE.
        let items = at("with recursive r(a) as (select 1 from x union all select | from r) select 1");
        assert_eq!(items[0], rel(None, None, "r"));
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[1], ScopeItem::Cte { name, .. } if name == "r"));
    }

    #[test]
    fn cte_to_cte() {
        let items = at("with a as (select 1 x), b as not materialized (select | from a) select * from b");
        assert_eq!(items[0], rel(None, None, "a"));
        assert_eq!(items[1], rel(None, None, "b"));
        let names: Vec<&str> = items[2..]
            .iter()
            .map(|i| match i {
                ScopeItem::Cte { name, columns, .. } => {
                    assert!(columns.is_none());
                    name.as_str()
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn set_operations_pick_cursor_branch() {
        assert_eq!(at("select 1 from a union select | from b"), [rel(None, None, "b")]);
        assert_eq!(at("select | from a union select 1 from b"), [rel(None, None, "a")]);
    }

    #[test]
    fn dml_statements() {
        assert_eq!(
            at("update t a set x = 1 from u b where a.|"),
            [rel(Some("a"), None, "t"), rel(Some("b"), None, "u")]
        );
        assert_eq!(at("update only app.t set x = 1 where |"), [rel(None, Some("app"), "t")]);
        assert_eq!(
            at("delete from t a using u b where a.|"),
            [rel(Some("a"), None, "t"), rel(Some("b"), None, "u")]
        );
        assert_eq!(at("insert into t as a values (1) returning a.|"), [rel(Some("a"), None, "t")]);
        assert_eq!(
            at("insert into app.t (x, y) values (1, 2) returning |"),
            [rel(None, Some("app"), "t")]
        );
        // `on conflict ... do update` ve `for update` yeni öğe üretmez.
        assert_eq!(
            at("insert into t values (1) on conflict (id) do update set x = 2 returning |"),
            [rel(None, None, "t")]
        );
        assert_eq!(at("select | from t for update"), [rel(None, None, "t")]);
        // WITH ile başlayan DML.
        let items = at("with s as (select 1) delete from t a using s where a.|");
        assert_eq!(items[0], rel(Some("a"), None, "t"));
        assert_eq!(items[1], rel(None, None, "s"));
    }

    #[test]
    fn not_a_from_keyword() {
        assert_eq!(at("select | from a where x is distinct from y"), [rel(None, None, "a")]);
        assert_eq!(at("select extract(year from d), | from a"), [rel(None, None, "a")]);
    }

    #[test]
    fn parenthesised_join() {
        assert_eq!(
            at("select | from (a x join b y on x.i = y.i) z"),
            [rel(Some("x"), None, "a"), rel(Some("y"), None, "b")]
        );
    }

    #[test]
    fn broken_syntax_keeps_collected_items() {
        assert_eq!(at("select | from orders o join"), [rel(Some("o"), None, "orders")]);
        assert_eq!(at("select | from orders o left join where"), [rel(Some("o"), None, "orders")]);
        assert_eq!(at("select o.| from orders o where (a"), [rel(Some("o"), None, "orders")]);
        assert_eq!(at("select | from (select * from t"), []);
        assert_eq!(at("select * from (select x.| from t x"), [rel(Some("x"), None, "t")]);
        assert_eq!(at("select | from app."), [rel(None, None, "app")]);
        assert!(at("select |").is_empty());
        assert!(at("|").is_empty());
        assert!(at(") ) | ((").is_empty());
        // Kapanmamış CTE gövdesi de CTE olarak görünür.
        let items = at("with x as (select 1 | ");
        assert!(matches!(items.as_slice(), [ScopeItem::Cte { name, .. }] if name == "x"));
    }

    #[test]
    fn comments_and_strings_are_ignored() {
        assert_eq!(at("select | /* from x */ from /* c */ t -- from y\n"), [rel(None, None, "t")]);
        assert_eq!(at("select ' from x ', | from t"), [rel(None, None, "t")]);
    }

    #[test]
    fn unicode_identifiers() {
        assert_eq!(
            at("select | from Çalışma c, \"Müşteri\" m, işlem"),
            [
                rel(Some("c"), None, "Çalışma"),
                rel(Some("m"), None, "Müşteri"),
                rel(None, None, "işlem")
            ]
        );
        assert_eq!(ident_norm("ÇALIŞMA"), "ÇaliŞma");
        assert_eq!(at("select | -- é\nfrom t"), [rel(None, None, "t")]);
    }

    #[test]
    fn ident_norm_cases() {
        assert_eq!(ident_norm("Foo"), "foo");
        assert_eq!(ident_norm("\"Foo\""), "Foo");
        assert_eq!(ident_norm("\"a\"\"b\""), "a\"b");
        assert_eq!(ident_norm("\"abc"), "abc");
        assert_eq!(ident_norm("\""), "");
        assert_eq!(ident_norm(""), "");
    }

    #[test]
    fn resolve_rules() {
        let items = at("select | from app.orders o, \"Cust\", other orders");
        assert_eq!(resolve(&items, "o"), Some(&items[0]));
        assert_eq!(resolve(&items, "O"), Some(&items[0]));
        assert_eq!(resolve(&items, "\"Cust\""), Some(&items[1]));
        assert_eq!(resolve(&items, "cust"), None);
        // Alias ada göre öncelikli: alias'ı `orders` olan öğe, adı `orders` olandan önce gelir.
        assert_eq!(resolve(&items, "orders"), Some(&items[2]));
        assert_eq!(resolve(&items, "app"), None);
        let items = at("with c as (select 1) select | from c");
        assert!(matches!(resolve(&items, "c"), Some(ScopeItem::Relation { .. })));
        let items = at("select | from (select 1) s");
        assert!(matches!(resolve(&items, "s"), Some(ScopeItem::Subquery { .. })));
    }

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn fuzz_never_panics() {
        let words = [
            "select", "from", "join", "where", "with", "as", "update", "delete", "insert", "into",
            "using", "union", "lateral", "only", "recursive", "values", "returning", "set", "a", "b",
            "\"Q\"", "ç", "(", ")", ",", ".", ";", "'", "\"", "$$", "--", "/*", "1", " ", "\n", "😀",
        ];
        let mut seed = 0x9E3779B97F4A7C15u64;
        for _ in 0..20_000 {
            let n = (xorshift(&mut seed) % 24) as usize;
            let mut sql = String::new();
            for _ in 0..n {
                sql.push_str(words[(xorshift(&mut seed) % words.len() as u64) as usize]);
                if xorshift(&mut seed) & 1 == 0 {
                    sql.push(' ');
                }
            }
            for off in [0, sql.len() / 2, sql.len(), sql.len() + 5] {
                for it in scope_at(&sql, off) {
                    if let ScopeItem::Cte { body, .. } | ScopeItem::Subquery { body, .. } = it {
                        assert!(body.start <= body.end && body.end <= sql.len(), "{sql:?}");
                        assert!(sql.is_char_boundary(body.start) && sql.is_char_boundary(body.end), "{sql:?}");
                    }
                }
            }
        }
    }
}
