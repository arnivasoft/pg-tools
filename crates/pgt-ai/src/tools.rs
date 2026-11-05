//! Asistanın salt okunur katalog araçları. Her araç sabit, `$n` parametreli bir katalog sorgusudur:
//! model serbest SQL gönderemez, satır verisi okunmaz (yalnız ad, tür, kısıt, indeks, açıklama ve tahmini
//! satır sayısı/boyut). Sorgular uygulamanın katalog bağlantı havuzunda koşar (kullanıcının sorgu oturumunu
//! ve açık transaction'ını etkilemez).

use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio_postgres::Client;

/// Bir araç yanıtındaki en çok satır; fazlası için daraltma önerilir.
pub const MAX_ROWS: usize = 200;
/// Görünüm tanımı gibi uzun metinlerin kırpılma sınırı (karakter); kırpıldığı açıkça yazılır.
const MAX_TEXT: usize = 8_000;

const SYSTEM_SCHEMAS: &str = "n.nspname NOT IN ('pg_catalog', 'information_schema') \
     AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp%'";

/// Araç tanımları (her istekte aynı dizi: araç kümesi değişirse thinking blokları ve önbellek bozulur).
pub fn definitions() -> Value {
    let nullable = |desc: &str| json!({ "type": ["string", "null"], "description": desc });
    json!([
        {
            "name": "list_relations",
            "description": "List tables, partitioned tables, views, materialized views and foreign tables in the connected database, with estimated row count, total size and comment. Partitions are omitted. Use it to discover which tables exist before writing SQL.",
            "strict": true,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": {
                    "schema": nullable("Exact schema name to restrict to, or null for all non-system schemas."),
                    "name_contains": nullable("Case-insensitive substring of the relation name, or null for all.")
                },
                "required": ["schema", "name_contains"],
                "additionalProperties": false
            }
        },
        {
            "name": "describe_relation",
            "description": "Describe one relation: columns (type, nullability, default, comment), primary/unique/check/foreign-key constraints, foreign keys from other tables that reference it, indexes, and the definition of a view. Use it before writing joins or filters on a table.",
            "strict": true,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": {
                    "schema": nullable("Exact schema name, or null to search all non-system schemas."),
                    "name": { "type": "string", "description": "Exact relation name (unquoted, case-sensitive as stored)." }
                },
                "required": ["schema", "name"],
                "additionalProperties": false
            }
        },
        {
            "name": "search_columns",
            "description": "Find columns whose name contains a substring (case-insensitive) across all non-system relations. Use it to locate where a concept (e.g. customer_id, email) is stored.",
            "strict": true,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": {
                    "name_contains": { "type": "string", "description": "Case-insensitive substring of the column name." }
                },
                "required": ["name_contains"],
                "additionalProperties": false
            }
        },
        {
            "name": EXPLAIN_TOOL,
            "description": "Get the estimated execution plan of ONE SQL statement with plain EXPLAIN (no ANALYZE: the statement is planned, never executed), in the user's query session so their search_path applies. Use it to check whether an index would be used or where the cost goes. Replace $1-style parameters with representative literal values. It fails if the user's session is busy, has an open transaction, or has not been opened yet; then answer without the plan.",
            "strict": true,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "description": "Exactly one SQL statement, without a leading EXPLAIN." }
                },
                "required": ["sql"],
                "additionalProperties": false
            }
        }
    ])
}

/// Kullanıcının sorgu oturumunda çalışan plan aracı (katalog havuzunda değil; çalıştırması uygulama tarafındadır).
pub const EXPLAIN_TOOL: &str = "explain_query";

/// Plan aracının JSON çıktısının kırpılma sınırı (karakter).
pub const MAX_PLAN_TEXT: usize = 30_000;

