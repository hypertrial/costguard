use sqlparser::ast::{BinaryOperator, Expr, Function, FunctionArg, FunctionArguments};

pub(crate) fn is_symmetric_wrapped_join_equality(left: &Expr, right: &Expr) -> bool {
    is_symmetric_normalization_eq(left, right) || is_symmetric_hash_eq(left, right)
}

pub(crate) fn join_predicate_has_function_on_key(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            if is_symmetric_normalization_eq(left, right)
                || is_time_bucket_join_eq(left, right)
                || is_coalesce_null_safe_join_eq(left, right)
                || is_coalesce_join_key(left)
                || is_coalesce_join_key(right)
                || is_normalization_of_same_column(left, right)
                || is_normalization_of_same_column(right, left)
                || is_symmetric_hash_eq(left, right)
            {
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

fn is_coalesce_null_safe_join_eq(left: &Expr, right: &Expr) -> bool {
    coalesce_null_safe_side(left, right) || coalesce_null_safe_side(right, left)
}

fn is_normalization_of_same_column(normalized: &Expr, bare: &Expr) -> bool {
    let Expr::Function(function) = normalized else {
        return false;
    };
    if !is_join_key_normalization_function(function) {
        return false;
    }
    let Some(inner) = function_first_arg_expr(function) else {
        return false;
    };
    column_names_equivalent(inner, bare)
}

fn function_first_arg_expr(function: &Function) -> Option<&Expr> {
    use sqlparser::ast::{FunctionArg, FunctionArgExpr};
    function.args.args().iter().find_map(|arg| match arg {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
        FunctionArg::Named {
            arg: FunctionArgExpr::Expr(expr),
            ..
        } => Some(expr),
        _ => None,
    })
}

fn is_symmetric_hash_eq(left: &Expr, right: &Expr) -> bool {
    match (left, right) {
        (Expr::Function(left_fn), Expr::Function(right_fn)) => {
            let left_name = function_name(left_fn);
            let right_name = function_name(right_fn);
            left_name == right_name
                && matches!(
                    left_name.as_str(),
                    "keccak256" | "keccak" | "sha256" | "sha2" | "md5" | "hash"
                )
        }
        _ => false,
    }
}

fn column_names_equivalent(left: &Expr, right: &Expr) -> bool {
    match (expr_column_name(left), expr_column_name(right)) {
        (Some(left_name), Some(right_name)) => left_name == right_name,
        _ => false,
    }
}

fn coalesce_null_safe_side(coalesce_side: &Expr, other: &Expr) -> bool {
    let Expr::Function(function) = coalesce_side else {
        return false;
    };
    if function_name(function) != "coalesce" {
        return false;
    }
    let Some(key) = expr_column_name(other) else {
        return false;
    };
    let args = function_arg_exprs(function);
    args.len() >= 2
        && args
            .iter()
            .all(|arg| expr_column_name(arg).as_deref() == Some(key.as_str()))
}

fn function_arg_exprs(function: &Function) -> Vec<&Expr> {
    use sqlparser::ast::{FunctionArg, FunctionArgExpr, FunctionArguments};
    let FunctionArguments::List(list) = &function.args else {
        return Vec::new();
    };
    list.args
        .iter()
        .filter_map(|arg| match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
            FunctionArg::Named {
                arg: FunctionArgExpr::Expr(expr),
                ..
            } => Some(expr),
            _ => None,
        })
        .collect()
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

pub(crate) fn expr_column_name(expr: &Expr) -> Option<String> {
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
            | "hr"
            | "day"
            | "date"
            | "week"
            | "month"
            | "block_time"
            | "evt_block_time"
            | "block_date"
            | "block_day"
            | "block_month"
            | "date_month"
            | "evt_block_date"
            | "timestamp"
            | "ts"
            | "period"
            | "time"
    )
}

fn is_coalesce_join_key(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(function) if function_name(function) == "coalesce")
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

pub(crate) fn function_name(function: &Function) -> String {
    function
        .name
        .0
        .last()
        .map(|ident| ident.value.to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) trait FunctionArgListExt {
    fn args(&self) -> &[FunctionArg];
}

impl FunctionArgListExt for FunctionArguments {
    fn args(&self) -> &[FunctionArg] {
        match self {
            FunctionArguments::List(list) => &list.args,
            _ => &[],
        }
    }
}
