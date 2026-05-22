//! Trino/Presto SQL uses Hive-family parser behavior in sqlparser.
use sqlparser::dialect::{Dialect, HiveDialect};
use std::any::TypeId;

/// Trino/Presto dialect: Hive-family parsing with a stable extension point.
#[derive(Debug)]
pub struct TrinoDialect {
    inner: HiveDialect,
}

impl TrinoDialect {
    pub fn new() -> Self {
        Self {
            inner: HiveDialect {},
        }
    }
}

impl Default for TrinoDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialect for TrinoDialect {
    fn dialect(&self) -> TypeId {
        TypeId::of::<TrinoDialect>()
    }

    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        self.inner.is_delimited_identifier_start(ch)
    }

    fn is_identifier_start(&self, ch: char) -> bool {
        self.inner.is_identifier_start(ch)
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        self.inner.is_identifier_part(ch)
    }

    fn supports_filter_during_aggregation(&self) -> bool {
        self.inner.supports_filter_during_aggregation()
    }

    fn supports_numeric_prefix(&self) -> bool {
        self.inner.supports_numeric_prefix()
    }

    fn require_interval_qualifier(&self) -> bool {
        self.inner.require_interval_qualifier()
    }
}
