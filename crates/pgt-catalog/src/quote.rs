use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;

const SQL: &str = "SELECT quote_ident(u.n) AS q FROM unnest($1::text[]) WITH ORDINALITY AS u(n, i) ORDER BY u.i";

impl Catalog {
    /// Adları sunucuda `quote_ident` ile tırnaklar (Rust'ta tırnaklama kuralı yoktur). Sıra korunur.
    pub async fn quote_identifiers(&self, server: Uuid, names: Vec<String>) -> Result<Vec<String>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self
            .mgr
            .run(server, Target::Maintenance, move |c| {
                let names = names.clone();
                async move { c.client.query(SQL, &[&names]).await }
            })
            .await?;
        Ok(rows.iter().map(|r| r.get::<_, String>("q")).collect())
    }
}
