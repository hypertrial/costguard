#!/usr/bin/env python3
"""Generate recall corpus fixtures for behavioral rules SQLCOST001-022 and SQLCOST028-044."""

from __future__ import annotations

import argparse
import filecmp
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "tests" / "fixtures" / "corpus"

FIXTURES: dict[str, dict[str, str]] = {
    "recall_select_star_plain": {
        "models/marts/fct_orders.sql": "select * from {{ ref('stg_orders') }}\n",
    },
    "recall_select_star_qualified": {
        "models/marts/fct_users.sql": "select t.* from users t\n",
    },
    "recall_select_star_staging_ok": {
        "models/staging/stg_events.sql": "select id, event_date from raw.events\n",
    },
    "recall_repeated_regex_plain": {
        "models/staging/stg_logs.sql": (
            "select regexp_extract(msg, 'error'), regexp_replace(msg, 'x', 'y') from logs\n"
        ),
    },
    "recall_repeated_regex_jinja": {
        "models/staging/stg_logs_jinja.sql": (
            "select regexp_extract({{ adapter.quote('msg') }}, 'a'), "
            "regexp_extract({{ adapter.quote('msg') }}, 'b') from logs\n"
        ),
    },
    "recall_single_regex_ok": {
        "models/staging/stg_clean.sql": "select regexp_extract(msg, 'token') as token from logs\n",
    },
    "recall_incremental_no_key_inline": {
        "models/marts/fct_events.sql": (
            "{{ config(materialized='incremental') }}\nselect id from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_incremental_no_key_yaml": {
        "models/marts/fct_sessions.sql": "select session_id from {{ ref('stg_sessions') }}\n",
        "models/schema.yml": (
            "version: 2\n\nmodels:\n  - name: fct_sessions\n    config:\n"
            "      materialized: incremental\n"
        ),
    },
    "recall_unbounded_join_plain": {
        "models/marts/fct_join.sql": "select * from orders o join users u on o.amount > u.threshold\n",
    },
    "recall_unbounded_join_cross_like": {
        "models/marts/fct_cross_like.sql": "select * from a join b on a.id <> b.id\n",
    },
    "recall_equality_join_ok": {
        "models/marts/fct_safe_join.sql": "select * from a join b on a.id = b.id\n",
    },
    "recall_order_by_plain": {
        "models/marts/fct_ranked.sql": "select id, score from events order by score desc\n",
    },
    "recall_order_by_jinja": {
        "models/marts/fct_sorted.sql": (
            "select id from {{ ref('stg_events') }}\norder by id\n"
        ),
    },
    "recall_order_by_limited_ok": {
        "models/marts/fct_top.sql": "select id from events order by id limit 10\n",
    },
    "recall_distinct_plain": {
        "models/marts/fct_unique.sql": "select distinct user_id from events\n",
    },
    "recall_distinct_jinja": {
        "models/marts/fct_dedup.sql": "select distinct {{ adapter.quote('user_id') }} from events\n",
    },
    "recall_distinct_grouped_ok": {
        "models/marts/fct_agg.sql": "select user_id, count(*) from events group by 1\n",
    },
    "recall_normalization_repeated": {
        "models/marts/fct_norm.sql": "select lower(trim(email)), lower(trim(email)) from users\n",
    },
    "recall_normalization_jinja": {
        "models/marts/fct_norm_jinja.sql": (
            "select lower(trim(email)), lower(trim(email)) from {{ ref('stg_users') }}\n"
        ),
    },
    "recall_cross_file_expensive_c": {
        "models/marts/fct_x.sql": "select json_extract(payload, '$.token') from alpha\n",
        "models/marts/fct_y.sql": "select json_extract(payload, '$.token') from beta\n",
        "models/marts/fct_z.sql": "select json_extract(payload, '$.token') from gamma\n",
    },
    "recall_normalization_single_ok": {
        "models/marts/fct_email.sql": "select lower(trim(email)) as email from users\n",
    },
    "recall_python_apply": {
        "models/marts/score.py": "df.apply(lambda row: row['x'] * 2, axis=1)\n",
    },
    "recall_python_iterrows": {
        "models/marts/process.py": "for _, row in df.iterrows():\n    pass\n",
    },
    "recall_python_vectorized_ok": {
        "models/marts/vectorized.py": "result = df['amount'] * df['rate']\n",
    },
    "recall_source_mart_second": {
        "models/marts/fct_raw.sql": "select id from {{ source('raw', 'events') }}\n",
    },
    "recall_source_via_ref_ok": {
        "models/marts/fct_clean.sql": "select id from {{ ref('stg_events') }}\n",
    },
    "recall_window_no_partition": {
        "models/marts/fct_window.sql": "select row_number() over (order by id) as rn from events\n",
    },
    "recall_window_no_partition_jinja": {
        "models/marts/fct_rank.sql": (
            "select rank() over (order by {{ adapter.quote('score') }}) as rnk from events\n"
        ),
    },
    "recall_window_partitioned_ok": {
        "models/marts/fct_part_window.sql": (
            "select row_number() over (partition by user_id order by id) as rn from events\n"
        ),
    },
    "recall_repeated_cte_plain": {
        "models/marts/fct_cte.sql": (
            "with base as (select id from events)\n"
            "select * from base join base b on base.id = b.id join base c on base.id = c.id\n"
        ),
    },
    "recall_repeated_cte_jinja": {
        "models/marts/fct_cte_jinja.sql": (
            "with c as (select id from {{ ref('stg_events') }})\n"
            "select c.id from c join c d on c.id = d.id join c e on c.id = e.id\n"
        ),
    },
    "recall_single_cte_ok": {
        "models/marts/fct_single_cte.sql": "with c as (select id from events) select * from c\n",
    },
    "recall_cross_file_expensive_a": {
        "models/marts/fct_a.sql": "select json_extract(payload, '$.session_id') from events\n",
    },
    "recall_cross_file_expensive_b": {
        "models/marts/fct_a.sql": "select json_extract(payload, '$.session_id') from events\n",
        "models/marts/fct_b.sql": "select json_extract(payload, '$.session_id') from sessions\n",
        "models/marts/fct_c.sql": "select json_extract(payload, '$.session_id') from users\n",
    },
    "recall_single_file_json_ok": {
        "models/marts/fct_json.sql": "select json_extract(payload, '$.id') as id from events\n",
    },
    "recall_non_sargable_date": {
        "models/marts/fct_date.sql": (
            "select id from events where date(block_time) = current_date\n"
        ),
    },
    "recall_non_sargable_jinja": {
        "models/marts/fct_filter.sql": (
            "select id from events where date(block_time) = {{ var('run_date') }}\n"
        ),
    },
    "recall_sargable_range_ok": {
        "models/marts/fct_range.sql": (
            "select id from events where block_time >= current_date and block_time < current_date + interval '1' day\n"
        ),
    },
    "recall_union_plain": {
        "models/staging/stg_union.sql": (
            "select id from stripe_payments union select id from paypal_payments\n"
        ),
    },
    "recall_union_jinja": {
        "models/staging/stg_union_jinja.sql": (
            "select id from {{ ref('stg_a') }} union select id from {{ ref('stg_b') }}\n"
        ),
    },
    "recall_union_all_ok": {
        "models/staging/stg_union_all.sql": (
            "select id from a union all select id from b\n"
        ),
    },
    "recall_incremental_unbounded_source": {
        "models/marts/fct_source.sql": (
            "{{ config(materialized='incremental', unique_key='id') }}\n"
            "with raw as (select * from {{ source('events', 'raw_events') }}),\n"
            "filtered as (select * from raw {% if is_incremental() %} where event_date >= current_date - 3 {% endif %})\n"
            "select * from filtered\n"
        ),
    },
    "recall_incremental_unbounded_source_jinja": {
        "models/marts/fct_events_inc.sql": (
            "-- owner: {{ var('owner') }}\n"
            "{{ config(materialized='incremental', unique_key='id') }}\n"
            "with raw as (select * from {{ source('events', 'raw_events') }}),\n"
            "filtered as (select * from raw {% if is_incremental() %} where event_date >= current_date - 3 {% endif %})\n"
            "select * from filtered\n"
        ),
    },
    "recall_incremental_source_bounded_ok": {
        "models/marts/fct_bounded.sql": (
            "{{ config(materialized='incremental', unique_key='id') }}\n"
            "select id from {{ source('raw', 'events') }}\n"
            "{% if is_incremental() %}\nwhere event_date >= current_date - 3\n{% endif %}\n"
        ),
    },
    "recall_count_distinct_plain": {
        "models/marts/fct_users.sql": (
            "select campaign_id, count(distinct user_id) from events group by 1\n"
        ),
    },
    "recall_count_distinct_jinja": {
        "models/marts/fct_campaigns.sql": (
            "select count(distinct {{ adapter.quote('user_id') }}) from events\n"
        ),
    },
    "recall_count_distinct_ok": {
        "models/marts/fct_totals.sql": "select count(*) from events\n",
    },
    "recall_bq_wildcard_plain": {
        "models/marts/fct_events.sql": "select * from `project.dataset.events_*`\n",
    },
    "recall_bq_wildcard_jinja": {
        "models/marts/fct_sharded.sql": "select * from `{{ var('project') }}.dataset.table_*`\n",
    },
    "recall_bq_wildcard_bounded_ok": {
        "models/marts/fct_suffix.sql": (
            "select * from `project.dataset.events_*` where _TABLE_SUFFIX between '20240101' and '20240131'\n"
        ),
    },
    "recall_python_collect": {
        "models/marts/fetch.py": "rows = session.sql('select 1').collect()\n",
    },
    "recall_python_to_pandas": {
        "models/marts/export.py": "pdf = df.to_pandas()\n",
    },
    "recall_python_local_ok": {
        "models/marts/transform.py": "result = df.with_column('x', df['a'] + df['b'])\n",
    },
    "recall_repeated_json_jinja": {
        "models/staging/stg_json_jinja.sql": (
            "select json_extract_scalar(payload, '$.id'), "
            "json_extract_scalar(payload, '$.id') from events\n"
        ),
    },
    "recall_incremental_missing_id_predicate": {
        "models/marts/fct_inc.sql": (
            "{{ config(materialized='incremental', unique_key='id') }}\n"
            "select id from {{ source('raw', 'events') }}\n"
            "{% if is_incremental() %}\nwhere id > 1\n{% endif %}\n"
        ),
        "models/schema.yml": (
            "version: 2\n\nsources:\n  - name: raw\n    tables:\n      - name: events\n"
        ),
    },
    "recall_join_key_trim": {
        "models/marts/fct_trim_join.sql": (
            "select a.id, b.id\n"
            "from {{ ref('stg_a') }} a\n"
            "join {{ ref('stg_b') }} b on lower(a.email) = b.user_id\n"
        ),
        "models/staging/stg_a.sql": "select 1 as id, 'a@x.com' as email\n",
        "models/staging/stg_b.sql": "select 1 as id, 'abc' as user_id\n",
    },
    "recall_missing_partition_plain": {
        "models/marts/fct_orders.sql": (
            "{{ config(materialized='incremental', unique_key='order_id') }}\n"
            "select order_id from {{ ref('stg_orders') }}\n"
        ),
    },
    "recall_missing_partition_yaml": {
        "models/marts/fct_sales.sql": "select sale_id from {{ ref('stg_sales') }}\n",
        "models/schema.yml": (
            "version: 2\n\nmodels:\n  - name: fct_sales\n    config:\n"
            "      materialized: table\n"
        ),
    },
    "recall_partition_cluster_ok": {
        "models/marts/fct_part.sql": (
            "{{ config(materialized='incremental', unique_key='id', partition_by={'field': 'event_date'}) }}\n"
            "select id, event_date from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_full_refresh_incremental": {
        "models/marts/fct_refresh.sql": (
            "{{ config(materialized='incremental', unique_key='id', full_refresh=true) }}\n"
            "select id from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_sync_all_columns": {
        "models/marts/fct_sync.sql": (
            "{{ config(materialized='incremental', unique_key='id', on_schema_change='sync_all_columns') }}\n"
            "select id from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_incremental_config_ok": {
        "models/marts/fct_ok.sql": (
            "{{ config(materialized='incremental', unique_key='id') }}\n"
            "select id from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_correlated_subquery_plain": {
        "models/marts/fct_corr.sql": (
            "select id from orders o\n"
            "where exists (select 1 from line_items li where li.order_id = o.id)\n"
        ),
    },
    "recall_correlated_subquery_jinja": {
        "models/marts/fct_corr_jinja.sql": (
            "select id from orders o\n"
            "where exists (select 1 from line_items li where li.order_id = o.id)\n"
        ),
    },
    "recall_uncorrelated_subquery_ok": {
        "models/marts/fct_sub_ok.sql": "select id from orders where status in (select status from allowed_statuses)\n",
    },
    "recall_leading_wildcard_like_plain": {
        "models/marts/fct_like.sql": "select id from users where email like '%@example.com'\n",
    },
    "recall_leading_wildcard_like_jinja": {
        "models/marts/fct_like_jinja.sql": (
            "-- domain filter: {{ var('domain') }}\n"
            "select id from users where email like '%@example.com'\n"
        ),
    },
    "recall_trailing_wildcard_ok": {
        "models/marts/fct_prefix.sql": "select id from users where email like 'user%'\n",
    },
    "recall_or_partition_plain": {
        "models/marts/fct_or_part.sql": (
            "select id from events where event_date = current_date or block_time >= current_date\n"
        ),
    },
    "recall_or_partition_jinja": {
        "models/marts/fct_or_jinja.sql": (
            "select id from events where event_date = {{ var('run_date') }} or block_time >= current_date\n"
        ),
    },
    "recall_and_partition_ok": {
        "models/marts/fct_and_part.sql": (
            "select id from events where event_date >= current_date and block_time < current_date + interval '1' day\n"
        ),
    },
    "recall_pattern_join_plain": {
        "models/marts/fct_pat_join.sql": "select * from a join b on a.name like b.token\n",
    },
    "recall_pattern_join_jinja": {
        "models/marts/fct_pat_join_jinja.sql": (
            "select * from {{ ref('stg_a') }} a join {{ ref('stg_b') }} b on a.name rlike b.token\n"
        ),
    },
    "recall_pattern_join_equality_ok": {
        "models/marts/fct_eq_join.sql": "select * from a join b on a.id = b.id\n",
    },
    "recall_scalar_subquery_plain": {
        "models/marts/fct_scalar.sql": (
            "select u.id, (select max(amount) from payments p where p.user_id = u.id) as max_pay\n"
            "from users u\n"
        ),
    },
    "recall_scalar_subquery_jinja": {
        "models/marts/fct_scalar_jinja.sql": (
            "select u.id, (select count(*) from {{ ref('stg_orders') }} o where o.user_id = u.id) as n\n"
            "from users u\n"
        ),
    },
    "recall_scalar_subquery_join_ok": {
        "models/marts/fct_scalar_ok.sql": (
            "select u.id, p.max_amount from users u join payment_totals p on u.id = p.user_id\n"
        ),
    },
    "recall_cross_catalog_plain": {
        "models/marts/fct_xcat.sql": (
            "select * from hive.default.orders o join iceberg.analytics.users u on o.id = u.id\n"
        ),
    },
    "recall_cross_catalog_jinja": {
        "models/marts/fct_xcat_jinja.sql": (
            "select * from hive.default.events e\n"
            "join iceberg.analytics.users u on e.user_id = u.id\n"
        ),
    },
    "recall_same_catalog_ok": {
        "models/marts/fct_same_cat.sql": (
            "select * from hive.default.orders o join hive.default.users u on o.id = u.id\n"
        ),
    },
    "recall_row_explosion_plain": {
        "models/marts/fct_tags.sql": (
            "select distinct user_id from orders cross join unnest(tag_list) as tag\n"
        ),
    },
    "recall_row_explosion_jinja": {
        "models/marts/fct_tags_jinja.sql": (
            "select distinct user_id from {{ ref('stg_orders') }} cross join unnest(tag_list) as tag\n"
        ),
    },
    "recall_row_explosion_ok": {
        "models/marts/fct_tags_ok.sql": (
            "select user_id, tag from orders cross join unnest(tag_list) as tag\n"
        ),
    },
    "recall_not_in_subquery_plain": {
        "models/marts/fct_orders.sql": (
            "select id from orders where id not in (select order_id from returns)\n"
        ),
    },
    "recall_not_in_subquery_jinja": {
        "models/marts/fct_orders_jinja.sql": (
            "select id from orders where id not in (select order_id from {{ ref('stg_returns') }})\n"
        ),
    },
    "recall_in_subquery_ok": {
        "models/marts/fct_orders_ok.sql": (
            "select id from orders where id in (select order_id from returns)\n"
        ),
    },
    "recall_fan_out_join_plain": {
        "models/marts/dim_users.sql": (
            "{{ config(unique_key='user_id') }} select user_id, email from users\n"
        ),
        "models/marts/fct_orders.sql": (
            "select * from orders join dim_users on orders.email = dim_users.email\n"
        ),
    },
    "recall_fan_out_join_jinja": {
        "models/marts/dim_users.sql": (
            "{{ config(unique_key='user_id') }} select user_id, email from users\n"
        ),
        "models/marts/fct_orders.sql": (
            "select * from orders join {{ ref('dim_users') }} u on orders.email = u.email\n"
        ),
    },
    "recall_fan_out_join_ok": {
        "models/marts/dim_users.sql": (
            "{{ config(unique_key='user_id') }} select user_id, email from users\n"
        ),
        "models/marts/fct_orders.sql": (
            "select * from orders join dim_users on orders.user_id = dim_users.user_id\n"
        ),
    },
    "recall_heavy_view_plain": {
        "models/intermediate/int_shared.sql": (
            "{{ config(materialized='view') }} select id, event_date from base\n"
        ),
        "models/marts/fct_a.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_b.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_c.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_d.sql": "select id from {{ ref('int_shared') }}\n",
    },
    "recall_heavy_view_jinja": {
        "models/intermediate/int_shared.sql": (
            "{{ config(materialized='view') }} select id, event_date from {{ ref('stg_base') }}\n"
        ),
        "models/marts/fct_a.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_b.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_c.sql": "select id from {{ ref('int_shared') }}\n",
        "models/marts/fct_d.sql": "select id from {{ ref('int_shared') }}\n",
    },
    "recall_heavy_view_ok": {
        "models/intermediate/int_shared.sql": (
            "{{ config(materialized='table') }} select id, event_date from base\n"
        ),
        "models/marts/fct_a.sql": "select id from {{ ref('int_shared') }}\n",
    },
    "recall_table_should_incremental_plain": {
        "models/marts/fct_events.sql": (
            "{{ config(materialized='table') }} select id, event_date from events\n"
        ),
    },
    "recall_table_should_incremental_jinja": {
        "models/marts/fct_events.sql": (
            "{{ config(materialized='table') }} select id, event_date from {{ ref('stg_events') }}\n"
        ),
    },
    "recall_table_incremental_ok": {
        "models/marts/fct_events.sql": (
            "{{ config(materialized='incremental', unique_key='id') }} select id, event_date from events\n"
        ),
    },
    "recall_unbounded_window_plain": {
        "models/marts/fct_window.sql": (
            "select sum(amount) over (partition by user_id order by ts "
            "rows between unbounded preceding and unbounded following) from events\n"
        ),
    },
    "recall_unbounded_window_jinja": {
        "models/marts/fct_window_jinja.sql": (
            "select sum(amount) over (partition by user_id order by {{ adapter.quote('ts') }} "
            "rows between unbounded preceding and unbounded following) from events\n"
        ),
    },
    "recall_bounded_window_ok": {
        "models/marts/fct_window_ok.sql": (
            "select sum(amount) over (partition by user_id order by ts "
            "rows between 29 preceding and current row) from events\n"
        ),
    },
    "recall_bq_missing_partition_plain": {
        "models/marts/fct_events.sql": "select * from {{ ref('stg_events') }}\n",
    },
    "recall_bq_missing_partition_jinja": {
        "models/marts/fct_events_jinja.sql": (
            "select * from {{ ref('stg_events') }} where status = 'active'\n"
        ),
    },
    "recall_bq_partition_filter_ok": {
        "models/marts/fct_events_ok.sql": (
            "select * from {{ ref('stg_events') }} "
            "where _PARTITIONDATE >= date_sub(current_date(), interval 3 day)\n"
        ),
    },
    "recall_merge_no_pruning_plain": {
        "models/marts/fct_events.sql": (
            "{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id') }}\n"
            "select id, updated_at from events\n"
            "{% if is_incremental() %}\nwhere updated_at >= current_date - 3\n{% endif %}\n"
        ),
    },
    "recall_merge_no_pruning_jinja": {
        "models/marts/fct_events_jinja.sql": (
            "{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id') }}\n"
            "select id, updated_at from {{ ref('stg_events') }}\n"
            "{% if is_incremental() %}\nwhere updated_at >= current_date - 3\n{% endif %}\n"
        ),
    },
    "recall_merge_pruning_ok": {
        "models/marts/fct_events_ok.sql": (
            "{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id', "
            "incremental_predicates=[\"DBT_INTERNAL_DEST.updated_at >= current_date - 3\"]) }}\n"
            "select id, updated_at from events\n"
        ),
    },
    "recall_recursive_cte_plain": {
        "models/marts/fct_graph.sql": (
            "with recursive graph as (select 1 as n union all select n + 1 from graph where n < 5) "
            "select * from graph\n"
        ),
    },
    "recall_recursive_cte_jinja": {
        "models/marts/fct_graph_jinja.sql": (
            "with recursive graph as (select 1 as n union all select n + 1 from graph where n < 5) "
            "select * from graph\n"
        ),
    },
    "recall_non_recursive_cte_ok": {
        "models/marts/fct_graph_ok.sql": "with graph as (select 1 as n) select * from graph\n",
    },
}


def normalized_content(content: str | tuple[str, ...]) -> str:
    if isinstance(content, tuple):
        return "".join(content)
    return content


def write_fixtures(corpus_dir: Path) -> None:
    for name, files in FIXTURES.items():
        case_dir = corpus_dir / name
        case_dir.mkdir(parents=True, exist_ok=True)
        for relative, content in files.items():
            path = case_dir / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(normalized_content(content), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit 1 when committed recall fixtures differ from generator output",
    )
    args = parser.parse_args()

    if args.check:
        with tempfile.TemporaryDirectory(prefix="costguard-recall-check-") as tmp:
            generated = Path(tmp)
            write_fixtures(generated)
            errors: list[str] = []
            for name in FIXTURES:
                expected_dir = generated / name
                actual_dir = CORPUS / name
                if not actual_dir.is_dir():
                    errors.append(f"missing fixture directory: {name}")
                    continue
                comparison = filecmp.dircmp(expected_dir, actual_dir)
                errors.extend(
                    f"missing fixture file: {name}/{path}"
                    for path in comparison.left_only
                )
                errors.extend(
                    f"extra fixture file: {name}/{path}"
                    for path in comparison.right_only
                )
                for path in comparison.common_files:
                    if not filecmp.cmp(expected_dir / path, actual_dir / path, shallow=False):
                        errors.append(f"stale fixture file: {name}/{path}")
            if errors:
                for error in errors:
                    print(error, file=sys.stderr)
                raise SystemExit(1)
        print(f"recall corpus fixtures up to date ({len(FIXTURES)} cases)")
        return 0

    write_fixtures(CORPUS)
    print(f"wrote {len(FIXTURES)} recall corpus fixtures under {CORPUS}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
