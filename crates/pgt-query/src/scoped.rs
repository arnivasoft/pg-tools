//! Alias'lı tamamlamanın saf kısmı (`CompletionRequest::Scoped`): imleç kapsamındaki öğelerden kolon kaynaklarını
//! (katalog, Parse, bilinen adlar) çıkarır; Parse edilecek SQL'i kurar (CTE bağımlılıkları `WITH` ile eklenir);
//! alias kolon listesiyle yeniden adlandırır; yerel `quote_ident`. Ağ yok. SQL metni günlüğe yazılmaz.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use crate::complete::{COMPLETION_LIMIT, CompletionItem, CompletionKind};
use crate::lex::{Kind, lex};
use crate::scope::{ScopeItem, ident_norm, is_kw, resolve, scope_at};
use crate::split::{split, stmt_at};

/// Kolonların nereden geldiği.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// Katalog kolonları (`to_regclass` ile, arama yoluna dokunmadan).
    Catalog { schema: Option<String>, name: String },
    /// Bu SQL Parse edilerek (çalıştırılmadan) bulunur.
    Parse(String),
    /// Bilinen adlar (açık kolon listesi, fonksiyon alias'ı).
    Fixed(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// Select listesi önerilerindeki niteleyici (ham ad); önek modunda `None`.
    pub qualifier: Option<String>,
    pub source: Source,
    /// Alias kolon listesi: kaynağın ilk N kolonunu yeniden adlandırır (PostgreSQL kuralı).
    pub rename: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    Targets(Vec<Target>),
    /// Kapsamda eşleşme yok: Plan 5 `Columns { schema: None, relation }` davranışı.
    Fallback(String),
}

/// PostgreSQL `quote_ident`'in yerel karşılığı: yalnızca `[a-z_][a-z0-9_]*` ve anahtar sözcük olmayan adlar
/// tırnaksız kalır (fazladan tırnaklamak zararsızdır).
pub(crate) fn quote_ident(s: &str) -> String {
    let safe = s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !is_kw(s);
    if safe { s.to_string() } else { format!("\"{}\"", s.replace('"', "\"\"")) }
}

/// Parse önbelleği anahtarı (Parse edilecek metnin karması).
pub(crate) fn body_hash(sql: &str) -> u64 {
    let mut h = DefaultHasher::new();
    sql.hash(&mut h);
    h.finish()
}

fn byte_at_char(sql: &str, ch: u32) -> usize {
    sql.char_indices().nth(ch as usize).map_or(sql.len(), |(b, _)| b)
}

/// İstek planı. `offset`: `sql` içinde Unicode scalar ofset; `prefix`: niteleyici yazıldığı gibi (boşsa select
/// listesi kipi).
pub(crate) fn plan(sql: &str, offset: u32, prefix: &str) -> Plan {
    // Savunma amaçlı yeniden bölme: yalnızca imlecin ifadesi.
    let byte = byte_at_char(sql, offset);
    let region = stmt_at(sql, byte)
        .and_then(|i| split(sql).get(i).map(|s| s.region_start..s.region_end))
        .unwrap_or(0..sql.len());
    let stmt = &sql[region.clone()];
    let local = byte.clamp(region.start, region.end) - region.start;
    let items = scope_at(stmt, local);

    let prefix = prefix.trim();
    if prefix.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for item in &items {
            let qualifier = match item {
                ScopeItem::Cte { .. } => continue,
                ScopeItem::Relation { alias, name, .. } | ScopeItem::Function { alias, name, .. } => {
                    alias.clone().unwrap_or_else(|| name.clone())
                }
                ScopeItem::Subquery { alias, .. } => alias.clone(),
            };
            if seen.contains(&qualifier) {
                continue; // iç blok önce: dış bloktaki aynı ad gölgelenir
            }
            seen.push(qualifier.clone());
            let mut t = target(stmt, &items, item);
            t.qualifier = Some(qualifier);
            out.push(t);
        }
        return Plan::Targets(out);
    }
    match resolve(&items, prefix) {
        Some(item) => Plan::Targets(vec![target(stmt, &items, item)]),
        None => Plan::Fallback(ident_norm(prefix)),
    }
}

