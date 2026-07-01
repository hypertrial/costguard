//! SQL shape feature extraction: regex heuristics and AST traversal.
//!
//! Two extractors run in parallel in [`crate::analyze_sql`]:
//! - **regex** (`regex::extract_features`): text scans; always runs; required when Jinja-heavy
//!   models fail to parse.
//! - **AST** (`ast::extract_shape_features_ast`): sqlparser walk; runs when raw or compiled SQL
//!   parses.
//!
//! [`ast::merge_shape_features`] combines them. When `parsed` is true, most fields prefer AST
//! counts when non-empty; `select_stars` can be replaced even when empty if `trust_empty_ast`;
//! `non_sargable_predicates` always comes from AST when parsed (regex also matches JOIN ON
//! predicates AST intentionally skips); joins merge comma-join regex hits with AST joins.
//! When `parsed` is false, regex output is returned unchanged.

pub mod ast;
pub mod comma_join;
pub mod join_heuristics;
mod join_predicates;
pub mod regex;
mod subquery;

pub use ast::{extract_shape_features_ast, merge_shape_features};
pub(crate) use regex::extract_features;

#[cfg(test)]
mod parity_tests {
    use super::{extract_features, extract_shape_features_ast};
    use crate::strip::strip_jinja_with_map;
    use crate::Platform;
    use costguard_diagnostics::LineIndex;
    use sqlparser::parser::Parser;

    #[derive(Debug, PartialEq, Eq)]
    struct FeatureCounts {
        joins: usize,
        ctes: usize,
        window_functions: usize,
        select_stars: usize,
        subqueries: usize,
    }

    impl FeatureCounts {
        fn from_features(features: &crate::SqlFeatures) -> Self {
            Self {
                joins: features.joins.len(),
                ctes: features.ctes.len(),
                window_functions: features.window_functions.len(),
                select_stars: features.select_stars.len(),
                subqueries: features.correlated_subqueries.len()
                    + features.scalar_subqueries_in_select.len()
                    + features.not_in_subqueries.len(),
            }
        }
    }

    fn both_extractors(sql: &str) -> (FeatureCounts, FeatureCounts) {
        let line_index = LineIndex::new(sql);
        let (sanitized, strip_map) = strip_jinja_with_map(sql);
        let parsed_raw =
            Parser::parse_sql(Platform::Generic.sqlparser_dialect().as_ref(), &sanitized).is_ok();
        let regex = extract_features(sql, &sanitized, &line_index, parsed_raw);
        let statements =
            Parser::parse_sql(Platform::Generic.sqlparser_dialect().as_ref(), &sanitized)
                .expect("fixture must parse");
        let ast = extract_shape_features_ast(&statements, &sanitized, sql, &strip_map, &line_index);
        (
            FeatureCounts::from_features(&regex),
            FeatureCounts::from_features(&ast),
        )
    }

    fn assert_parity(sql: &str) {
        let (regex, ast) = both_extractors(sql);
        assert_eq!(
            regex, ast,
            "regex and AST feature counts diverged for:\n{sql}"
        );
    }

    #[test]
    fn regex_and_ast_agree_on_select_star_and_window() {
        assert_parity(
            "select *, row_number() over (partition by id order by ts) as rn from orders",
        );
    }

    #[test]
    fn regex_and_ast_agree_on_cte() {
        assert_parity("with cte as (select 1 as x) select * from cte");
    }

    #[test]
    fn regex_and_ast_agree_on_inner_join() {
        assert_parity("select a.id from orders a inner join customers b on a.customer_id = b.id");
    }

    #[test]
    fn regex_and_ast_agree_on_not_in_subquery() {
        assert_parity("select id from orders where id not in (select order_id from refunds)");
    }
}
