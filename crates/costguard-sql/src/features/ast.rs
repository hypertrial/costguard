use crate::strip::JinjaStripMap;
use crate::{CteFeature, ExpressionFeature, JoinFeature, JoinKind, SqlFeatures, WindowFeature};
use costguard_diagnostics::{LineIndex, Span};
use sqlparser::ast::{
    Expr, Function, Join, JoinConstraint, JoinOperator, Query, Select, SelectItem, SetExpr,
    Statement, TableFactor, TableWithJoins, WindowSpec, WindowType, With,
};

pub fn extract_shape_features_ast(
    statements: &[Statement],
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
) -> SqlFeatures {
    let mut features = SqlFeatures::default();
    for statement in statements {
        if let Statement::Query(query) = statement {
            extract_query(query, sanitized, raw, strip_map, line_index, &mut features);
        }
    }
    features.cte_references =
        extract_cte_references_from_names(&features.ctes, sanitized, raw, strip_map, line_index);
    features
}

fn extract_query(
    query: &Query,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    if let Some(with) = &query.with {
        extract_with(with, sanitized, raw, strip_map, line_index, features);
    }
    extract_set_expr(&query.body, sanitized, raw, strip_map, line_index, features);
    if query.order_by.is_some() {
        if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, "order by") {
            features.order_by_clauses.push(ExpressionFeature {
                span,
                key: "order by".into(),
                text: "order by".into(),
            });
        }
    }
}

fn extract_with(
    with: &With,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    for cte in &with.cte_tables {
        let name = cte.alias.name.value.to_ascii_lowercase();
        if let Some(span) = find_word_span(sanitized, raw, strip_map, line_index, &name) {
            features.ctes.push(CteFeature { name, span });
        }
    }
}

fn extract_set_expr(
    body: &SetExpr,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    match body {
        SetExpr::Select(select) => {
            extract_select(select, sanitized, raw, strip_map, line_index, features)
        }
        SetExpr::Query(query) => {
            extract_query(query, sanitized, raw, strip_map, line_index, features)
        }
        SetExpr::SetOperation { left, right, .. } => {
            extract_set_expr(left, sanitized, raw, strip_map, line_index, features);
            extract_set_expr(right, sanitized, raw, strip_map, line_index, features);
        }
        _ => {}
    }
}

fn extract_select(
    select: &Select,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    if select.distinct.is_some() {
        if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, "distinct") {
            features.distincts.push(ExpressionFeature {
                span,
                key: "select distinct".into(),
                text: "select distinct".into(),
            });
        }
    }

    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, "*") {
                    features.select_stars.push(ExpressionFeature {
                        span,
                        key: "select *".into(),
                        text: "*".into(),
                    });
                }
            }
            SelectItem::ExprWithAlias { expr, .. } | SelectItem::UnnamedExpr(expr) => {
                extract_expr(expr, sanitized, raw, strip_map, line_index, features);
            }
        }
    }

    for table in &select.from {
        extract_table_with_joins(table, sanitized, raw, strip_map, line_index, features);
    }
}

fn extract_table_with_joins(
    table: &TableWithJoins,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    extract_table_factor(
        &table.relation,
        sanitized,
        raw,
        strip_map,
        line_index,
        features,
    );
    for join in &table.joins {
        extract_join(join, sanitized, raw, strip_map, line_index, features);
    }
}

fn extract_join(
    join: &Join,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    let kind = match &join.join_operator {
        JoinOperator::CrossJoin | JoinOperator::CrossApply => JoinKind::Cross,
        JoinOperator::LeftOuter(_) => JoinKind::Left,
        JoinOperator::RightOuter(_) => JoinKind::Right,
        JoinOperator::FullOuter(_) => JoinKind::Full,
        _ => JoinKind::Inner,
    };
    let needle = match kind {
        JoinKind::Cross => "cross join",
        JoinKind::Left => "left join",
        JoinKind::Right => "right join",
        JoinKind::Full => "full join",
        JoinKind::Inner | JoinKind::Comma => " join",
    };
    let predicate = match &join.join_operator {
        JoinOperator::Inner(inner)
        | JoinOperator::LeftOuter(inner)
        | JoinOperator::RightOuter(inner)
        | JoinOperator::FullOuter(inner) => match inner {
            JoinConstraint::On(expr) => Some(expr.to_string()),
            JoinConstraint::Using(ids) => Some(format!("USING({ids:?})")),
            _ => None,
        },
        _ => None,
    };
    let predicate_lower = predicate
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, needle) {
        features.joins.push(JoinFeature {
            span,
            kind,
            predicate,
            has_equality: has_equality_predicate(&predicate_lower),
            function_on_both_sides: function_on_both_sides(&predicate_lower),
        });
    }
    extract_table_factor(
        &join.relation,
        sanitized,
        raw,
        strip_map,
        line_index,
        features,
    );
}

