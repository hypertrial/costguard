use serde::{Deserialize, Serialize};
use sqlparser::dialect::{
    BigQueryDialect, DatabricksDialect, DuckDbDialect, GenericDialect, PostgreSqlDialect,
    RedshiftSqlDialect, SnowflakeDialect,
};
use std::fmt;
use std::str::FromStr;

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
        }
    }
}

pub type Warehouse = Platform;
pub type SqlDialect = Platform;