/// Plan çıktısını sınırlar; kırpıldığı açıkça yazılır.
pub fn clip_plan(s: &str) -> String {
    if s.chars().count() <= MAX_PLAN_TEXT {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_PLAN_TEXT).collect();
    format!("{head}\n… (plan truncated at {MAX_PLAN_TEXT} characters)")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplainInput {
    sql: String,
}

/// Plan aracı girdisinden SQL (doğrulanmış girdide).
pub fn explain_sql(input: &Value) -> Option<String> {
    serde_json::from_value::<ExplainInput>(input.clone()).ok().map(|i| i.sql)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    schema: Option<String>,
    name_contains: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeInput {
    schema: Option<String>,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    name_contains: String,
}

/// Arayüzde gösterilecek kısa özet ("public.orders" gibi); geçersiz girdide araç adı.
pub fn summary(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    match name {
        "list_relations" => [s("schema"), s("name_contains").map(|n| format!("*{n}*"))].into_iter().flatten().collect::<Vec<_>>().join(" "),
        "describe_relation" => match (s("schema"), s("name")) {
            (Some(sc), Some(n)) => format!("{sc}.{n}"),
            (None, Some(n)) => n,
            _ => String::new(),
        },
        "search_columns" => s("name_contains").unwrap_or_default(),
        EXPLAIN_TOOL => {
            let one_line = s("sql").unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
            if one_line.chars().count() > 60 { format!("{}…", one_line.chars().take(60).collect::<String>()) } else { one_line }
        }
        _ => String::new(),
    }
}

/// Girdiyi doğrular. `Err`: modele `is_error` olarak dönecek açıklama (araç çalıştırılmaz).
pub fn validate(name: &str, input: &Value) -> Result<(), String> {
    let bad = |e: serde_json::Error| format!("Invalid input for {name}: {e}");
    // Şemadaki her alan zorunludur (null olabilir); serde eksik `Option` alanını sessizce kabul ederdi.
    let defs = definitions();
    if let Some(def) = defs.as_array().and_then(|a| a.iter().find(|d| d["name"] == name)) {
        let obj = input.as_object().ok_or_else(|| format!("Invalid input for {name}: expected an object"))?;
        for key in def["input_schema"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                return Err(format!("Invalid input for {name}: missing field `{key}`"));
            }
        }
    }
    match name {
        "list_relations" => serde_json::from_value::<ListInput>(input.clone()).map(|_| ()).map_err(bad),
        "describe_relation" => {
            let i = serde_json::from_value::<DescribeInput>(input.clone()).map_err(bad)?;
            if i.name.trim().is_empty() { Err("Invalid input for describe_relation: name is empty".into()) } else { Ok(()) }
        }
        "search_columns" => {
            let i = serde_json::from_value::<SearchInput>(input.clone()).map_err(bad)?;
            if i.name_contains.trim().is_empty() { Err("Invalid input for search_columns: name_contains is empty".into()) } else { Ok(()) }
        }
        EXPLAIN_TOOL => {
            let i = serde_json::from_value::<ExplainInput>(input.clone()).map_err(bad)?;
            let head = i.sql.trim_start().get(..7).unwrap_or("").to_ascii_lowercase();
            if i.sql.trim().is_empty() {
                Err("Invalid input for explain_query: sql is empty".into())
            } else if head == "explain" {
                Err("Invalid input for explain_query: send the statement without EXPLAIN; the tool adds it.".into())
            } else {
                Ok(())
            }
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_TEXT).collect();
    format!("{head}\n… (truncated at {MAX_TEXT} characters)")
}

fn more_note(out: &mut String, n: usize) {
    if n > MAX_ROWS {
        let _ = writeln!(out, "… more than {MAX_ROWS} results; narrow the search.");
    }
}

fn kind(k: &str) -> &'static str {
    match k {
        "r" => "table",
        "p" => "partitioned table",
        "v" => "view",
        "m" => "materialized view",
        "f" => "foreign table",
        _ => "relation",
    }
}

const LIST_SQL: &str = "SELECT s.nspname, s.relname, s.relkind, s.reltuples, \
            pg_catalog.pg_total_relation_size(s.oid)::pg_catalog.int8, pg_catalog.obj_description(s.oid, 'pg_class') \
     FROM (SELECT c.oid, n.nspname::pg_catalog.text, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, \
                  c.reltuples::pg_catalog.float8 \
           FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
           WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition AND {SYS} \
             AND ($1::pg_catalog.text IS NULL OR n.nspname = $1::pg_catalog.text) \
             AND ($2::pg_catalog.text IS NULL OR pg_catalog.strpos(pg_catalog.lower(c.relname::pg_catalog.text), pg_catalog.lower($2::pg_catalog.text)) > 0) \
           ORDER BY 2, 3 LIMIT 201) s \
     ORDER BY 1, 2";

const FIND_SQL: &str = "SELECT c.oid, n.nspname::pg_catalog.text, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, \
            c.reltuples::pg_catalog.float8, pg_catalog.obj_description(c.oid, 'pg_class') \
     FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND c.relname = $2::pg_catalog.text AND {SYS} \
       AND ($1::pg_catalog.text IS NULL OR n.nspname = $1::pg_catalog.text) \
     ORDER BY 2 LIMIT 20";

const COLUMNS_SQL: &str = "SELECT a.attname::pg_catalog.text, pg_catalog.format_type(a.atttypid, a.atttypmod), a.attnotnull, \
            pg_catalog.pg_get_expr(d.adbin, d.adrelid), pg_catalog.col_description(a.attrelid, a.attnum) \
     FROM pg_catalog.pg_attribute a \
     LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
     WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum";

const CONSTRAINTS_SQL: &str = "SELECT conname::pg_catalog.text, pg_catalog.pg_get_constraintdef(oid, true) \
     FROM pg_catalog.pg_constraint WHERE conrelid = $1 ORDER BY contype, conname";

