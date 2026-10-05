//! Katalog tamamlama sorguları. Sabit, `$n` parametreli SQL; her katalog adı `pg_catalog.` ile nitelenir
//! (kullanıcının `search_path`'inden bağımsız); tırnaklı ad sunucuda `quote_ident` ile üretilir.
//! Sorgular oturumun kendi bağlantısında, yalnızca oturum boştayken çalışır (bkz. `QueryManager::complete`).

use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exec::Executor;

/// Bir istekte döndürülen en çok öğe.
pub const COMPLETION_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CompletionRequest {
    /// Şemalar; `include_system`: `pg_*` ve `information_schema` da dahil.
    Schemas { include_system: bool },
    /// `schema` yoksa oturumun arama yolundaki (`current_schemas(false)`) şemalar.
    Relations { schema: Option<String> },
    /// Bir ilişkinin kolonları (`relation` ham addır, tırnaksız).
    Columns { schema: Option<String>, relation: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CompletionKind {
    Schema,
    Table,
    View,
    MatView,
    Column,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompletionItem {
    /// Ham ad.
    pub label: String,
    /// `quote_ident(label)`: arayüz editöre bunu yazar.
    pub quoted: String,
    pub kind: CompletionKind,
    /// Kolonlarda tür adı.
    pub detail: Option<String>,
}

const SCHEMAS_SQL: &str = "SELECT n.nspname::pg_catalog.text, pg_catalog.quote_ident(n.nspname)::pg_catalog.text \
     FROM pg_catalog.pg_namespace n \
     WHERE $1::pg_catalog.bool OR (pg_catalog.substr(n.nspname::pg_catalog.text, 1, 3) <> 'pg_' AND n.nspname <> 'information_schema') \
     ORDER BY n.nspname LIMIT 500";

const RELATIONS_SQL: &str = "SELECT c.relname::pg_catalog.text, pg_catalog.quote_ident(c.relname)::pg_catalog.text, \
            c.relkind::pg_catalog.text \
     FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') \
       AND (($1::pg_catalog.text IS NOT NULL AND n.nspname = $1::pg_catalog.text) \
         OR ($1::pg_catalog.text IS NULL AND n.nspname = ANY (pg_catalog.current_schemas(false)))) \
     ORDER BY c.relname LIMIT 500";

/// Aynı ad arama yolunda birden çok şemada varsa ilk şemadakini almak için `oid` döner (Rust tarafında süzülür).
const COLUMNS_SQL: &str = "SELECT a.attname::pg_catalog.text, pg_catalog.quote_ident(a.attname)::pg_catalog.text, \
            pg_catalog.format_type(a.atttypid, a.atttypmod)::pg_catalog.text, c.oid::pg_catalog.text \
     FROM pg_catalog.pg_attribute a \
       JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relname = $2::pg_catalog.text \
       AND (($1::pg_catalog.text IS NOT NULL AND n.nspname = $1::pg_catalog.text) \
         OR ($1::pg_catalog.text IS NULL AND n.nspname = ANY (pg_catalog.current_schemas(false)))) \
       AND a.attnum > 0 AND NOT a.attisdropped \
     ORDER BY pg_catalog.array_position(pg_catalog.current_schemas(false)::pg_catalog.text[], n.nspname::pg_catalog.text), \
              c.oid, a.attnum LIMIT 500";

fn text(v: Option<String>) -> String {
    v.unwrap_or_default()
}

fn relation_kind(relkind: &str) -> CompletionKind {
    match relkind {
        "v" => CompletionKind::View,
        "m" => CompletionKind::MatView,
        _ => CompletionKind::Table,
    }
}

/// İsteği oturumun bağlantısında çalıştırır.
pub(crate) async fn lookup(exec: &mut Executor, req: &CompletionRequest) -> Result<Vec<CompletionItem>> {
    let mut items = Vec::new();
    match req {
        CompletionRequest::Schemas { include_system } => {
            for mut r in exec.query_text(SCHEMAS_SQL, &[include_system]).await? {
                let (quoted, label) = (r.remove(1), r.remove(0));
                items.push(CompletionItem {
                    label: text(label),
                    quoted: text(quoted),
                    kind: CompletionKind::Schema,
                    detail: None,
                });
            }
        }
        CompletionRequest::Relations { schema } => {
            for r in exec.query_text(RELATIONS_SQL, &[schema]).await? {
                let mut it = r.into_iter();
                let (label, quoted, kind) = (text(it.next().flatten()), text(it.next().flatten()), text(it.next().flatten()));
                items.push(CompletionItem { label, quoted, kind: relation_kind(&kind), detail: None });
            }
        }
        CompletionRequest::Columns { schema, relation } => {
            if relation.is_empty() {
                return Err(Error::Invalid("ilişki adı boş olamaz".into()));
            }
            let mut first_oid: Option<String> = None;
            for r in exec.query_text(COLUMNS_SQL, &[schema, relation]).await? {
                let mut it = r.into_iter();
                let (label, quoted, detail, oid) =
                    (text(it.next().flatten()), text(it.next().flatten()), it.next().flatten(), text(it.next().flatten()));
                if first_oid.get_or_insert_with(|| oid.clone()) != &oid {
                    break;
                }
                items.push(CompletionItem { label, quoted, kind: CompletionKind::Column, detail });
            }
        }
    }
    items.truncate(COMPLETION_LIMIT);
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_use_tagged_camel_case() {
        let r = CompletionRequest::Columns { schema: Some("app".into()), relation: "t".into() };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({"type": "columns", "schema": "app", "relation": "t"})
        );
        let r: CompletionRequest = serde_json::from_str(r#"{"type":"schemas","includeSystem":true}"#).unwrap();
        assert_eq!(r, CompletionRequest::Schemas { include_system: true });
    }

    #[test]
    fn catalog_names_are_all_qualified() {
        for sql in [SCHEMAS_SQL, RELATIONS_SQL, COLUMNS_SQL] {
            for bad in [" substr(", " pg_namespace", " pg_class", " pg_attribute", " quote_ident", " format_type", " current_schemas"] {
                assert!(!sql.contains(bad), "{bad}");
            }
        }
    }
}
