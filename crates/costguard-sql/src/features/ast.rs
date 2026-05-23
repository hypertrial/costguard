use crate::strip::JinjaStripMap;
use crate::{CteFeature, ExpressionFeature, JoinFeature, JoinKind, SqlFeatures, WindowFeature};
use costguard_diagnostics::{LineIndex, Span};
use sqlparser::ast::{
    BinaryOperator, DataType, DuplicateTreatment, Expr, Function, FunctionArguments, Join,
    JoinConstraint, JoinOperator, ObjectName, Query, Select, SelectItem, SetExpr, SetOperator,
    SetQuantifier, Statement, TableFactor, TableWithJoins, WindowSpec, WindowType, With,
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
        SetExpr::SetOperation {
            op,
            set_quantifier,
            left,
            right,
        } => {
            if matches!(op, SetOperator::Union)
                && !matches!(
                    set_quantifier,
                    SetQuantifier::All | SetQuantifier::AllByName
                )
            {
                if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, "union")
                {
                    features.unions_without_all.push(ExpressionFeature {
                        span,
                        key: "union".into(),
                        text: "union".into(),
                    });
                }
            }
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

    if let Some(where_expr) = &select.selection {
        extract_non_sargable_predicates(
            where_expr, sanitized, raw, strip_map, line_index, features,
        );
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
    let (predicate, has_equality, function_on_join_key) = match &join.join_operator {
        JoinOperator::Inner(inner)
        | JoinOperator::LeftOuter(inner)
        | JoinOperator::RightOuter(inner)
        | JoinOperator::FullOuter(inner) => match inner {
            JoinConstraint::On(expr) => {
                let predicate = expr.to_string();
                let predicate_lower = predicate.to_ascii_lowercase();
                (
                    Some(predicate),
                    has_equality_predicate(&predicate_lower),
                    join_predicate_has_function_on_key(expr),
                )
            }
            JoinConstraint::Using(ids) => {
                let predicate = format!("USING({ids:?})");
                (Some(predicate), true, false)
            }
            _ => (None, false, false),
        },
        _ => (None, false, false),
    };
    if matches!(kind, JoinKind::Cross) && is_exempt_cross_join_target(&join.relation) {
        extract_table_factor(
            &join.relation,
            sanitized,
            raw,
            strip_map,
            line_index,
            features,
        );
        return;
    }
    if let Some(span) = find_clause_span(sanitized, raw, strip_map, line_index, needle) {
        features.joins.push(JoinFeature {
            span,
            kind,
            predicate,
            has_equality,
            function_on_join_key,
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
        TableFactor::Table { name, .. } => {
            if table_name_has_wildcard(name) && !text_has_table_suffix_bound(sanitized) {
                let table_text = name.to_string();
                if let Some(span) =
                    find_clause_span(sanitized, raw, strip_map, line_index, &table_text)
                {
                    features.wildcard_table_scans.push(ExpressionFeature {
                        span,
                        key: table_text.clone(),
                        text: table_text,
                    });
                }
            }
        }
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
            if is_count_distinct(function) {
                let snippet = function.to_string();
                if let Some(span) =
                    find_clause_span(sanitized, raw, strip_map, line_index, &snippet)
                {
                    features.count_distincts.push(ExpressionFeature {
                        span,
                        key: "count(distinct".into(),
                        text: snippet,
                    });
                }
            }
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

fn extract_non_sargable_predicates(
    expr: &Expr,
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::And | BinaryOperator::Or => {
                extract_non_sargable_predicates(
                    left, sanitized, raw, strip_map, line_index, features,
                );
                extract_non_sargable_predicates(
                    right, sanitized, raw, strip_map, line_index, features,
                );
            }
            BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => {
                for side in [left.as_ref(), right.as_ref()] {
                    if is_non_sargable_filter(side) {
                        let snippet = side.to_string();
                        if let Some(span) =
                            find_clause_span(sanitized, raw, strip_map, line_index, &snippet)
                        {
                            features.non_sargable_predicates.push(ExpressionFeature {
                                span,
                                key: snippet.clone(),
                                text: snippet,
                            });
                        }
                    }
                }
            }
            _ => {}
        },
        Expr::Nested(inner) => {
            extract_non_sargable_predicates(inner, sanitized, raw, strip_map, line_index, features)
        }
        _ => {}
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

fn is_exempt_cross_join_target(factor: &TableFactor) -> bool {
    match factor {
        TableFactor::UNNEST { .. } | TableFactor::TableFunction { .. } => true,
        TableFactor::Function { name, .. } => {
            matches!(
                object_name_last(name).as_str(),
                "unnest" | "flatten" | "table"
            )
        }
        TableFactor::Table { name, .. } => {
            let table = object_name_last(name);
            table == "unnest" || is_date_spine_table(&table)
        }
        _ => false,
    }
}

fn is_date_spine_table(name: &str) -> bool {
    matches!(
        name,
        "check_date" | "date_spine" | "time_seq" | "calendar" | "dates" | "time_dimension"
    )
}

fn object_name_last(name: &ObjectName) -> String {
    name.0
        .last()
        .map(|ident| ident.value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn merge_join_features(base: &[JoinFeature], ast: &[JoinFeature]) -> Vec<JoinFeature> {
    let mut merged: Vec<JoinFeature> = base
        .iter()
        .filter(|join| join.kind != JoinKind::Cross)
        .cloned()
        .collect();
    merged.extend(ast.iter().cloned());
    dedupe_join_features(merged)
}

fn dedupe_join_features(joins: Vec<JoinFeature>) -> Vec<JoinFeature> {
    let mut seen = std::collections::HashSet::new();
    joins
        .into_iter()
        .filter(|join| seen.insert((join.span.byte_start, join.span.byte_end, join.kind as u8)))
        .collect()
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

fn join_predicate_has_function_on_key(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            if is_symmetric_normalization_eq(left, right) || is_time_bucket_join_eq(left, right) {
                return false;
            }
            is_function_wrapped_join_key(left) || is_function_wrapped_join_key(right)
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => join_predicate_has_function_on_key(left) || join_predicate_has_function_on_key(right),
        Expr::Nested(inner) => join_predicate_has_function_on_key(inner),
        _ => false,
    }
}

fn is_symmetric_normalization_eq(left: &Expr, right: &Expr) -> bool {
    match (left, right) {
        (Expr::Function(left_fn), Expr::Function(right_fn)) => {
            let left_name = function_name(left_fn);
            let right_name = function_name(right_fn);
            if left_name != right_name {
                return false;
            }
            matches!(
                left_name.as_str(),
                "lower" | "upper" | "trim" | "ltrim" | "rtrim" | "date_trunc" | "coalesce" | "cast"
            )
        }
        (Expr::Cast { .. }, Expr::Cast { .. }) => true,
        _ => false,
    }
}

fn is_time_bucket_join_eq(left: &Expr, right: &Expr) -> bool {
    is_time_bucket_column_expr(left) && is_time_truncation_expr(right)
        || is_time_bucket_column_expr(right) && is_time_truncation_expr(left)
}

fn is_time_bucket_column_expr(expr: &Expr) -> bool {
    expr_column_name(expr).is_some_and(|name| is_time_bucket_column_name(&name))
}

fn is_time_truncation_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Function(function) => matches!(
            function_name(function).as_str(),
            "date_trunc" | "timestamp_trunc" | "date" | "datetime"
        ),
        Expr::Cast { expr: inner, .. } => !matches!(inner.as_ref(), Expr::Value(_)),
        Expr::Nested(inner) => is_time_truncation_expr(inner),
        _ => false,
    }
}

fn expr_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
        Expr::CompoundIdentifier(parts) => {
            parts.last().map(|ident| ident.value.to_ascii_lowercase())
        }
        _ => None,
    }
}

fn is_time_bucket_column_name(name: &str) -> bool {
    matches!(
        name,
        "minute"
            | "hour"
            | "day"
            | "date"
            | "week"
            | "month"
            | "block_date"
            | "block_day"
            | "evt_block_date"
    )
}

fn is_function_wrapped_join_key(expr: &Expr) -> bool {
    match expr {
        Expr::Function(function) => {
            is_join_key_normalization_function(function) && !function_wraps_literal(function)
        }
        Expr::Cast { expr: inner, .. } => {
            !matches!(inner.as_ref(), Expr::Value(_)) && is_function_wrapped_join_key(inner)
        }
        Expr::Nested(inner) => is_function_wrapped_join_key(inner),
        _ => false,
    }
}

fn function_wraps_literal(function: &Function) -> bool {
    function.args.args().iter().all(|arg| {
        matches!(
            arg,
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
                Expr::Value(_)
            )) | sqlparser::ast::FunctionArg::Named {
                arg: sqlparser::ast::FunctionArgExpr::Expr(Expr::Value(_)),
                ..
            }
        )
    })
}

fn is_join_key_normalization_function(function: &Function) -> bool {
    let name = function_name(function);
    matches!(
        name.as_str(),
        "lower"
            | "upper"
            | "trim"
            | "ltrim"
            | "rtrim"
            | "cast"
            | "date"
            | "date_trunc"
            | "to_char"
            | "to_varchar"
            | "coalesce"
    )
}

fn is_non_sargable_filter(expr: &Expr) -> bool {
    match expr {
        Expr::Function(function) => {
            is_sargability_breaking_function(function)
                && function
                    .args
                    .args()
                    .iter()
                    .any(expr_contains_partition_column_in_arg)
        }
        Expr::Cast {
            expr: inner,
            data_type,
            ..
        } => {
            is_date_like_type(data_type)
                && expr_contains_partition_column(inner)
                && !matches!(inner.as_ref(), Expr::Value(_))
        }
        Expr::Nested(inner) => is_non_sargable_filter(inner),
        _ => false,
    }
}

fn is_sargability_breaking_function(function: &Function) -> bool {
    let name = function_name(function);
    matches!(
        name.as_str(),
        "date" | "timestamp_trunc" | "datetime" | "to_date" | "trunc" | "cast"
    )
}

fn is_count_distinct(function: &Function) -> bool {
    function_name(function) == "count"
        && matches!(function.args, FunctionArguments::List(ref list) if matches!(list.duplicate_treatment, Some(DuplicateTreatment::Distinct)))
}

fn function_name(function: &Function) -> String {
    function
        .name
        .0
        .last()
        .map(|ident| ident.value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn expr_contains_partition_column(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(ident) => is_partition_column_name(&ident.value),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .is_some_and(|ident| is_partition_column_name(&ident.value)),
        Expr::Function(function) => function
            .args
            .args()
            .iter()
            .any(expr_contains_partition_column_in_arg),
        Expr::Cast { expr: inner, .. } => expr_contains_partition_column(inner),
        Expr::Nested(inner) => expr_contains_partition_column(inner),
        _ => false,
    }
}

fn expr_contains_partition_column_in_arg(arg: &sqlparser::ast::FunctionArg) -> bool {
    use sqlparser::ast::FunctionArg;
    match arg {
        FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr))
        | FunctionArg::Named {
            arg: sqlparser::ast::FunctionArgExpr::Expr(expr),
            ..
        } => expr_contains_partition_column(expr),
        _ => false,
    }
}

