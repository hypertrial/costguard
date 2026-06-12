mod trino;

use serde::{Deserialize, Serialize};
use sqlparser::dialect::{
    BigQueryDialect, DatabricksDialect, DuckDbDialect, GenericDialect, PostgreSqlDialect,
    RedshiftSqlDialect, SnowflakeDialect,
};
use std::fmt;
use std::str::FromStr;
use trino::TrinoDialect;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Generic,
    Snowflake,
    BigQuery,
    Databricks,
    Redshift,
    Postgres,
    DuckDB,
    Trino,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportTier {
    Production,
    Preview,
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" => Ok(Self::Generic),
            "snowflake" => Ok(Self::Snowflake),
            "bigquery" => Ok(Self::BigQuery),
            "databricks" => Ok(Self::Databricks),
            "redshift" => Ok(Self::Redshift),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "duckdb" => Ok(Self::DuckDB),
            "trino" | "presto" => Ok(Self::Trino),
            other => Err(format!("unknown platform '{other}'")),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Snowflake => "snowflake",
            Self::BigQuery => "bigquery",
            Self::Databricks => "databricks",
            Self::Redshift => "redshift",
            Self::Postgres => "postgres",
            Self::DuckDB => "duckdb",
            Self::Trino => "trino",
        }
    }

    pub fn sqlparser_dialect(self) -> Box<dyn sqlparser::dialect::Dialect> {
        match self {
            Self::Generic => Box::new(GenericDialect {}),
            Self::Snowflake => Box::new(SnowflakeDialect {}),
            Self::BigQuery => Box::new(BigQueryDialect {}),
            Self::Databricks => Box::new(DatabricksDialect {}),
            Self::Redshift => Box::new(RedshiftSqlDialect {}),
            Self::Postgres => Box::new(PostgreSqlDialect {}),
            Self::DuckDB => Box::new(DuckDbDialect {}),
            Self::Trino => Box::new(TrinoDialect::default()),
        }
    }

    pub fn support_tier(self) -> SupportTier {
        match self {
            Self::Generic | Self::Snowflake | Self::BigQuery | Self::Trino => {
                SupportTier::Production
            }
            Self::Databricks | Self::Redshift | Self::Postgres | Self::DuckDB => {
                SupportTier::Preview
            }
        }
    }
}

pub type Warehouse = Platform;
pub type SqlDialect = Platform;

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::parser::Parser;

    #[test]
    fn trino_dialect_parses_spellbook_style_sql() {
        let sql = r#"
            select tx_hash, block_time
            from dex.trades
            where block_time >= date_sub(current_date, interval '3' day)
        "#;
        let dialect = Platform::Trino.sqlparser_dialect();
        assert!(Parser::parse_sql(dialect.as_ref(), sql).is_ok());
    }

    #[test]
    fn v1_support_tiers_are_explicit() {
        for platform in [
            Platform::Generic,
            Platform::Snowflake,
            Platform::BigQuery,
            Platform::Trino,
        ] {
            assert_eq!(platform.support_tier(), SupportTier::Production);
        }
        for platform in [
            Platform::Databricks,
            Platform::Redshift,
            Platform::Postgres,
            Platform::DuckDB,
        ] {
            assert_eq!(platform.support_tier(), SupportTier::Preview);
        }
    }
}
