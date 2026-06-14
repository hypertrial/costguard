use costguard_dbt::extract_sql_features;
use costguard_diagnostics::{LineIndex, SourceProvenance};
use costguard_platform::Platform;
use costguard_sql::*;
use sqlparser::parser::Parser;
use std::path::PathBuf;

fn analyze_test_sql(
    path: PathBuf,
    text: &str,
    platform: Platform,
    index: &LineIndex,
    compiled_code: Option<&str>,
    is_dbt_sql_model: bool,
) -> SqlDocument {
    let dbt = extract_sql_features(text);
    analyze_sql(
        path,
        text,
        platform,
        index,
        compiled_code,
        is_dbt_sql_model,
        dbt,
    )
}

#[test]
fn extracts_select_star_and_window() {
    let text = "select *, row_number() over () as rn from a cross join b";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert_eq!(doc.features.select_stars.len(), 1);
    assert_eq!(doc.features.window_functions.len(), 1);
    assert_eq!(doc.features.joins.len(), 1);
}

#[test]
fn ast_join_span_tracks_second_repeated_join() {
    let text = "select a.id from a join b on a.id = b.id\njoin c on a.id = c.id";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(doc.features.joins.iter().any(|join| join.span.line == 1));
    assert!(doc
        .features
        .joins
        .iter()
        .any(|join| join.span.line == 2 && join.span.column == 1));
}

#[test]
fn ast_window_span_tracks_unpartitioned_window() {
    let text = "select sum(x) over (partition by id) as sx,\nrow_number() over () as rn from t";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    let unpartitioned = doc
        .features
        .window_functions
        .iter()
        .find(|window| !window.has_partition_by)
        .expect("unpartitioned window");
    assert_eq!(unpartitioned.span.line, 2);
    assert_eq!(unpartitioned.span.column, 1);
}

#[test]
fn ast_repeated_select_star_spans_are_distinct() {
    let text = "select * from (select * from t) nested";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert_eq!(doc.features.select_stars.len(), 2);
    assert!(
        doc.features.select_stars[0].span.byte_start < doc.features.select_stars[1].span.byte_start
    );
}

#[test]
fn ast_cte_references_do_not_collapse_to_definition() {
    let text = "with c as (select 1), d as (select * from c) select * from c join c c2 on true";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    let c_definition = doc
        .features
        .ctes
        .iter()
        .find(|cte| cte.name == "c")
        .expect("cte c");
    let c_references = doc
        .features
        .cte_references
        .iter()
        .filter(|reference| reference.key == "c")
        .collect::<Vec<_>>();
    assert!(c_references.len() >= 3, "{c_references:?}");
    assert!(c_references
        .iter()
        .all(|reference| reference.span.byte_start > c_definition.span.byte_start));
    assert!(c_references
        .windows(2)
        .all(|window| window[0].span.byte_start < window[1].span.byte_start));
}

#[test]
fn normalize_strips_comments_and_try_cast() {
    let sql = "-- header\nSELECT TRY_CAST(x AS bigint) FROM t";
    let normalized = normalize_for_parse(sql, Platform::Trino);
    assert!(!normalized.contains("-- header"));
    assert!(normalized.contains("CAST("));
    assert!(!normalized.to_ascii_lowercase().contains("try_cast"));
}

#[test]
fn compiled_fallback_uses_raw_when_compiled_unparseable() {
    let raw = "select id from stg";
    let compiled = "SELECT id FROM stg WHERE TRY_INVALID_SYNTAX(!!!";
    let index = LineIndex::new(raw);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        raw,
        Platform::Trino,
        &index,
        Some(compiled),
        true,
    );
    assert!(!doc.parsed_compiled);
    assert!(doc.parsed_raw);
    assert!(doc.parsed);
    assert_eq!(doc.parse_input, ParseInput::CompiledWithRawFallback);
}

#[test]
fn trino_try_cast_normalizes_to_parseable_sql() {
    let sql = "SELECT TRY_CAST(block_time AS timestamp) FROM dex.trades";
    let normalized = normalize_for_parse(sql, Platform::Trino);
    let dialect = Platform::Trino.sqlparser_dialect();
    assert!(Parser::parse_sql(dialect.as_ref(), &normalized).is_ok());
}