fn target(stmt: &str, items: &[ScopeItem], item: &ScopeItem) -> Target {
    let (source, rename) = match item {
        ScopeItem::Relation { schema, name, columns, .. } => {
            let cte = schema.is_none().then(|| find_cte(items, name)).flatten();
            match cte {
                Some(c) => (cte_source(stmt, items, c), columns.clone()),
                None => (Source::Catalog { schema: schema.clone(), name: name.clone() }, columns.clone()),
            }
        }
        ScopeItem::Cte { .. } => (cte_source(stmt, items, item), None),
        ScopeItem::Subquery { body, columns, .. } => match columns {
            Some(c) => (Source::Fixed(c.clone()), None),
            None => (Source::Parse(subquery_sql(stmt, items, body)), None),
        },
        ScopeItem::Function { alias, name, columns } => {
            let cols = columns.clone().unwrap_or_else(|| vec![alias.clone().unwrap_or_else(|| name.clone())]);
            (Source::Fixed(cols), None)
        }
    };
    Target { qualifier: None, source, rename }
}

fn find_cte<'a>(items: &'a [ScopeItem], name: &str) -> Option<&'a ScopeItem> {
    items.iter().find(|i| matches!(i, ScopeItem::Cte { name: n, .. } if n == name))
}

fn cte_source(stmt: &str, items: &[ScopeItem], cte: &ScopeItem) -> Source {
    match cte {
        ScopeItem::Cte { columns: Some(c), .. } => Source::Fixed(c.clone()),
        ScopeItem::Cte { name, .. } => Source::Parse(cte_sql(stmt, items, name)),
        _ => Source::Fixed(Vec::new()),
    }
}

/// Metin aralığındaki ad belirteçleri (karşılaştırma biçiminde).
fn refs(stmt: &str, body: &Range<usize>) -> Vec<String> {
    let text = stmt.get(body.clone()).unwrap_or("");
    lex(text)
        .into_iter()
        .filter(|t| matches!(t.kind, Kind::Word | Kind::Ident))
        .map(|t| ident_norm(t.text(text)))
        .collect()
}

/// Kapsamdaki bir CTE: (ad, gövde aralığı, açık kolon listesi).
type CteRef<'a> = (&'a str, &'a Range<usize>, Option<&'a Vec<String>>);

/// Kapsamdaki CTE'ler (aynı ad bir kez; iç blok önce).
fn ctes(items: &[ScopeItem]) -> Vec<CteRef<'_>> {
    let mut out: Vec<CteRef<'_>> = Vec::new();
    for i in items {
        if let ScopeItem::Cte { name, body, columns } = i
            && !out.iter().any(|(n, ..)| n == name)
        {
            out.push((name, body, columns.as_ref()));
        }
    }
    out
}

/// `seed` adlarının (ve bağımlılıklarının) başvurduğu CTE'ler, metin sırasıyla.
fn closure<'a>(
    stmt: &str,
    all: &[CteRef<'a>],
    seed: &[String],
) -> Vec<CteRef<'a>> {
    let mut need: Vec<CteRef<'a>> = Vec::new();
    let mut queue: Vec<String> = seed.to_vec();
    while let Some(r) = queue.pop() {
        if need.iter().any(|(n, ..)| *n == r) {
            continue;
        }
        if let Some(c) = all.iter().find(|(n, ..)| *n == r) {
            need.push(*c);
            queue.extend(refs(stmt, c.1));
        }
    }
    need.sort_by_key(|(_, b, _)| b.start);
    need
}