fn is_partition_column_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "block_time",
        "event_time",
        "created_at",
        "updated_at",
        "event_date",
        "ingested_at",
        "_partitiontime",
        "_partitiondate",
        "partition_date",
        "block_date",
        "block_timestamp",
        "block_number",
        "block_num",
        "evt_block_time",
        "evt_block_number",
        "evt_block_date",
        "block_day",
    ]
    .iter()
    .any(|needle| lower == *needle || lower.ends_with(&format!("_{needle}")))
}

fn is_date_like_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Date | DataType::Datetime(_) | DataType::Timestamp(_, _) | DataType::Time(_, _)
    )
}

fn table_name_has_wildcard(name: &ObjectName) -> bool {
    name.to_string().contains('*')
}

fn text_has_table_suffix_bound(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("_table_suffix")
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
    if !ast.window_functions.is_empty() {
        base.window_functions = ast.window_functions;
    }
    if !ast.ctes.is_empty() {
        base.ctes = ast.ctes;
    }
    if !ast.cte_references.is_empty() {
        base.cte_references = ast.cte_references;
    }
    if !ast.non_sargable_predicates.is_empty() {
        base.non_sargable_predicates = ast.non_sargable_predicates;
    }
    if !ast.unions_without_all.is_empty() {
        base.unions_without_all = ast.unions_without_all;
    }
    if !ast.count_distincts.is_empty() {
        base.count_distincts = ast.count_distincts;
    }
    if !ast.wildcard_table_scans.is_empty() {
        base.wildcard_table_scans = ast.wildcard_table_scans;
    }
    base.joins = merge_join_features(&base.joins, &ast.joins);
    base
}

trait FunctionArgListExt {
    fn args(&self) -> &[sqlparser::ast::FunctionArg];
}

impl FunctionArgListExt for FunctionArguments {
    fn args(&self) -> &[sqlparser::ast::FunctionArg] {
        match self {
            FunctionArguments::List(list) => &list.args,
            _ => &[],
        }
    }
}
