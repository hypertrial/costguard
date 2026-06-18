use super::join_heuristics::{has_equality_predicate, is_date_spine_table};
use super::join_predicates::{
    expr_column_name, function_name, is_symmetric_wrapped_join_equality,
    join_predicate_has_function_on_key, FunctionArgListExt,
};
use super::subquery::{
    extract_correlated_subqueries_in_expr, extract_not_in_subqueries_in_expr,
    extract_scalar_subquery_in_select,
};
use crate::strip::JinjaStripMap;
use crate::{CteFeature, ExpressionFeature, JoinFeature, JoinKind, SqlFeatures, WindowFeature};
use costguard_diagnostics::{LineIndex, Span};
use sqlparser::ast::{
    BinaryOperator, DuplicateTreatment, Expr, Function, FunctionArguments, GroupByExpr, Join,
    JoinConstraint, JoinOperator, ObjectName, ObjectNamePart, Query, Select, SelectItem, SetExpr,
    SetOperator, SetQuantifier, Statement, TableFactor, TableWithJoins, Value, ValueWithSpan,
    WindowFrameBound, WindowSpec, WindowType, With,
};
use std::collections::HashMap;

pub fn extract_shape_features_ast(
    statements: &[Statement],
    sanitized: &str,
    raw: &str,
    strip_map: &JinjaStripMap,
    line_index: &LineIndex,
) -> SqlFeatures {
    let mut features = SqlFeatures::default();
    let mut finder = SpanFinder::new(sanitized, raw, strip_map, line_index);
    for statement in statements {
        if let Statement::Query(query) = statement {
            extract_query(query, &mut finder, &mut features);
        }
    }
    features.cte_references = filter_cte_table_references(&features.ctes, &features.cte_references);
    features
}

fn extract_query(query: &Query, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    if let Some(with) = &query.with {
        extract_with(with, finder, features);
    }
    extract_set_expr(&query.body, finder, features);
    if query.order_by.is_some() {
        if let Some(span) = finder.find_next("order by") {
            features.order_by_clauses.push(ExpressionFeature {
                span,
                key: "order by".into(),
                text: "order by".into(),
            });
        }
    }
}

fn extract_with(with: &With, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    if with.recursive {
        if let Some(span) = finder.find_next("recursive") {
            features.recursive_ctes.push(ExpressionFeature {
                span,
                key: "with recursive".into(),
                text: "with recursive".into(),
            });
        }
    }
    for cte in &with.cte_tables {
        let name = cte.alias.name.value.to_ascii_lowercase();
        if let Some(span) = finder.find_word_next(&name) {
            features.ctes.push(CteFeature { name, span });
        }
        extract_query(&cte.query, finder, features);
    }
}

fn extract_set_expr(body: &SetExpr, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    match body {
        SetExpr::Select(select) => extract_select(select, finder, features),
        SetExpr::Query(query) => extract_query(query, finder, features),
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
                if let Some(span) = finder.find_next("union") {
                    features.unions_without_all.push(ExpressionFeature {
                        span,
                        key: "union".into(),
                        text: "union".into(),
                    });
                }
            }
            extract_set_expr(left, finder, features);
            extract_set_expr(right, finder, features);
        }
        _ => {}
    }
}

fn extract_select(select: &Select, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    if select.distinct.is_some() {
        if let Some(span) = finder.find_next("distinct") {
            features.distincts.push(ExpressionFeature {
                span,
                key: "select distinct".into(),
                text: "select distinct".into(),
            });
        }
    }

    if select_has_group_by(select) {
        if let Some(span) = finder.find_next("group by") {
            features.group_by_clauses.push(ExpressionFeature {
                span,
                key: "group by".into(),
                text: "group by".into(),
            });
        }
    }

    if let Some(where_expr) = &select.selection {
        extract_non_sargable_predicates(where_expr, finder, features);
        extract_leading_wildcard_likes(where_expr, finder, features);
        extract_or_partition_predicates(where_expr, finder, features);
        extract_not_in_subqueries_in_expr(where_expr, finder, features);
        extract_correlated_subqueries_in_expr(
            where_expr,
            &collect_table_aliases(select),
            finder,
            features,
        );
    }

    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => {
                if let Some(span) = finder.find_next("*") {
                    features.select_stars.push(ExpressionFeature {
                        span,
                        key: "select *".into(),
                        text: "*".into(),
                    });
                }
            }
            SelectItem::ExprWithAlias { expr, .. }
            | SelectItem::ExprWithAliases { expr, .. }
            | SelectItem::UnnamedExpr(expr) => {
                extract_scalar_subquery_in_select(expr, finder, features);
                extract_expr(expr, finder, features);
            }
        }
    }

    for table in &select.from {
        extract_table_with_joins(table, finder, features);
    }
}