#[test]
fn parses_largest_statement_from_multi_statement_compiled() {
    let sql = "SET session x = '1'; SELECT id FROM t WHERE block_time >= date_sub(current_date, interval '3' day);";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_underscore_identifiers_in_compiled_sql() {
    let sql = "SELECT current_timestamp AS _updated_at, s._dstChainId FROM s";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_json_query_omit_quotes() {
    let sql = "SELECT try_cast(json_query(x, 'lax $.a' omit quotes) as boolean) FROM t";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_cast_array_type_syntax() {
    let sql = "SELECT * FROM t CROSS JOIN UNNEST(CAST(JSON_EXTRACT(j, '$.s') AS ARRAY(VARCHAR))) AS u(strategy)";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_row_map_angle_type_syntax() {
    let sql = "SELECT CAST(NULL AS ROW(output map<varchar, JSON>)) FROM t";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_asof_left_join() {
    let sql = "SELECT hs.hour FROM hour_spine hs ASOF LEFT JOIN sparse_ohlcv s ON s.market_id = hs.market_id";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn plain_comma_join_is_flagged() {
    let text = "select a.id, b.id from stg_a a, stg_b b";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(
        doc.features
            .joins
            .iter()
            .any(|join| join.kind == JoinKind::Comma),
        "expected comma join; parsed_raw={} joins={:?}",
        doc.parsed_raw,
        doc.features.joins
    );
}

#[test]
fn compiled_ast_fallback_uses_compiled_sql_when_raw_unparseable() {
    let raw = "{% set cols = ['id'] %} select {{ cols | join(', ') }} from t where date_trunc('day', block_time) >= current_date";
    let compiled = "select id from t where date_trunc('day', block_time) >= current_date";
    let index = LineIndex::new(raw);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/jinja.sql"),
        raw,
        Platform::Trino,
        &index,
        Some(compiled),
        true,
    );
    assert!(doc.parsed_compiled);
    assert!(!doc.parsed_raw);
    assert!(doc.feature_extraction_used_ast);
    assert!(doc.features.non_sargable_predicates.is_empty());
}

#[test]
fn compiled_ast_features_are_marked_unmapped() {
    let raw = "{% set cols = ['id'] %} select {{ cols | join(', ') }} from a";
    let compiled = "select * from a cross join b";
    let index = LineIndex::new(raw);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/jinja.sql"),
        raw,
        Platform::Generic,
        &index,
        Some(compiled),
        true,
    );
    assert!(doc.feature_extraction_used_ast);
    let join = doc
        .features
        .joins
        .iter()
        .find(|join| join.kind == JoinKind::Cross)
        .expect("compiled cross join");
    assert_eq!(
        join.span.source_provenance,
        Some(SourceProvenance::CompiledUnmapped)
    );
}

#[test]
fn cte_inner_from_is_not_comma_join() {
    let text = r#"
with model_count as (
select count(*) as count_a from {{ model }}
)
select * from unit_test where diff_count > 0
"#;
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("macros/equal_rowcount.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(
        !doc.features
            .joins
            .iter()
            .any(|join| join.kind == JoinKind::Comma),
        "expected no comma join; joins={:?}",
        doc.features.joins
    );
}

#[test]
fn dbt_comma_join_with_refs_is_flagged() {
    let text = "select a.id, b.id\nfrom {{ ref('stg_a') }} a, {{ ref('stg_b') }} b\n";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(
        doc.features
            .joins
            .iter()
            .any(|join| join.kind == JoinKind::Comma),
        "expected comma join; parsed_raw={} joins={:?}",
        doc.parsed_raw,
        doc.features.joins
    );
}

#[test]
fn cross_join_unnest_is_not_flagged() {
    let text = "SELECT t.x FROM arr CROSS JOIN UNNEST(arr) AS t(x)";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma)));
}

#[test]
fn cross_join_date_spine_is_not_flagged() {
    let text = "select t.hash from {{ source('ethereum','transactions') }} t cross join check_date cd on true";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Cross));
}

#[test]
fn minute_date_trunc_join_is_not_function_wrapped_key() {
    let text = "select p.price from prices p join logs le on p.minute = date_trunc('minute', le.block_time)";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.function_on_join_key));
}

#[test]
fn compiled_time_bucket_join_is_not_function_wrapped_key() {
    let raw = "select * from {{ ref('events') }}";
    let compiled = "select * from dexs left join prices p on p.minute = date_trunc('minute', dexs.block_time) and p.contract_address = dexs.token_address";
    let index = LineIndex::new(raw);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        raw,
        Platform::Trino,
        &index,
        Some(compiled),
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.function_on_join_key));
}