fn extract_table_factor(
    factor: &TableFactor,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    match factor {
        TableFactor::Derived { subquery, .. } => {
            extract_set_expr(
                subquery.body.as_ref(),
                sanitized,
                raw,
                strip_map,
                line_index,
                features,
            );
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => extract_table_with_joins(
            table_with_joins,
            sanitized,
            raw,
            strip_map,
            line_index,
            features,
        ),
        _ => {}
    }
}

fn extract_expr(
    expr: &Expr,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::Function(function) => {
            extract_function(function, sanitized, raw, strip_map, line_index, features)
        }
        Expr::Nested(inner) => extract_expr(inner, sanitized, raw, strip_map, line_index, features),
        _ => {}
    }
}

fn extract_function(
    function: &Function,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    if let Some(window) = &function.over {
        match window {
            WindowType::WindowSpec(spec) => extract_window(
                spec, function, sanitized, raw, strip_map, line_index, features,
            ),
            WindowType::NamedWindow(_) => {
                if let Some(span) =
                    find_clause_span(sanitized, raw, strip_map, line_index, "over (")
                {
                    features.window_functions.push(WindowFeature {
                        span,
                        text: "over (...)".into(),
                        has_partition_by: false,
                    });
                }
            }
        }
    }
}

fn extract_window(
    window: &WindowSpec,
    function: &Function,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    let snippet = format!("{} over", function.name).to_ascii_lowercase();
    if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, &snippet) {
        features.window_functions.push(WindowFeature {
            span,
            text: snippet,
            has_partition_by: !window.partition_by.is_empty(),
        });
    } else if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, "over (") {
        features.window_functions.push(WindowFeature {
            span,
            text: "over (...)".into(),
            has_partition_by: !window.partition_by.is_empty(),
        });
    }
}

fn extract_cte_references_from_names(
    ctes: &[CteFeature],
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
) -> Vec<ExpressionFeature> {
    let mut references = Vec::new();
    for cte in ctes {
        if let Some(span) = find_word_span(sanitized, raw, strip_map, line_index, &cte.name) {
            if span.byte_start != cte.span.byte_start {
                references.push(ExpressionFeature {
                    span,
                    key: cte.name.clone(),
                    text: cte.name.clone(),
                });
            }
        }
    }
    references
}

fn find_clause_span(
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    needle: &str,
) -> Option<Span> {
    let lower_sanitized = sanitized.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let start = lower_sanitized.find(&lower_needle)?;
    let end = start + lower_needle.len();
    let (raw_start, raw_end) = strip_map.map_sanitized_range(start, end)?;
    Some(line_index.span(raw_start, raw_end.min(raw.len())))
}

fn find_word_span(
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    word: &str,
) -> Option<Span> {
    find_clause_span(sanitized, raw, strip_map, line_index, word)
}

fn has_equality_predicate(predicate: &str) -> bool {
    predicate.contains('=') && !predicate.contains(">=") && !predicate.contains("<=")
}

fn function_on_both_sides(predicate: &str) -> bool {
    predicate.contains('(') && predicate.contains('=')
}

pub fn merge_shape_features(mut base: SqlFeatures, ast: SqlFeatures, parsed: bool) -> SqlFeatures {
    if !parsed {
        return base;
    }
    if !ast.select_stars.is_empty() {
        base.select_stars = ast.select_stars;
    }
    if !ast.order_by_clauses.is_empty() {
        base.order_by_clauses = ast.order_by_clauses;
    }
    if !ast.distincts.is_empty() {
        base.distincts = ast.distincts;
    }
    if !ast.joins.is_empty() {
        base.joins = ast.joins;
    }
    if !ast.window_functions.is_empty() {
        base.window_functions = ast.window_functions;
    }
    if !ast.ctes.is_empty() {
        base.ctes = ast.ctes;
    }
    if !ast.cte_references.is_empty() {
        base.cte_references = ast.cte_references;
    }
    base
}
