//! Diyalogların açılır listeleri için salt okunur katalog sorguları (rol, şema, tablespace, eklenti, yöntem, tablo, kolon).

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::Result;
use pgt_core::connection::{ConnectionManager, Target};
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use ts_rs::TS;
use uuid::Uuid;

/// İndeks erişim yöntemleri: arayüzde gösterilen sabit sıra (sunucuda var olanlarla kesişir).
const INDEX_METHODS: [&str; 6] = ["btree", "hash", "gin", "gist", "brin", "spgist"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum LookupRequest {
    Roles,
    Schemas,
    Tablespaces,
    AvailableExtensions,
    IndexMethods,
    Tables { schema: Option<String> },
    TableColumns { schema: String, table: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LookupItem {
    pub value: String,
    pub detail: Option<String>,
}

static ROLES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_roles.sql"))]);
static SCHEMAS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_schemas.sql"))]);
static TABLESPACES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_tablespaces.sql"))]);
static EXTENSIONS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_available_extensions.sql"))]);
static METHODS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_index_methods.sql"))]);
static TABLES: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_tables.sql"))]);
static COLUMNS: VersionedSql = VersionedSql(&[(10, include_str!("sql/lookup_table_columns.sql"))]);

/// Metin parametreleriyle (`$1`, `$2`…; `None` = NULL) çalıştırır.
async fn fetch(mgr: &ConnectionManager, server: Uuid, db: u32, sql: &'static VersionedSql, params: Vec<Option<String>>) -> Result<Vec<Row>> {
    mgr.run(server, Target::Database(db), move |c| {
        let params = params.clone();
        async move {
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
            c.client.query(sql.pick(PgVersion(c.version_num())), &refs).await
        }
    })
    .await
}

fn items(rows: &[Row]) -> Vec<LookupItem> {
    rows.iter().map(|r| LookupItem { value: r.get("value"), detail: r.get("detail") }).collect()
}

pub async fn lookup(mgr: &ConnectionManager, server: Uuid, database_oid: u32, req: LookupRequest) -> Result<Vec<LookupItem>> {
    let db = database_oid;
    Ok(match req {
        LookupRequest::Roles => items(&fetch(mgr, server, db, &ROLES, vec![]).await?),
        LookupRequest::Schemas => items(&fetch(mgr, server, db, &SCHEMAS, vec![]).await?),
        LookupRequest::Tablespaces => items(&fetch(mgr, server, db, &TABLESPACES, vec![]).await?),
        LookupRequest::AvailableExtensions => fetch(mgr, server, db, &EXTENSIONS, vec![])
            .await?
            .iter()
            .map(|r| {
                let installed: Option<String> = r.get("installed_version");
                let comment: Option<String> = r.get("comment");
                let default: Option<String> = r.get("default_version");
                LookupItem {
                    value: r.get("value"),
                    detail: Some(format!(
                        "default {} · yüklü {} · {}",
                        default.unwrap_or_default(),
                        installed.unwrap_or_else(|| "—".to_string()),
                        comment.unwrap_or_default()
                    )),
                }
            })
            .collect(),
        LookupRequest::IndexMethods => {
            let present = items(&fetch(mgr, server, db, &METHODS, vec![]).await?);
            INDEX_METHODS
                .iter()
                .filter(|m| present.iter().any(|p| p.value == **m))
                .map(|m| LookupItem { value: (*m).to_string(), detail: None })
                .collect()
        }
        LookupRequest::Tables { schema } => items(&fetch(mgr, server, db, &TABLES, vec![schema]).await?),
        LookupRequest::TableColumns { schema, table } => items(&fetch(mgr, server, db, &COLUMNS, vec![Some(schema), Some(table)]).await?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_json_shape() {
        let v = serde_json::to_value(LookupRequest::Tables { schema: Some("app".into()) }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "tables", "schema": "app"}));
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "available_extensions"})).unwrap();
        assert_eq!(r, LookupRequest::AvailableExtensions);
        let r: LookupRequest = serde_json::from_value(serde_json::json!({"kind": "table_columns", "schema": "a", "table": "t"})).unwrap();
        assert_eq!(r, LookupRequest::TableColumns { schema: "a".into(), table: "t".into() });
    }
}
