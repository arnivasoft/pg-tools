//! Şema nesnelerinin listesi ve şema denetimi (katalog sorguları `sql/` altında; sürüme göre PG 10 / 11 / 13).

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_ddl::ObjectKind;
use tokio_postgres::Client;

use crate::key::Listed;

static LIST_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/list_objects.pg10.sql")),
    (11, include_str!("sql/list_objects.pg11.sql")),
    (13, include_str!("sql/list_objects.sql")),
]);
const SCHEMA_CHECK_SQL: &str = include_str!("sql/schema_check.sql");

/// Şema denetimi sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaCheck {
    Ok,
    Missing,
    /// `pg_*` ve `information_schema`: karşılaştırılmaz.
    System,
}

pub async fn schema_check(c: &Client, schema: &str) -> Result<SchemaCheck, tokio_postgres::Error> {
    let rows = c.query(SCHEMA_CHECK_SQL, &[&schema]).await?;
    Ok(match rows.first() {
        None => SchemaCheck::Missing,
        Some(r) if r.get::<_, bool>("is_system") => SchemaCheck::System,
        Some(_) => SchemaCheck::Ok,
    })
}

fn kind_of(s: &str) -> Option<ObjectKind> {
    Some(match s {
        "table" => ObjectKind::Table,
        "view" => ObjectKind::View,
        "materialized_view" => ObjectKind::MaterializedView,
        "sequence" => ObjectKind::Sequence,
        "index" => ObjectKind::Index,
        "trigger" => ObjectKind::Trigger,
        "function" => ObjectKind::Function,
        "procedure" => ObjectKind::Procedure,
        "type" => ObjectKind::Type,
        "domain" => ObjectKind::Domain,
        _ => return None,
    })
}

/// Şemadaki karşılaştırılabilir nesneleri `kind, table_name, name` sırasıyla listeler.
pub async fn list_objects(
    c: &Client,
    version_num: u32,
    schema: &str,
) -> Result<Vec<Listed>, tokio_postgres::Error> {
    let rows = c
        .query(LIST_SQL.pick(PgVersion(version_num)), &[&schema])
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let kind = kind_of(&r.get::<_, String>("kind"))?;
            Some(Listed {
                kind,
                oid: r.get("oid"),
                name: r.get("name"),
                table: r.get("table_name"),
                signature: r.get("signature"),
                is_partition: r.get("is_partition"),
                deps: r.get("deps"),
            })
        })
        .collect())
}
