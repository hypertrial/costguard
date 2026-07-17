//! SQL shape feature extraction: regex heuristics and AST traversal.
//!
//! Two extractors run in parallel in [`crate::analyze_sql`]:
//! - **regex** (`regex::extract_features`): text scans; always runs; required when Jinja-heavy
//!   models fail to parse.
//! - **AST** (`ast::extract_shape_features_ast`): sqlparser walk; runs when raw or compiled SQL
//!   parses.
//!
//! [`ast::merge_shape_features`] combines them under this exhaustive contract:
//!
//! | Policy | Fields |
//! | --- | --- |
//! | Regex-only | JSON extraction, regex calls, normalization calls |
//! | AST-authoritative, including empty | top-level `ORDER BY`, non-sargable predicates |
//! | AST-authoritative when `trust_empty_ast` | `SELECT *` |
//! | Prefer nonempty AST | grouping, distinct, windows, CTEs/references, unions, count-distinct, wildcard scans, correlated/scalar/not-in subqueries, wildcard likes, partition ORs, row explosions, recursive CTEs |
//! | Join-specific | AST joins plus regex comma-join evidence, with regex fallback for an empty AST |
//!
//! When parsing fails, regex output is returned unchanged. Tests destructure every
//! [`crate::SqlFeatures`] field so adding a feature requires declaring its policy.

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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MergePolicy {
        RegexOnly,
        AstIncludingEmpty,
        AstWhenTrusted,
        PreferNonemptyAst,
        JoinSpecific,
    }

    const FIELD_POLICIES: [(&str, MergePolicy); 22] = [
        ("select_stars", MergePolicy::AstWhenTrusted),
        ("order_by_clauses", MergePolicy::AstIncludingEmpty),
        ("group_by_clauses", MergePolicy::PreferNonemptyAst),
        ("distincts", MergePolicy::PreferNonemptyAst),
        ("joins", MergePolicy::JoinSpecific),
        ("window_functions", MergePolicy::PreferNonemptyAst),
        ("json_extractions", MergePolicy::RegexOnly),
        ("regex_calls", MergePolicy::RegexOnly),
        ("normalization_calls", MergePolicy::RegexOnly),
        ("ctes", MergePolicy::PreferNonemptyAst),
        ("cte_references", MergePolicy::PreferNonemptyAst),
        ("non_sargable_predicates", MergePolicy::AstIncludingEmpty),
        ("unions_without_all", MergePolicy::PreferNonemptyAst),
        ("count_distincts", MergePolicy::PreferNonemptyAst),
        ("wildcard_table_scans", MergePolicy::PreferNonemptyAst),
        ("correlated_subqueries", MergePolicy::PreferNonemptyAst),
        ("leading_wildcard_likes", MergePolicy::PreferNonemptyAst),
        ("or_partition_predicates", MergePolicy::PreferNonemptyAst),
        (
            "scalar_subqueries_in_select",
            MergePolicy::PreferNonemptyAst,
        ),
        ("row_explosions", MergePolicy::PreferNonemptyAst),
        ("not_in_subqueries", MergePolicy::PreferNonemptyAst),
        ("recursive_ctes", MergePolicy::PreferNonemptyAst),
    ];

    #[derive(Debug, PartialEq, Eq)]
    struct FeatureSnapshot([usize; 22]);

    impl FeatureSnapshot {
        fn from_features(features: &crate::SqlFeatures) -> Self {
            let crate::SqlFeatures {
                select_stars,
                order_by_clauses,
                group_by_clauses,
                distincts,
                joins,
                window_functions,
                json_extractions,
                regex_calls,
                normalization_calls,
                ctes,
                cte_references,
                non_sargable_predicates,
                unions_without_all,
                count_distincts,
                wildcard_table_scans,
                correlated_subqueries,
                leading_wildcard_likes,
                or_partition_predicates,
                scalar_subqueries_in_select,
                row_explosions,
                not_in_subqueries,
                recursive_ctes,
            } = features;
            Self([
                select_stars.len(),
                order_by_clauses.len(),
                group_by_clauses.len(),
                distincts.len(),
                joins.len(),
                window_functions.len(),
                json_extractions.len(),
                regex_calls.len(),
                normalization_calls.len(),
                ctes.len(),
                cte_references.len(),
                non_sargable_predicates.len(),
                unions_without_all.len(),
                count_distincts.len(),
                wildcard_table_scans.len(),
                correlated_subqueries.len(),
                leading_wildcard_likes.len(),
                or_partition_predicates.len(),
                scalar_subqueries_in_select.len(),
                row_explosions.len(),
                not_in_subqueries.len(),
                recursive_ctes.len(),
            ])
        }
    }

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

    fn expression(key: &str) -> crate::ExpressionFeature {
        crate::ExpressionFeature {
            span: costguard_diagnostics::Span {
                byte_start: 0,
                byte_end: 1,
                line: 1,
                column: 1,
                source_provenance: None,
            },
            text: key.into(),
            key: key.into(),
        }
    }

    #[test]
    fn every_sql_feature_has_a_declared_merge_policy() {
        let snapshot = FeatureSnapshot::from_features(&crate::SqlFeatures::default());
        assert_eq!(snapshot.0.len(), FIELD_POLICIES.len());
        let unique = FIELD_POLICIES
            .iter()
            .map(|(field, _)| *field)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), FIELD_POLICIES.len());
    }

    #[test]
    fn merge_policy_classes_are_enforced() {
        let mut regex = crate::SqlFeatures::default();
        regex.select_stars.push(expression("regex-select"));
        regex.order_by_clauses.push(expression("regex-order"));
        regex
            .non_sargable_predicates
            .push(expression("regex-predicate"));
        regex.group_by_clauses.push(expression("regex-group"));
        regex.json_extractions.push(expression("regex-json"));
        regex.regex_calls.push(expression("regex-call"));
        regex
            .normalization_calls
            .push(expression("regex-normalization"));

        let mut ast = crate::SqlFeatures::default();
        ast.group_by_clauses.push(expression("ast-group"));
        ast.json_extractions.push(expression("ast-json"));
        let merged = super::merge_shape_features(regex.clone(), ast, true, true);
        assert!(merged.select_stars.is_empty());
        assert!(merged.order_by_clauses.is_empty());
        assert!(merged.non_sargable_predicates.is_empty());
        assert_eq!(merged.group_by_clauses[0].key, "ast-group");
        assert_eq!(merged.json_extractions[0].key, "regex-json");
        assert_eq!(merged.regex_calls[0].key, "regex-call");
        assert_eq!(merged.normalization_calls[0].key, "regex-normalization");

        let failed =
            super::merge_shape_features(regex.clone(), crate::SqlFeatures::default(), false, true);
        assert_eq!(
            FeatureSnapshot::from_features(&failed),
            FeatureSnapshot::from_features(&regex)
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
