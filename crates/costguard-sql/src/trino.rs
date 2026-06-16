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
        // Trino allows identifiers such as `_updated_at` and `_dstChainId`.
        ch == '_' || self.inner.is_identifier_start(ch)
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

    fn supports_match_recognize(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::parser::Parser;

    #[test]
    fn parses_underscore_identifiers() {
        let dialect = TrinoDialect::default();
        assert!(Parser::parse_sql(&dialect, "SELECT _updated_at FROM t").is_ok());
        assert!(Parser::parse_sql(&dialect, "SELECT s._dstChainId FROM s").is_ok());
    }

    #[test]
    fn parses_match_recognize() {
        let dialect = TrinoDialect::default();
        let sql = "SELECT * FROM t MATCH_RECOGNIZE (PATTERN (A+) DEFINE A AS TRUE)";
        assert!(Parser::parse_sql(&dialect, sql).is_ok());
    }
}