fn select_has_group_by(select: &Select) -> bool {
    match &select.group_by {
        GroupByExpr::All(exprs) => !exprs.is_empty(),
        GroupByExpr::Expressions(exprs, _) => !exprs.is_empty(),
    }
}

fn extract_table_with_joins(
    table: &TableWithJoins,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    let mut left_catalog = table_factor_catalog(&table.relation);
    let from_min = finder
        .find_next("from")
        .map(|span| span.byte_end)
        .unwrap_or(0);
    extract_table_factor(&table.relation, finder, features, from_min);
    for join in &table.joins {
        let right_catalog = table_factor_catalog(&join.relation);
        let cross_catalog = catalogs_differ(left_catalog.as_deref(), right_catalog.as_deref());
        extract_join(join, finder, features, cross_catalog);
        left_catalog = right_catalog.or(left_catalog);
    }
}

fn extract_join(
    join: &Join,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
    cross_catalog: bool,
) {
    let kind = match &join.join_operator {
        JoinOperator::CrossJoin(_) | JoinOperator::CrossApply => JoinKind::Cross,
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
    let right_relation = table_factor_relation_name(&join.relation);
    let (predicate, has_equality, function_on_join_key, pattern_matching, equality_keys) =
        match &join.join_operator {
            JoinOperator::Join(inner)
            | JoinOperator::Inner(inner)
            | JoinOperator::LeftOuter(inner)
            | JoinOperator::RightOuter(inner)
            | JoinOperator::FullOuter(inner) => match inner {
                JoinConstraint::On(expr) => {
                    let predicate = expr.to_string();
                    let predicate_lower = predicate.to_ascii_lowercase();
                    extract_leading_wildcard_likes(expr, finder, features);
                    extract_not_in_subqueries_in_expr(expr, finder, features);
                    extract_correlated_subqueries_in_expr(
                        expr,
                        &outer_aliases_for_join(&join.relation),
                        finder,
                        features,
                    );
                    (
                        Some(predicate.clone()),
                        has_equality_predicate(&predicate_lower),
                        join_predicate_has_function_on_key(expr),
                        predicate_is_pattern_matching(expr),
                        join_equality_keys(expr),
                    )
                }
                JoinConstraint::Using(ids) => {
                    let predicate = format!("USING({ids:?})");
                    let keys = ids.iter().map(object_name_last).collect();
                    (Some(predicate), true, false, false, keys)
                }
                _ => (None, false, false, false, Vec::new()),
            },
            _ => (None, false, false, false, Vec::new()),
        };
    if matches!(kind, JoinKind::Cross) && is_exempt_cross_join_target(&join.relation) {
        let join_min = finder
            .find_next(needle)
            .map(|span| span.byte_end)
            .unwrap_or(0);
        extract_table_factor(&join.relation, finder, features, join_min);
        return;
    }
    let span = if kind == JoinKind::Inner {
        finder.find_word_next("join")
    } else {
        finder.find_next(needle)
    };
    if let Some(span) = span {
        features.joins.push(JoinFeature {
            span,
            kind,
            predicate,
            has_equality,
            function_on_join_key,
            pattern_matching,
            cross_catalog,
            right_relation,
            equality_keys,
        });
        extract_table_factor(&join.relation, finder, features, span.byte_end);
    } else {
        extract_table_factor(&join.relation, finder, features, 0);
    }
}

fn extract_table_factor(
    factor: &TableFactor,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
    min_byte: usize,
) {
    if is_row_explosion_factor(factor) {
        let text = row_explosion_text(factor);
        if let Some(span) = finder.find_next(&text) {
            features.row_explosions.push(ExpressionFeature {
                span,
                key: text.clone(),
                text,
            });
        }
    }
    match factor {
        TableFactor::Table { name, .. } => {
            record_single_part_table_reference(name, finder, features, min_byte);
            if table_name_has_wildcard(name) && !finder.text_has_table_suffix_bound() {
                let table_text = name.to_string();
                if let Some(span) = finder.find_next(&table_text) {
                    features.wildcard_table_scans.push(ExpressionFeature {
                        span,
                        key: table_text.clone(),
                        text: table_text,
                    });
                }
            }
        }
        TableFactor::Derived { subquery, .. } => {
            extract_set_expr(subquery.body.as_ref(), finder, features);
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => extract_table_with_joins(table_with_joins, finder, features),
        _ => {}
    }
}

fn extract_expr(expr: &Expr, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    match expr {
        Expr::Function(function) => {
            if is_count_distinct(function) {
                let snippet = function.to_string();
                if let Some(span) = finder.find_next(&snippet) {
                    features.count_distincts.push(ExpressionFeature {
                        span,
                        key: "count(distinct".into(),
                        text: snippet,
                    });
                }
            }
            extract_function(function, finder, features)
        }
        Expr::Nested(inner) => extract_expr(inner, finder, features),
        _ => {}
    }
}

fn extract_function(function: &Function, finder: &mut SpanFinder<'_>, features: &mut SqlFeatures) {
    if let Some(window) = &function.over {
        match window {
            WindowType::WindowSpec(spec) => extract_window(spec, function, finder, features),
            WindowType::NamedWindow(_) => {
                if let Some(span) = finder.find_next("over (") {
                    features.window_functions.push(WindowFeature {
                        span,
                        text: "over (...)".into(),
                        has_partition_by: false,
                        unbounded_frame: false,
                    });
                }
            }
        }
    }
}

fn extract_non_sargable_predicates(
    expr: &Expr,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::And | BinaryOperator::Or => {
                extract_non_sargable_predicates(left, finder, features);
                extract_non_sargable_predicates(right, finder, features);
            }
            BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => {
                for side in [left.as_ref(), right.as_ref()] {
                    if is_non_sargable_filter(side, Some(op)) {
                        let snippet = side.to_string();
                        let raw_after = features
                            .non_sargable_predicates
                            .last()
                            .map(|feature| feature.span.byte_end)
                            .unwrap_or(0);
                        if let Some(span) = finder.find_after(raw_after, &snippet) {
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
        Expr::Nested(inner) => extract_non_sargable_predicates(inner, finder, features),
        _ => {}
    }
}

fn extract_window(
    window: &WindowSpec,
    function: &Function,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    let snippet = function.name.to_string().to_ascii_lowercase();
    let unbounded_frame = window
        .window_frame
        .as_ref()
        .is_some_and(is_unbounded_window_frame);
    if let Some(span) = finder.find_next(&snippet) {
        features.window_functions.push(WindowFeature {
            span,
            text: snippet,
            has_partition_by: !window.partition_by.is_empty(),
            unbounded_frame,
        });
    } else if let Some(span) = finder.find_next("over (") {
        features.window_functions.push(WindowFeature {
            span,
            text: "over (...)".into(),
            has_partition_by: !window.partition_by.is_empty(),
            unbounded_frame,
        });
    }
}

fn record_single_part_table_reference(
    name: &ObjectName,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
    min_byte: usize,
) {
    if name.0.len() != 1 {
        return;
    }
    let ident = object_name_part_ident(&name.0[0]);
    // ponytail: min_byte skips homonyms before the current FROM/JOIN; table-ref cursor is separate from CTE-name search
    let span = if min_byte > 0 {
        finder.find_word_after(&ident, min_byte)
    } else {
        finder.find_word_next_table_ref(&ident)
    };
    if let Some(span) = span {
        features.cte_references.push(ExpressionFeature {
            span,
            key: ident.clone(),
            text: ident,
        });
    }
}

fn filter_cte_table_references(
    ctes: &[CteFeature],
    table_refs: &[ExpressionFeature],
) -> Vec<ExpressionFeature> {
    table_refs
        .iter()
        .filter(|reference| {
            ctes.iter().any(|cte| {
                cte.name == reference.key && reference.span.byte_start > cte.span.byte_end
            })
        })
        .cloned()
        .collect()
}

fn is_exempt_cross_join_target(factor: &TableFactor) -> bool {
    is_row_explosion_factor(factor)
        || match factor {
            TableFactor::Table { name, .. } => {
                let table = object_name_last(name);
                is_date_spine_table(&table)
            }
            _ => false,
        }
}

fn is_row_explosion_factor(factor: &TableFactor) -> bool {
    match factor {
        TableFactor::UNNEST { .. } | TableFactor::TableFunction { .. } => true,
        TableFactor::Function { name, .. } => {
            matches!(
                object_name_last(name).as_str(),
                "unnest" | "flatten" | "table"
            )
        }
        TableFactor::Table { name, .. } => object_name_last(name) == "unnest",
        _ => false,
    }
}

fn row_explosion_text(factor: &TableFactor) -> String {
    match factor {
        TableFactor::UNNEST { .. } => "unnest".into(),
        TableFactor::TableFunction { .. } => "table function".into(),
        TableFactor::Function { name, .. } => object_name_last(name),
        TableFactor::Table { name, .. } => object_name_last(name),
        _ => "row explosion".into(),
    }
}

fn table_factor_relation_name(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, alias, .. } => {
            if name.0.len() == 1 {
                Some(object_name_part_ident(&name.0[0]))
            } else {
                alias
                    .as_ref()
                    .map(|alias| alias.name.value.to_ascii_lowercase())
                    .or_else(|| name.0.last().map(object_name_part_ident))
            }
        }
        TableFactor::Derived {
            alias: Some(alias), ..
        } => Some(alias.name.value.to_ascii_lowercase()),
        _ => None,
    }
}

fn join_equality_keys(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            if is_symmetric_wrapped_join_equality(left, right) {
                return vec!["wrapped_equality".into()];
            }
            let mut keys = Vec::new();
            if let Some(key) = expr_column_name(left) {
                keys.push(key);
            }
            if let Some(key) = expr_column_name(right) {
                keys.push(key);
            }
            keys
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut keys = join_equality_keys(left);
            keys.extend(join_equality_keys(right));
            keys
        }
        Expr::Nested(inner) => join_equality_keys(inner),
        _ => Vec::new(),
    }
}

fn is_unbounded_window_frame(frame: &sqlparser::ast::WindowFrame) -> bool {
    matches!(
        frame.start_bound,
        WindowFrameBound::Preceding(None) | WindowFrameBound::Following(None)
    ) && matches!(
        frame.end_bound,
        Some(WindowFrameBound::Preceding(None) | WindowFrameBound::Following(None))
    )
}

fn object_name_last(name: &ObjectName) -> String {
    name.0.last().map(object_name_part_ident).unwrap_or_default()
}

fn object_name_part_ident(part: &ObjectNamePart) -> String {
    part.as_ident()
        .map(|ident| ident.value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn merge_join_features(base: &[JoinFeature], ast: &[JoinFeature]) -> Vec<JoinFeature> {
    let mut merged: Vec<JoinFeature> = base
        .iter()
        .filter(|join| join.kind == JoinKind::Comma)
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

pub(crate) struct SpanFinder<'a> {
    lower_sanitized: String,
    raw: &'a str,
    strip_map: &'a JinjaStripMap,
    line_index: &'a LineIndex,
    cursors: HashMap<String, usize>,
}

impl<'a> SpanFinder<'a> {
    fn new(
        sanitized: &str,
        raw: &'a str,
        strip_map: &'a JinjaStripMap,
        line_index: &'a LineIndex,
    ) -> Self {
        Self {
            lower_sanitized: sanitized.to_ascii_lowercase(),
            raw,
            strip_map,
            line_index,
            cursors: HashMap::new(),
        }
    }

    pub(crate) fn find_next(&mut self, needle: &str) -> Option<Span> {
        let lower_needle = needle.to_ascii_lowercase();
        let start_from = self.cursors.get(&lower_needle).copied().unwrap_or(0);
        let (start, span) = self.find_sanitized_from(&lower_needle, start_from, |_, _| true)?;
        self.cursors
            .insert(lower_needle.clone(), start + lower_needle.len());
        Some(span)
    }

    fn find_after(&self, raw_after: usize, needle: &str) -> Option<Span> {
        let lower_needle = needle.to_ascii_lowercase();
        self.find_sanitized_from(&lower_needle, 0, |raw_start, _| raw_start >= raw_after)
            .map(|(_, span)| span)
    }

    fn find_word_next(&mut self, word: &str) -> Option<Span> {
        let lower_word = word.to_ascii_lowercase();
        let cursor_key = format!("word:{lower_word}");
        let start_from = self.cursors.get(&cursor_key).copied().unwrap_or(0);
        let (start, span) =
            self.find_sanitized_from(&lower_word, start_from, |raw_start, raw_end| {
                self.word_boundaries(raw_start, raw_end)
            })?;
        self.cursors.insert(cursor_key, start + lower_word.len());
        Some(span)
    }

    fn find_word_next_table_ref(&mut self, word: &str) -> Option<Span> {
        let lower_word = word.to_ascii_lowercase();
        let cursor_key = format!("table_ref:{lower_word}");
        let start_from = self.cursors.get(&cursor_key).copied().unwrap_or(0);
        let (start, span) =
            self.find_sanitized_from(&lower_word, start_from, |raw_start, raw_end| {
                self.word_boundaries(raw_start, raw_end)
            })?;
        self.cursors.insert(cursor_key, start + lower_word.len());
        Some(span)
    }

    fn find_word_after(&self, word: &str, raw_after: usize) -> Option<Span> {
        let lower_word = word.to_ascii_lowercase();
        self.find_sanitized_from(&lower_word, 0, |raw_start, raw_end| {
            raw_start >= raw_after && self.word_boundaries(raw_start, raw_end)
        })
        .map(|(_, span)| span)
    }

    fn text_has_table_suffix_bound(&self) -> bool {
        self.lower_sanitized.contains("_table_suffix")
    }

    fn find_sanitized_from<F>(
        &self,
        lower_needle: &str,
        mut start_from: usize,
        raw_filter: F,
    ) -> Option<(usize, Span)>
    where
        F: Fn(usize, usize) -> bool,
    {
        while start_from < self.lower_sanitized.len() {
            let relative = self.lower_sanitized[start_from..].find(lower_needle)?;
            let start = start_from + relative;
            let end = start + lower_needle.len();
            if let Some((raw_start, raw_end)) = self.strip_map.map_sanitized_range(start, end) {
                if raw_filter(raw_start, raw_end) {
                    return Some((
                        start,
                        self.line_index.span(raw_start, raw_end.min(self.raw.len())),
                    ));
                }
            }
            start_from = end;
        }
        None
    }

    fn word_boundaries(&self, raw_start: usize, raw_end: usize) -> bool {
        if raw_start > raw_end
            || raw_end > self.raw.len()
            || !self.raw.is_char_boundary(raw_start)
            || !self.raw.is_char_boundary(raw_end)
        {
            return false;
        }
        let before = self.raw[..raw_start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_word_char(ch));
        let after = self.raw[raw_end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_word_char(ch));
        before && after
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_non_sargable_filter(expr: &Expr, op: Option<&BinaryOperator>) -> bool {
    match expr {
        Expr::Function(function) => {
            let name = function_name(function);
            if matches!(name.as_str(), "date_trunc" | "timestamp_trunc" | "date")
                && function
                    .args
                    .args()
                    .iter()
                    .any(expr_contains_partition_column_in_arg)
                && op.is_some_and(|op| {
                    matches!(
                        op,
                        BinaryOperator::Lt
                            | BinaryOperator::LtEq
                            | BinaryOperator::Gt
                            | BinaryOperator::GtEq
                    )
                })
            {
                return false;
            }
            is_sargability_breaking_function(function)
                && function
                    .args
                    .args()
                    .iter()
                    .any(expr_contains_partition_column_in_arg)
        }
        Expr::Cast { .. } => false,
        Expr::Nested(inner) => is_non_sargable_filter(inner, op),
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

fn table_name_has_wildcard(name: &ObjectName) -> bool {
    name.to_string().contains('*')
}

fn table_factor_catalog(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { name, .. } => object_name_catalog(name),
        TableFactor::Derived { .. } => None,
        _ => None,
    }
}

fn object_name_catalog(name: &ObjectName) -> Option<String> {
    if name.0.len() >= 3 {
        Some(object_name_part_ident(&name.0[0]))
    } else {
        None
    }
}

fn catalogs_differ(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

fn collect_table_aliases(select: &Select) -> Vec<String> {
    let mut aliases = Vec::new();
    for table in &select.from {
        collect_aliases_from_table_with_joins(table, &mut aliases);
    }
    aliases
}

fn collect_aliases_from_table_with_joins(table: &TableWithJoins, aliases: &mut Vec<String>) {
    push_table_factor_alias(&table.relation, aliases);
    for join in &table.joins {
        push_table_factor_alias(&join.relation, aliases);
    }
}

fn push_table_factor_alias(factor: &TableFactor, aliases: &mut Vec<String>) {
    match factor {
        TableFactor::Table { name, alias, .. } => {
            if let Some(alias) = alias {
                aliases.push(alias.name.value.to_ascii_lowercase());
            } else if let Some(table) = name.0.last() {
                aliases.push(object_name_part_ident(table));
            }
        }
        TableFactor::Derived {
            alias: Some(alias), ..
        } => {
            aliases.push(alias.name.value.to_ascii_lowercase());
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => collect_aliases_from_table_with_joins(table_with_joins, aliases),
        _ => {}
    }
}

fn outer_aliases_for_join(factor: &TableFactor) -> Vec<String> {
    let mut aliases = Vec::new();
    push_table_factor_alias(factor, &mut aliases);
    aliases
}

fn extract_leading_wildcard_likes(
    expr: &Expr,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::Like {
            pattern,
            negated: false,
            ..
        }
        | Expr::ILike {
            pattern,
            negated: false,
            ..
        } => {
            if pattern_starts_with_wildcard(pattern) {
                let snippet = expr.to_string();
                if let Some(span) = finder.find_next(&snippet) {
                    features.leading_wildcard_likes.push(ExpressionFeature {
                        span,
                        key: "leading wildcard like".into(),
                        text: snippet,
                    });
                }
            }
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And | BinaryOperator::Or,
            right,
        } => {
            extract_leading_wildcard_likes(left, finder, features);
            extract_leading_wildcard_likes(right, finder, features);
        }
        Expr::Nested(inner) => extract_leading_wildcard_likes(inner, finder, features),
        _ => {}
    }
}

fn pattern_starts_with_wildcard(expr: &Expr) -> bool {
    match expr {
        Expr::Value(ValueWithSpan {
            value: Value::SingleQuotedString(value) | Value::DoubleQuotedString(value),
            ..
        }) => value.starts_with('%') || value.starts_with('_'),
        Expr::Nested(inner) => pattern_starts_with_wildcard(inner),
        _ => false,
    }
}

fn extract_or_partition_predicates(
    expr: &Expr,
    finder: &mut SpanFinder<'_>,
    features: &mut SqlFeatures,
) {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } if expr_contains_partition_column(left) && expr_contains_partition_column(right) => {
            let snippet = expr.to_string();
            if let Some(span) = finder.find_next(&snippet) {
                features.or_partition_predicates.push(ExpressionFeature {
                    span,
                    key: "or partition predicate".into(),
                    text: snippet,
                });
            }
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And | BinaryOperator::Or,
            right,
        } => {
            extract_or_partition_predicates(left, finder, features);
            extract_or_partition_predicates(right, finder, features);
        }
        Expr::Nested(inner) => extract_or_partition_predicates(inner, finder, features),
        _ => {}
    }
}

fn predicate_is_pattern_matching(expr: &Expr) -> bool {
    match expr {
        Expr::Like { .. } | Expr::ILike { .. } | Expr::RLike { .. } | Expr::SimilarTo { .. } => {
            true
        }
        Expr::Function(function) => matches!(
            function_name(function).as_str(),
            "regexp_like" | "regexp" | "rlike"
        ),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => predicate_is_pattern_matching(left) || predicate_is_pattern_matching(right),
        Expr::Nested(inner) => predicate_is_pattern_matching(inner),
        _ => false,
    }
}

pub fn merge_shape_features(
    mut base: SqlFeatures,
    ast: SqlFeatures,
    parsed: bool,
    trust_empty_ast: bool,
) -> SqlFeatures {
    if !parsed {
        return base;
    }
    macro_rules! merge_field {
        ($field:ident) => {
            if !ast.$field.is_empty() {
                base.$field = ast.$field;
            }
        };
    }
    if trust_empty_ast {
        base.select_stars = ast.select_stars;
    } else {
        merge_field!(select_stars);
    }
    merge_field!(order_by_clauses);
    merge_field!(group_by_clauses);
    merge_field!(distincts);
    merge_field!(window_functions);
    merge_field!(ctes);
    merge_field!(cte_references);
    if parsed {
        // ponytail: regex non_sargable matches JOIN ON date_trunc; trust AST WHERE-only extraction
        base.non_sargable_predicates = ast.non_sargable_predicates;
    } else {
        merge_field!(non_sargable_predicates);
    }
    merge_field!(unions_without_all);
    merge_field!(count_distincts);
    merge_field!(wildcard_table_scans);
    merge_field!(correlated_subqueries);
    merge_field!(leading_wildcard_likes);
    merge_field!(or_partition_predicates);
    merge_field!(scalar_subqueries_in_select);
    merge_field!(row_explosions);
    merge_field!(not_in_subqueries);
    merge_field!(recursive_ctes);
    base.joins = if trust_empty_ast {
        if ast.joins.is_empty() {
            base.joins
        } else {
            ast.joins
        }
    } else if !ast.joins.is_empty() {
        merge_join_features(&base.joins, &ast.joins)
    } else {
        base.joins
    };
    base
}
