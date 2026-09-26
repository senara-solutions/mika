//! Read API for operational items.

use crate::db::Database;
use crate::operational::types::{OperationalItem, OperationalItemFilter};
use anyhow::Result;

impl OperationalItem {
    /// Canonical read query — delegates to [`Database::query_operational_items`].
    pub fn query(db: &Database, filter: &OperationalItemFilter) -> Result<Vec<OperationalItem>> {
        db.query_operational_items(filter)
    }
}