#[test]
fn compiled_range_function_is_not_treated_as_join_key() {
    let raw = "select * from {{ ref('events') }}";
    let compiled = "select * from markets m join existing e on m.token_id = e.token_id and e.day >= cast(try_cast(m.start_time as timestamp) as date)";
    let index = LineIndex::new(raw);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        raw,
        Platform::Trino,
        &index,
        Some(compiled),
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.function_on_join_key));
}

#[test]
fn parsed_range_function_is_not_treated_as_join_key() {
    let text = "select * from markets m join existing e on m.token_id = e.token_id and e.day >= cast(try_cast(m.start_time as timestamp) as date)";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert_eq!(doc.features.joins.len(), 1);
    assert!(!doc.features.joins[0].function_on_join_key);
}

#[test]
fn explicit_join_with_group_by_commas_are_not_comma_joins() {
    let text = "select s.id from basic_yak_swaps s inner join logs l on l.tx_hash = s.tx_hash group by s.id, s.block_number";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("macros/yield_yak.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        false,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Comma));
}

#[test]
fn group_by_commas_are_not_comma_joins() {
    let text =
        "select strategy, sum(shares) as shares, date from daily_share group by strategy, date";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Comma));
}

#[test]
fn jinja_comment_from_is_not_comma_join() {
    let text = "{# Read from addresses_daily #}\nselect d.address from addresses_daily as d\ngroup by d.address";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("macros/addresses_stg_transfers_agg.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        false,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Comma));
}

#[test]
fn coalesce_null_safe_join_is_not_function_wrapped_key() {
    let text = "select 1 from transfers tr full outer join executed_txs et on tr.address = et.address left join ffb on coalesce(tr.address, et.address) = ffb.address";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/staging/stg_info.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.function_on_join_key));
}

#[test]
fn cross_join_date_ranges_is_not_flagged() {
    let text = "select t.hash from transactions t cross join date_ranges dr where t.block_time between dr.start_time and dr.end_time";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Cross));
}

#[test]
fn inner_subquery_comma_is_not_comma_join() {
    let text = "select * from (select a.id, b.id from inner_a a, inner_b b) derived";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("models/marts/fct.sql"),
        text,
        Platform::Trino,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| join.kind == JoinKind::Comma));
}

#[test]
fn string_literal_cross_join_is_not_flagged_when_sql_parses() {
    let text = "select 'cross join in string' as note from t";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(!doc
        .features
        .joins
        .iter()
        .any(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma)));
}

#[test]
fn trino_parses_cross_join_unnest() {
    let sql = "SELECT time.time AS day FROM time_seq CROSS JOIN unnest(time) AS time(time)";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_wrapped_quoted_table() {
    let sql = r#"SELECT id FROM ("delta_prod"."across_v2_arbitrum"."arbitrum_spokepool_evt_fundsdeposited") d"#;
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn trino_parses_join_alias_without_as() {
    let sql = "SELECT b.epoch FROM base b LEFT JOIN base start ON start.epoch = b.epoch";
    assert!(try_parse_compiled_sql(sql, Platform::Trino));
}

#[test]
fn extracts_not_in_subquery_and_recursive_cte() {
    let text = "with recursive graph as (select 1 as n) select id from t where id not in (select id from excluded)";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(!doc.features.not_in_subqueries.is_empty());
    assert!(!doc.features.recursive_ctes.is_empty());
}

#[test]
fn extracts_row_explosion_and_unbounded_window_frame() {
    let text = "select distinct user_id, sum(amount) over (partition by user_id order by ts rows between unbounded preceding and unbounded following) from orders cross join unnest(tag_list) as tag";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    assert!(!doc.features.row_explosions.is_empty());
    assert!(doc
        .features
        .window_functions
        .iter()
        .any(|window| window.unbounded_frame));
}

#[test]
fn extracts_join_right_relation_and_equality_keys() {
    let text = "select * from orders join dim_users on orders.user_id = dim_users.user_id";
    let index = LineIndex::new(text);
    let doc = analyze_test_sql(
        PathBuf::from("x.sql"),
        text,
        Platform::Generic,
        &index,
        None,
        true,
    );
    let join = doc.features.joins.first().expect("join");
    assert_eq!(join.right_relation.as_deref(), Some("dim_users"));
    assert!(join.equality_keys.iter().any(|key| key == "user_id"));
}