const REFERENCED_BY_SQL: &str = "SELECT conrelid::pg_catalog.regclass::pg_catalog.text, conname::pg_catalog.text, \
            pg_catalog.pg_get_constraintdef(oid, true) \
     FROM pg_catalog.pg_constraint WHERE confrelid = $1 AND contype = 'f' ORDER BY 1, 2 LIMIT 201";

const INDEXES_SQL: &str = "SELECT pg_catalog.pg_get_indexdef(indexrelid) FROM pg_catalog.pg_index \
     WHERE indrelid = $1 ORDER BY indexrelid";

const VIEWDEF_SQL: &str = "SELECT pg_catalog.pg_get_viewdef($1::pg_catalog.oid, true)";

const SEARCH_SQL: &str = "SELECT n.nspname::pg_catalog.text, c.relname::pg_catalog.text, a.attname::pg_catalog.text, \
            pg_catalog.format_type(a.atttypid, a.atttypmod) \
     FROM pg_catalog.pg_attribute a \
     JOIN pg_catalog.pg_class c ON c.oid = a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition AND {SYS} \
       AND pg_catalog.strpos(pg_catalog.lower(a.attname::pg_catalog.text), pg_catalog.lower($1::pg_catalog.text)) > 0 \
     ORDER BY 1, 2, 3 LIMIT 201";

fn sql(template: &str) -> String {
    template.replace("{SYS}", SYSTEM_SCHEMAS)
}

fn rows_estimate(t: f64) -> String {
    // PG 14+: hiç analiz edilmemiş tabloda -1.
    if t < 0.0 { "unknown".into() } else { format!("{}", t.round() as i64) }
}