fn with_clause(stmt: &str, need: &[CteRef<'_>]) -> String {
    // Kendine ya da metinde sonra gelen bir CTE'ye başvuru varsa RECURSIVE gerekir.
    let recursive = need.iter().any(|(_, body, _)| {
        let r = refs(stmt, body);
        need.iter().any(|(n, b, _)| b.start >= body.start && r.iter().any(|x| x == n))
    });
    let parts: Vec<String> = need
        .iter()
        .map(|(name, body, cols)| {
            let cols = cols.map_or_else(String::new, |c| {
                format!("({})", c.iter().map(|x| quote_ident(x)).collect::<Vec<_>>().join(", "))
            });
            format!("{}{cols} AS (\n{}\n)", quote_ident(name), stmt.get((*body).clone()).unwrap_or(""))
        })
        .collect();
    format!("WITH {}{}", if recursive { "RECURSIVE " } else { "" }, parts.join(", "))
}

/// `WITH <bağımlılıklar>, ad AS (gövde) SELECT * FROM ad`.
fn cte_sql(stmt: &str, items: &[ScopeItem], name: &str) -> String {
    let all = ctes(items);
    let need = closure(stmt, &all, &[name.to_string()]);
    format!("{} SELECT * FROM {}", with_clause(stmt, &need), quote_ident(name))
}

/// Bağımlılık yoksa gövdenin kendisi; varsa `WITH … SELECT * FROM (gövde) AS pgt_s`.
fn subquery_sql(stmt: &str, items: &[ScopeItem], body: &Range<usize>) -> String {
    let text = stmt.get(body.clone()).unwrap_or("");
    let all = ctes(items);
    let need = closure(stmt, &all, &refs(stmt, body));
    if need.is_empty() {
        return text.to_string();
    }
    format!("{} SELECT * FROM (\n{text}\n) AS pgt_s", with_clause(stmt, &need))
}

/// Parse sonucu kolonları öğelere çevirir.
pub(crate) fn parsed_items(cols: &[(String, String)]) -> Vec<CompletionItem> {
    cols.iter()
        .map(|(name, ty)| CompletionItem {
            label: name.clone(),
            quoted: quote_ident(name),
            kind: CompletionKind::Column,
            detail: Some(ty.clone()),
        })
        .collect()
}

pub(crate) fn fixed_items(names: &[String]) -> Vec<CompletionItem> {
    names
        .iter()
        .map(|n| CompletionItem { label: n.clone(), quoted: quote_ident(n), kind: CompletionKind::Column, detail: None })
        .collect()
}

/// Kaynağın kolonlarına yeniden adlandırma ve niteleyiciyi uygular.
pub(crate) fn finish(target: &Target, mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    if let Some(names) = &target.rename {
        for (it, n) in items.iter_mut().zip(names) {
            it.label = n.clone();
            it.quoted = quote_ident(n);
        }
    }
    if let Some(q) = &target.qualifier {
        let qq = quote_ident(q);
        for it in &mut items {
            it.label = format!("{q}.{}", it.label);
            it.quoted = format!("{qq}.{}", it.quoted);
        }
    }
    items
}

/// Hedeflerin sonuçlarını birleştirir (en çok `COMPLETION_LIMIT`).
pub(crate) fn assemble(targets: &[Target], bases: Vec<Option<Vec<CompletionItem>>>) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for (t, base) in targets.iter().zip(bases) {
        if let Some(base) = base {
            out.extend(finish(t, base));
        }
        if out.len() >= COMPLETION_LIMIT {
            break;
        }
    }
    out.truncate(COMPLETION_LIMIT);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str, prefix: &str) -> Plan {
        let off = s.find('|').expect("imleç");
        let sql = format!("{}{}", &s[..off], &s[off + 1..]);
        plan(&sql, sql[..off].chars().count() as u32, prefix)
    }

    fn one(s: &str, prefix: &str) -> Target {
        match p(s, prefix) {
            Plan::Targets(mut t) if t.len() == 1 => t.remove(0),
            other => panic!("{other:?}"),
        }
    }

    fn catalog(schema: Option<&str>, name: &str) -> Source {
        Source::Catalog { schema: schema.map(Into::into), name: name.into() }
    }

    fn names(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn relation_alias_and_quoted() {
        assert_eq!(one("select o.| from app.orders o", "o").source, catalog(Some("app"), "orders"));
        assert_eq!(one("select O.| from app.orders o", "O").source, catalog(Some("app"), "orders"));
        assert_eq!(
            one("select oi.| from \"Order Items\" oi", "oi").source,
            catalog(None, "Order Items")
        );
        assert_eq!(one("select \"OI\".| from t \"OI\"", "\"OI\"").source, catalog(None, "t"));
        // Alias kolon listesi yeniden adlandırma olarak taşınır.
        let t = one("select x.| from t x(a, b)", "x");
        assert_eq!(t.source, catalog(None, "t"));
        assert_eq!(t.rename, Some(names(&["a", "b"])));
    }

    #[test]
    fn no_match_falls_back_to_plan5() {
        assert_eq!(p("select orders.| from x", "Orders"), Plan::Fallback("orders".into()));
        assert_eq!(p("select \"Q\".|", "\"Q\""), Plan::Fallback("Q".into()));
    }

    #[test]
    fn utf16_independent_scalar_offset() {
        // Emoji ve çok baytlı harf imleçten önce: ofset scalar sayılır.
        assert_eq!(one("select '😀', ğ.| from tablo ğ", "ğ").source, catalog(None, "tablo"));
    }

    #[test]
    fn only_the_cursor_statement() {
        assert_eq!(one("select 1 from a x; select x.| from b x; select 2 from c x", "x").source, catalog(None, "b"));
    }

    #[test]
    fn cte_and_subquery_sources() {
        let t = one("with x as (insert into t values (1) returning id) select x.| from x", "x");
        assert_eq!(
            t.source,
            Source::Parse("WITH x AS (\ninsert into t values (1) returning id\n) SELECT * FROM x".into())
        );
        let t = one("with a as (select 1 v), b as (select v from a) select b.| from b", "b");
        assert_eq!(
            t.source,
            Source::Parse("WITH a AS (\nselect 1 v\n), b AS (\nselect v from a\n) SELECT * FROM b".into())
        );
        // Açık kolon listesi Parse gerektirmez; ilişki alias'ının listesi yeniden adlandırır.
        let t = one("with c(p, q) as (select 1, 2) select z.| from c z(r)", "z");
        assert_eq!(t.source, Source::Fixed(names(&["p", "q"])));
        assert_eq!(t.rename, Some(names(&["r"])));
        let t = one("select s.| from (select a, b from t) s", "s");
        assert_eq!(t.source, Source::Parse("select a, b from t".into()));
        let t = one("select s.| from (select a from t) s(x)", "s");
        assert_eq!(t.source, Source::Fixed(names(&["x"])));
        // Alt sorgu bir CTE'ye başvuruyor.
        let t = one("with a(\"V\") as (select 1) select s.| from (select * from a) s", "s");
        assert_eq!(
            t.source,
            Source::Parse("WITH a(\"V\") AS (\nselect 1\n) SELECT * FROM (\nselect * from a\n) AS pgt_s".into())
        );
        // Kendine başvuran CTE: RECURSIVE.
        let t = one("with recursive r as (select 1 n union all select n + 1 from r) select r.| from r", "r");
        assert!(matches!(&t.source, Source::Parse(s) if s.starts_with("WITH RECURSIVE r AS")), "{t:?}");
    }

    #[test]
    fn functions() {
        assert_eq!(one("select g.| from generate_series(1, 3) g", "g").source, Source::Fixed(names(&["g"])));
        assert_eq!(
            one("select u.| from unnest(a, b) as u(x, y)", "u").source,
            Source::Fixed(names(&["x", "y"]))
        );
    }

    #[test]
    fn select_list_mode() {
        let Plan::Targets(ts) = p("select | from app.orders o join \"Order Items\" on true, (select 1 a) s", "") else {
            panic!()
        };
        let q: Vec<_> = ts.iter().map(|t| t.qualifier.clone().unwrap()).collect();
        assert_eq!(q, ["o", "Order Items", "s"]);
        // Kapsam boşsa hedef yok.
        assert_eq!(p("select |", ""), Plan::Targets(vec![]));
    }

    #[test]
    fn rename_qualify_and_cap() {
        let t = Target { qualifier: Some("Ö".into()), source: Source::Fixed(vec![]), rename: Some(names(&["A"])) };
        let items = finish(&t, fixed_items(&names(&["x", "y"])));
        let got: Vec<(&str, &str)> = items.iter().map(|i| (i.label.as_str(), i.quoted.as_str())).collect();
        assert_eq!(got, [("Ö.A", "\"Ö\".\"A\""), ("Ö.y", "\"Ö\".y")]);
        let many: Vec<String> = (0..400).map(|i| format!("c{i}")).collect();
        let t = Target { qualifier: Some("a".into()), source: Source::Fixed(vec![]), rename: None };
        let out = assemble(&[t.clone(), t], vec![Some(fixed_items(&many)), Some(fixed_items(&many))]);
        assert_eq!(out.len(), COMPLETION_LIMIT);
    }

    #[test]
    fn local_quote_ident() {
        for (i, o) in [
            ("abc", "abc"),
            ("_a1", "_a1"),
            ("Abc", "\"Abc\""),
            ("1a", "\"1a\""),
            ("a b", "\"a b\""),
            ("a\"b", "\"a\"\"b\""),
            ("select", "\"select\""),
            ("ç", "\"ç\""),
            ("a$", "\"a$\""),
            ("", "\"\""),
        ] {
            assert_eq!(quote_ident(i), o, "{i}");
        }
    }
}
