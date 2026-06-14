//! SQLCOST rule implementations and registry.
//!
//! Each rule implements the [`Rule`] trait and inspects a [`RuleContext`] built
//! from parsed SQL, dbt model metadata, and project indexes. The default
//! [`RuleRegistry`] registers all 44 built-in SQLCOST rules.

mod cost_patterns;
mod cross_file;
mod dbt;
mod expressions;
mod helpers;
mod metadata;
mod python;
mod registry;
mod sql_shape;

pub use registry::{
    Platform, ProjectIndexes, Rule, RuleContext, RuleMetadata, RuleOverride, RuleOverrides,
    RuleRegistry,
};

#[cfg(test)]
mod tests;
