use super::ast::SpanFinder;
use super::join_predicates::FunctionArgListExt;
use crate::{ExpressionFeature, SqlFeatures};
use sqlparser::ast::{BinaryOperator, Expr, Query, Select, SelectItem, SetExpr};

pub(crate) fn extract_not_in_subqueries_in_expr(
    expr: &Expr,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::InSubquery {
            negated: true,
            subquery,
            ..
        } => {
            let snippet = subquery.to_string();
            if let Some(span) = finder.find_next(&snippet) {
                features.not_in_subqueries.push(ExpressionFeature {
                    span,
                    key: "not in subquery".into(),
                    text: snippet,
                });
            }
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And | BinaryOperator::Or,
            right,
        } => {
            extract_not_in_subqueries_in_expr(left, finder, features);
            extract_not_in_subqueries_in_expr(right, finder, features);
        }
        Expr::Nested(inner) => extract_not_in_subqueries_in_expr(inner, finder, features),
        _ => {}
    }
}

pub(crate) fn extract_scalar_subquery_in_select(
    expr: &Expr,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    if matches!(expr, Expr::Subquery(_)) {
        let snippet = expr.to_string();
        if let Some(span) = finder.find_next(&snippet) {
            features
                .scalar_subqueries_in_select
                .push(ExpressionFeature {
                    span,
                    key: "scalar subquery".into(),
                    text: snippet,
                });
        }
    }
}

pub(crate) fn extract_correlated_subqueries_in_expr(
    expr: &Expr,
    outer_aliases: &[String],
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::Exists { subquery, .. } => {
            if subquery_references_outer(subquery, outer_aliases) {
                push_correlated_subquery(subquery, finder, features);
            }
        }
        Expr::InSubquery { subquery, .. } => {
            if subquery_references_outer(subquery, outer_aliases) {
                push_correlated_subquery(subquery, finder, features);
            }
        }
        Expr::Subquery(subquery) => {
            if subquery_references_outer(subquery, outer_aliases) {
                push_correlated_subquery(subquery, finder, features);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            for side in [left.as_ref(), right.as_ref()] {
                if let Expr::Subquery(subquery) = side {
                    if subquery_references_outer(subquery, outer_aliases) {
                        push_correlated_subquery(subquery, finder, features);
                    }
                }
            }
            extract_correlated_subqueries_in_expr(left, outer_aliases, finder, features);
            extract_correlated_subqueries_in_expr(right, outer_aliases, finder, features);
        }
        Expr::Nested(inner) => {
            extract_correlated_subqueries_in_expr(inner, outer_aliases, finder, features);
        }
        _ => {}
    }
}

fn push_correlated_subquery(
    subquery: &Query,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    let snippet = subquery.to_string();
    if let Some(span) = finder.find_next(&snippet) {
        features.correlated_subqueries.push(ExpressionFeature {
            span,
            key: "correlated subquery".into(),
            text: snippet,
        });
    }
}

fn subquery_references_outer(subquery: &Query, outer_aliases: &[String]) -> bool {
    if outer_aliases.is_empty() {
        return false;
    }
    set_expr_references_outer_aliases(subquery.body.as_ref(), outer_aliases)
}

fn set_expr_references_outer_aliases(body: &SetExpr, outer_aliases: &[String]) -> bool {
    match body {
        SetExpr::Select(select) => select_references_outer_aliases(select, outer_aliases),
        SetExpr::Query(query) => {
            set_expr_references_outer_aliases(query.body.as_ref(), outer_aliases)
        }
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_references_outer_aliases(left, outer_aliases)
                || set_expr_references_outer_aliases(right, outer_aliases)
        }
        _ => false,
    }
}

fn select_references_outer_aliases(select: &Select, outer_aliases: &[String]) -> bool {
    select
        .selection
        .as_ref()
        .is_some_and(|expr| expr_references_outer_aliases(expr, outer_aliases))
        || select.projection.iter().any(|item| match item {
            SelectItem::ExprWithAlias { expr, .. } | SelectItem::UnnamedExpr(expr) => {
                expr_references_outer_aliases(expr, outer_aliases)
            }
            _ => false,
        })
}

fn expr_references_outer_aliases(expr: &Expr, outer_aliases: &[String]) -> bool {
    match expr {
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => outer_aliases
            .iter()
            .any(|alias| alias == &parts[0].value.to_ascii_lowercase()),
        Expr::BinaryOp { left, right, .. } => {
            expr_references_outer_aliases(left, outer_aliases)
                || expr_references_outer_aliases(right, outer_aliases)
        }
        Expr::Nested(inner) => expr_references_outer_aliases(inner, outer_aliases),
        Expr::Exists { subquery, .. } | Expr::InSubquery { subquery, .. } => {
            subquery_references_outer(subquery, outer_aliases)
        }
        Expr::Subquery(subquery) => subquery_references_outer(subquery, outer_aliases),
        Expr::Function(function) => function
            .args
            .args()
            .iter()
            .any(|arg| function_arg_references_outer_aliases(arg, outer_aliases)),
        _ => false,
    }
}

fn function_arg_references_outer_aliases(
    arg: &sqlparser::ast::FunctionArg,
    outer_aliases: &[String],
) -> bool {
    use sqlparser::ast::{FunctionArg, FunctionArgExpr};
    match arg {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
        | FunctionArg::Named {
            arg: FunctionArgExpr::Expr(expr),
            ..
        } => expr_references_outer_aliases(expr, outer_aliases),
        _ => false,
    }
}