/// Aracı çalıştırır. Dış `Err`: veritabanı hatası (bağlantı katmanı sınıflandırır); iç `Err`: modele
/// `is_error` olarak dönecek açıklama. Girdi önceden `validate` ile denetlenmiş olmalıdır.
pub async fn run(client: &Client, name: &str, input: &Value) -> Result<Result<String, String>, tokio_postgres::Error> {
    if let Err(e) = validate(name, input) {
        return Ok(Err(e));
    }
    match name {
        "list_relations" => {
            let i: ListInput = serde_json::from_value(input.clone()).expect("doğrulandı");
            let rows = client.query(&sql(LIST_SQL), &[&i.schema, &i.name_contains]).await?;
            if rows.is_empty() {
                return Ok(Ok("No matching relations.".into()));
            }
            let mut out = String::from("schema\tname\tkind\testimated_rows\ttotal_size_bytes\tcomment\n");
            for r in rows.iter().take(MAX_ROWS) {
                let comment: Option<String> = r.get(5);
                let _ = writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    r.get::<_, String>(0),
                    r.get::<_, String>(1),
                    kind(&r.get::<_, String>(2)),
                    rows_estimate(r.get(3)),
                    r.get::<_, i64>(4),
                    comment.unwrap_or_default().replace(['\n', '\t'], " ")
                );
            }
            more_note(&mut out, rows.len());
            Ok(Ok(out))
        }
        "describe_relation" => {
            let i: DescribeInput = serde_json::from_value(input.clone()).expect("doğrulandı");
            let found = client.query(&sql(FIND_SQL), &[&i.schema, &i.name]).await?;
            match found.len() {
                0 => return Ok(Err(format!("Relation \"{}\" not found. Use list_relations to see what exists.", i.name))),
                1 => {}
                _ => {
                    let schemas: Vec<String> = found.iter().map(|r| r.get(1)).collect();
                    return Ok(Err(format!(
                        "\"{}\" exists in several schemas ({}); call again with schema set.",
                        i.name,
                        schemas.join(", ")
                    )));
                }
            }
            let f = &found[0];
            let oid: u32 = f.get(0);
            let relkind: String = f.get(3);
            let mut out = String::new();
            let _ = writeln!(out, "{}.{} ({}), estimated rows: {}", f.get::<_, String>(1), f.get::<_, String>(2), kind(&relkind), rows_estimate(f.get(4)));
            if let Some(c) = f.get::<_, Option<String>>(5) {
                let _ = writeln!(out, "comment: {}", c.replace('\n', " "));
            }
            out.push_str("\ncolumns (name\ttype\tnullable\tdefault\tcomment):\n");
            for r in client.query(COLUMNS_SQL, &[&oid]).await? {
                let def: Option<String> = r.get(3);
                let comment: Option<String> = r.get(4);
                let _ = writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    r.get::<_, String>(0),
                    r.get::<_, String>(1),
                    if r.get::<_, bool>(2) { "not null" } else { "null" },
                    def.unwrap_or_default(),
                    comment.unwrap_or_default().replace(['\n', '\t'], " ")
                );
            }
            let cons = client.query(CONSTRAINTS_SQL, &[&oid]).await?;
            if !cons.is_empty() {
                out.push_str("\nconstraints:\n");
                for r in &cons {
                    let _ = writeln!(out, "{}: {}", r.get::<_, String>(0), r.get::<_, String>(1));
                }
            }
            let refs = client.query(REFERENCED_BY_SQL, &[&oid]).await?;
            if !refs.is_empty() {
                out.push_str("\nreferenced by:\n");
                for r in refs.iter().take(MAX_ROWS) {
                    let _ = writeln!(out, "{} ({}): {}", r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2));
                }
                more_note(&mut out, refs.len());
            }
            let idx = client.query(INDEXES_SQL, &[&oid]).await?;
            if !idx.is_empty() {
                out.push_str("\nindexes:\n");
                for r in &idx {
                    let _ = writeln!(out, "{}", r.get::<_, String>(0));
                }
            }
            if relkind == "v" || relkind == "m" {
                let def: Option<String> = client.query_one(VIEWDEF_SQL, &[&oid]).await?.get(0);
                if let Some(d) = def {
                    let _ = write!(out, "\ndefinition:\n{}\n", clip(d.trim()));
                }
            }
            Ok(Ok(out))
        }
        "search_columns" => {
            let i: SearchInput = serde_json::from_value(input.clone()).expect("doğrulandı");
            let rows = client.query(&sql(SEARCH_SQL), &[&i.name_contains]).await?;
            if rows.is_empty() {
                return Ok(Ok("No matching columns.".into()));
            }
            let mut out = String::from("schema\trelation\tcolumn\ttype\n");
            for r in rows.iter().take(MAX_ROWS) {
                let _ = writeln!(
                    out,
                    "{}\t{}\t{}\t{}",
                    r.get::<_, String>(0),
                    r.get::<_, String>(1),
                    r.get::<_, String>(2),
                    r.get::<_, String>(3)
                );
            }
            more_note(&mut out, rows.len());
            Ok(Ok(out))
        }
        _ => Ok(Err(format!("Unknown tool: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_are_strict_with_required_fields() {
        let defs = definitions();
        let defs = defs.as_array().unwrap();
        assert_eq!(defs.len(), 4);
        for d in defs {
            assert_eq!(d["strict"], true);
            assert_eq!(d["input_schema"]["additionalProperties"], false);
            let props = d["input_schema"]["properties"].as_object().unwrap();
            let req: Vec<&str> = d["input_schema"]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(props.len(), req.len(), "strict: tüm alanlar zorunlu ({})", d["name"]);
        }
    }

    #[test]
    fn validation_rejects_bad_input_without_running() {
        assert!(validate("list_relations", &json!({"schema": null, "name_contains": null})).is_ok());
        assert!(validate("list_relations", &json!({"schema": null})).is_err());
        assert!(validate("describe_relation", &json!({"schema": null, "name": "  "})).is_err());
        assert!(validate("describe_relation", &json!({"schema": "public", "name": "t", "x": 1})).is_err());
        assert!(validate("search_columns", &json!({"name_contains": "email"})).is_ok());
        assert!(validate("drop_table", &json!({})).is_err());
    }

    #[test]
    fn summaries() {
        assert_eq!(summary("describe_relation", &json!({"schema": "public", "name": "orders"})), "public.orders");
        assert_eq!(summary("list_relations", &json!({"schema": null, "name_contains": "ord"})), "*ord*");
        assert_eq!(summary("search_columns", &json!({"name_contains": "email"})), "email");
        assert_eq!(summary(EXPLAIN_TOOL, &json!({"sql": "select *\n  from   orders"})), "select * from orders");
        let long = format!("select {} from t", "a, ".repeat(40));
        assert!(summary(EXPLAIN_TOOL, &json!({"sql": long})).ends_with('…'));
    }

    #[test]
    fn explain_input_is_one_statement_text_without_explain() {
        assert!(validate(EXPLAIN_TOOL, &json!({"sql": "select 1"})).is_ok());
        assert!(validate(EXPLAIN_TOOL, &json!({"sql": "  "})).is_err());
        assert!(validate(EXPLAIN_TOOL, &json!({"sql": "EXPLAIN ANALYZE delete from t"})).is_err());
        assert!(validate(EXPLAIN_TOOL, &json!({})).is_err());
        assert_eq!(explain_sql(&json!({"sql": "select 1"})).as_deref(), Some("select 1"));
        assert!(clip_plan(&"x".repeat(MAX_PLAN_TEXT + 1)).ends_with("(plan truncated at 30000 characters)"));
    }

    #[test]
    fn long_text_is_clipped_visibly() {
        let s = "x".repeat(MAX_TEXT + 5);
        assert!(clip(&s).ends_with("(truncated at 8000 characters)"));
        assert_eq!(clip("kısa"), "kısa");
    }
}
