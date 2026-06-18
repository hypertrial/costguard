use crate::{ProjectIndexes, RuleContext, RuleOverrides, RuleRegistry};
use costguard_dbt::extract_sql_features;
use costguard_diagnostics::{apply_suppressions, Confidence, LineIndex};
use costguard_scanner::{FileKind, ProjectFile};
use costguard_sql::Platform;
use costguard_sql::{analyze_sql, SqlDocument};
use std::path::PathBuf;

fn sql_file(path: &str, text: &str) -> ProjectFile {
    ProjectFile {
        path: PathBuf::from(path),
        root_relative_path: PathBuf::from(path),
        kind: FileKind::DbtSqlModel,
        text: text.into(),
        line_index: LineIndex::new(text),
    }
}

fn python_file(path: &str, text: &str) -> ProjectFile {
    ProjectFile {
        path: PathBuf::from(path),
        root_relative_path: PathBuf::from(path),
        kind: FileKind::Python,
        text: text.into(),
        line_index: LineIndex::new(text),
    }
}

fn run_diagnostics_for_file(
    file: &ProjectFile,
    docs: &[SqlDocument],
) -> Vec<costguard_diagnostics::Diagnostic> {
    let registry = RuleRegistry::default_rules();
    let indexes = ProjectIndexes::from_sql_documents(docs);
    let sql = docs.iter().find(|doc| doc.path == file.path);
    let diagnostics = registry.run(&RuleContext {
        warehouse: Platform::Generic,
        file,
        sql,
        dbt_model: None,
        all_sql: docs,
        project_indexes: &indexes,
        overrides: &RuleOverrides::default(),
    });
    apply_suppressions(&file.text, diagnostics)
}

fn run_for_file(file: &ProjectFile, docs: &[SqlDocument]) -> Vec<String> {
    run_diagnostics_for_file(file, docs)
        .into_iter()
        .map(|diagnostic| diagnostic.rule_id)
        .collect()
}

fn analyze(file: &ProjectFile) -> SqlDocument {
    let dbt = extract_sql_features(&file.text);
    analyze_sql(
        file.path.clone(),
        &file.text,
        Platform::Generic,
        &file.line_index,
        None,
        true,
        dbt,
    )
}

#[test]
fn sql_rules_have_positive_coverage() {
    let cases = [
        ("SQLCOST001", "models/marts/fct.sql", "select * from x"),
        (
            "SQLCOST002",
            "models/staging/stg.sql",
            "select json_extract(payload, '$.a'), json_extract(payload, '$.a') from raw.events",
        ),
        (
            "SQLCOST003",
            "models/staging/stg.sql",
            "select regexp_extract(a, 'x'), regexp_replace(a, 'x', 'y') from t",
        ),
        (
            "SQLCOST004",
            "models/marts/fct.sql",
            "{{ config(materialized='incremental') }} select id from t",
        ),
        (
            "SQLCOST005",
            "models/marts/fct.sql",
            "{{ config(materialized='incremental', unique_key='id') }} select id from t {% if is_incremental() %} where id > 1 {% endif %}",
        ),
        ("SQLCOST006", "models/marts/fct.sql", "select * from a join b on a.id > b.id"),
        ("SQLCOST007", "models/marts/fct.sql", "select id from t order by id"),
        ("SQLCOST008", "models/marts/fct.sql", "select distinct id from t"),
        (
            "SQLCOST009",
            "models/marts/fct.sql",
            "select lower(trim(email)), lower(trim(email)) from t",
        ),
        ("SQLCOST011", "models/marts/fct.sql", "select id from {{ source('raw', 'events') }}"),
        ("SQLCOST012", "models/marts/fct.sql", "select * from a cross join b"),
        ("SQLCOST013", "models/marts/fct.sql", "select row_number() over () from t"),
        (
            "SQLCOST014",
            "models/marts/fct.sql",
            "with c as (select * from t) select * from c join c b on c.id = b.id join c c2 on c.id = c2.id",
        ),
        (
            "SQLCOST016",
            "models/marts/fct.sql",
            "select id from t where date(block_time) = current_date",
        ),
        (
            "SQLCOST017",
            "models/marts/fct.sql",
            "select * from events e join users u on lower(e.email) = u.user_id",
        ),
        (
            "SQLCOST018",
            "models/staging/stg.sql",
            "select id from stripe_payments union select id from paypal_payments",
        ),
        (
            "SQLCOST019",
            "models/marts/fct.sql",
            "{{ config(materialized='incremental', unique_key='id') }}
with raw as (
  select * from {{ source('events', 'raw_events') }}
),
filtered as (
  select * from raw
  {% if is_incremental() %}
  where event_date >= current_date - 3
  {% endif %}
)
select * from filtered",
        ),
        (
            "SQLCOST020",
            "models/marts/fct.sql",
            "select campaign_id, count(distinct user_id) from events group by 1",
        ),
        (
            "SQLCOST022",
            "models/marts/model.py",
            "rows = df.collect()",
        ),
        (
            "SQLCOST028",
            "models/marts/fct.sql",
            "{{ config(materialized='incremental', unique_key='id') }} select id from t",
        ),
        (
            "SQLCOST029",
            "models/marts/fct.sql",
            "{{ config(materialized='incremental', unique_key='id', full_refresh=true) }} select id from t",
        ),
        (
            "SQLCOST030",
            "models/marts/fct.sql",
            "select id from orders o where exists (select 1 from line_items li where li.order_id = o.id)",
        ),
        (
            "SQLCOST031",
            "models/marts/fct.sql",
            "select id from users where email like '%@example.com'",
        ),
        (
            "SQLCOST032",
            "models/marts/fct.sql",
            "select id from events where event_date = current_date or block_time >= current_date",
        ),
        (
            "SQLCOST033",
            "models/marts/fct.sql",
            "select * from a join b on a.name like b.token",
        ),
        (
            "SQLCOST034",
            "models/marts/fct.sql",
            "select id, (select max(amount) from payments p where p.user_id = u.id) from users u",
        ),
        (
            "SQLCOST035",
            "models/marts/fct.sql",
            "select * from hive.default.orders o join iceberg.analytics.users u on o.id = u.id",
        ),
        (
            "SQLCOST036",
            "models/marts/fct.sql",
            "select distinct user_id from orders cross join unnest(tag_list) as tag",
        ),
        (
            "SQLCOST037",
            "models/marts/fct.sql",
            "select id from orders where id not in (select order_id from returns)",
        ),
        (
            "SQLCOST040",
            "models/marts/fct.sql",
            "{{ config(materialized='table') }} select id, event_date from events",
        ),
        (
            "SQLCOST041",
            "models/marts/fct.sql",
            "select sum(amount) over (partition by user_id order by ts rows between unbounded preceding and unbounded following) from t",
        ),
        (
            "SQLCOST044",
            "models/marts/fct.sql",
            "with recursive graph as (select 1 as n union all select n + 1 from graph where n < 5) select * from graph",
        ),
    ];

    for (rule_id, path, text) in cases {
        let file = if path.ends_with(".py") {
            python_file(path, text)
        } else {
            sql_file(path, text)
        };
        let doc = if path.ends_with(".py") {
            None
        } else {
            Some(analyze_for_rule(rule_id, &file))
        };
        let docs = doc.iter().cloned().collect::<Vec<_>>();
        let ids = run_for_rule(rule_id, &file, &docs);
        assert!(
            ids.contains(&rule_id.to_string()),
            "missing {rule_id}: {ids:?}"
        );
    }
}

fn analyze_for_rule(rule_id: &str, file: &ProjectFile) -> SqlDocument {
    let platform = match rule_id {
        "SQLCOST021" | "SQLCOST028" | "SQLCOST042" => Platform::BigQuery,
        "SQLCOST035" => Platform::Trino,
        "SQLCOST043" => Platform::Snowflake,
        _ => Platform::Generic,
    };
    let dbt = extract_sql_features(&file.text);
    analyze_sql(
        file.path.clone(),
        &file.text,
        platform,
        &file.line_index,
        None,
        true,
        dbt,
    )
}

fn run_for_rule(rule_id: &str, file: &ProjectFile, docs: &[SqlDocument]) -> Vec<String> {
    let registry = RuleRegistry::default_rules();
    let indexes = ProjectIndexes::from_sql_documents(docs);
    let sql = docs.iter().find(|doc| doc.path == file.path);
    let warehouse = match rule_id {
        "SQLCOST021" | "SQLCOST028" | "SQLCOST042" => Platform::BigQuery,
        "SQLCOST035" => Platform::Trino,
        "SQLCOST043" => Platform::Snowflake,
        _ => Platform::Generic,
    };
    let diagnostics = registry.run(&RuleContext {
        warehouse,
        file,
        sql,
        dbt_model: None,
        all_sql: docs,
        project_indexes: &indexes,
        overrides: &RuleOverrides::default(),
    });
    apply_suppressions(&file.text, diagnostics)
        .into_iter()
        .map(|diagnostic| diagnostic.rule_id)
        .collect()
}

#[test]
fn bigquery_wildcard_rule_has_positive_coverage() {
    let file = sql_file(
        "models/marts/events.sql",
        "select * from `project.dataset.events_*`",
    );
    let doc = analyze_bigquery(&file);
    let registry = RuleRegistry::default_rules();
    let indexes = ProjectIndexes::from_sql_documents(std::slice::from_ref(&doc));
    let diagnostics = registry.run(&RuleContext {
        warehouse: Platform::BigQuery,
        file: &file,
        sql: Some(&doc),
        dbt_model: None,
        all_sql: std::slice::from_ref(&doc),
        project_indexes: &indexes,
        overrides: &RuleOverrides::default(),
    });
    assert!(diagnostics.iter().any(|d| d.rule_id == "SQLCOST021"));
}

fn analyze_bigquery(file: &ProjectFile) -> SqlDocument {
    let dbt = extract_sql_features(&file.text);
    analyze_sql(
        file.path.clone(),
        &file.text,
        Platform::BigQuery,
        &file.line_index,
        None,
        true,
        dbt,
    )
}

#[test]
fn python_rule_has_positive_coverage() {
    let file = python_file(
        "models/marts/model.py",
        "df.apply(lambda row: row['x'], axis=1)",
    );
    let ids = run_for_file(&file, &[]);
    assert!(ids.contains(&"SQLCOST010".to_string()));
}

#[test]
fn cross_file_expensive_expression_uses_project_index() {
    let first = sql_file(
        "models/marts/a.sql",
        "select json_extract(payload, '$.a') from t",
    );
    let second = sql_file(
        "models/marts/b.sql",
        "select json_extract(payload, '$.a') from t",
    );
    let third = sql_file(
        "models/marts/c.sql",
        "select json_extract(payload, '$.a') from t",
    );
    let docs = vec![analyze(&first), analyze(&second), analyze(&third)];
    let ids = run_for_file(&first, &docs);
    assert!(ids.contains(&"SQLCOST015".to_string()));
}

#[test]
fn cross_file_expensive_expression_ignores_comment_prose() {
    let header =
        "-- core list: frozen stablecoin addresses used for initial incremental balances\n";
    let body = "select 'arbitrum' as blockchain, contract_address from (values (0x1)) as t(contract_address)";
    let first = sql_file("models/marts/a.sql", &format!("{header}{body}"));
    let second = sql_file("models/marts/b.sql", &format!("{header}{body}"));
    let third = sql_file("models/marts/c.sql", &format!("{header}{body}"));
    let docs = vec![analyze(&first), analyze(&second), analyze(&third)];
    let ids = run_for_file(&first, &docs);
    assert!(!ids.contains(&"SQLCOST015".to_string()));
    assert!(!ids.contains(&"SQLCOST002".to_string()));
}

#[test]
fn cross_catalog_join_ignores_subquery_alias() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select * from (select token from delta_prod.balancer_v2_arbitrum.events) b \
         left join delta_prod.tokens.erc20 t on t.contract_address = b.token",
    );
    let doc = analyze_for_rule("SQLCOST035", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST035".to_string()));
}

#[test]
fn repeated_cte_ignores_column_and_schema_homonyms() {
    let file = sql_file(
        "models/marts/fct.sql",
        "with pools as (select bpool as pools from t), \
         joins as (select p.pools from delta_prod.prices.usd inner join pools p on true), \
         exits as (select p.pools from delta_prod.prices.usd inner join pools p on true) \
         select * from joins union all select * from exits",
    );
    let doc = analyze_for_rule("SQLCOST014", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST014".to_string()));
}

#[test]
fn balancer_balances_model_does_not_fire_repeated_cte() {
    let raw = include_str!("../../../tests/fixtures/balancer_balances_raw.sql");
    let compiled = include_str!("../../../tests/fixtures/balancer_balances_compiled.sql");
    let file = sql_file(
        "dbt_subprojects/daily_spellbook/models/_projects/balancer_cowswap_amm/arbitrum/balancer_cowswap_amm_arbitrum_balances.sql",
        raw,
    );
    let dbt = extract_sql_features(&file.text);
    let doc = analyze_sql(
        file.path.clone(),
        &file.text,
        Platform::Trino,
        &file.line_index,
        Some(compiled),
        true,
        dbt,
    );
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST014".to_string()));
}

#[test]
fn blind_distinct_skips_when_group_by_present() {
    let file = sql_file(
        "models/marts/fct.sql",
        "with contracts as (select distinct address from t) select address from contracts group by 1",
    );
    let doc = analyze_for_rule("SQLCOST008", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST008".to_string()));
}

#[test]
fn macro_template_skips_function_wrapped_join_rule() {
    let file = sql_file(
        "dbt_subprojects/hourly_spellbook/macros/sector/gas/evm_l1_gas_fees.sql",
        "select * from base b join prices p on p.timestamp = date_trunc('day', b.block_time)",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST017".to_string()));
}

#[test]
fn cross_join_to_scalar_cte_is_not_flagged() {
    let file = sql_file(
        "models/marts/fct.sql",
        "with global_fees as (select fee from rates where rn = 1) \
         select swap.amount from swap cross join global_fees g where g.rn = 1",
    );
    let doc = analyze_for_rule("SQLCOST012", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST012".to_string()));
}

#[test]
fn comma_join_to_scalar_cte_is_not_flagged() {
    let file = sql_file(
        "models/marts/fct.sql",
        "with global_fees as (select fee from rates where rn = 1) \
         select swap.amount from swap, global_fees g where g.rn = 1",
    );
    let doc = analyze_for_rule("SQLCOST012", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST012".to_string()));
}

#[test]
fn cross_join_unnest_is_not_flagged() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select t.x from base cross join unnest(base.items) as t(x)",
    );
    let doc = analyze_for_rule("SQLCOST012", &file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST012".to_string()));
}

#[test]
fn equality_join_from_fp_registry_is_not_unbounded() {
    let file = sql_file(
        "models/marts/fct_safe_join.sql",
        "select * from a join b on a.id = b.id",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn unbounded_join_positive_cases_still_fire() {
    for sql in [
        "select * from orders o join users u on o.amount > u.threshold",
        "select * from a join b on a.id <> b.id",
    ] {
        let file = sql_file("models/marts/fct.sql", sql);
        let doc = analyze(&file);
        let ids = run_for_file(&file, &[doc]);
        assert!(
            ids.contains(&"SQLCOST006".to_string()),
            "expected SQLCOST006 for {sql}"
        );
    }
}

#[test]
fn macro_template_skips_repeated_cte_and_unbounded_join_rules() {
    let file = sql_file(
        "dbt_subprojects/dex/macros/add_amount_usd_dex_trades.sql",
        "with prices as (select price from t), trusted_tokens as (select contract_address from t) \
         select * from x bt left join prices pb on bt.id = pb.id left join prices ps on bt.id = ps.id \
         left join trusted_tokens tt_bought on bt.id = tt_bought.id \
         left join trusted_tokens tt_sold on bt.id = tt_sold.id",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST014".to_string()));
}

#[test]
fn subquery_inner_join_with_trailing_equality_on_is_not_unbounded() {
    let file = sql_file(
        "models/marts/opportunityfieldhistory.sql",
        "SELECT o.id\nFROM source_table o\nINNER JOIN (\n    SELECT id, MAX(seq) AS seq\n    FROM source_table\n    GROUP BY id\n) oo\nON o.id = oo.id\n    AND o.seq = oo.seq",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn jinja_ref_left_join_with_equality_on_next_line_is_not_unbounded() {
    let file = sql_file(
        "models/tokens/tokens_arbitrum_nft.sql",
        "{{ config(alias = 'nft') }}\nSELECT c.contract_address, t.name\nFROM {{ ref('tokens_arbitrum_nft_standards')}} c\nLEFT JOIN {{ref('tokens_arbitrum_nft_curated')}} t\nON c.contract_address = t.contract_address",
    );
    let doc = analyze(&file);
    let parsed = doc.parsed;
    let used_ast = doc.feature_extraction_used_ast;
    let ids = run_for_file(&file, &[doc]);
    assert!(
        !ids.contains(&"SQLCOST006".to_string()),
        "parsed={parsed} ast={used_ast}"
    );
}

#[test]
fn full_outer_join_on_constant_equality_is_not_unbounded() {
    let file = sql_file(
        "tests/check_rowcount.sql",
        "select count_a, count_b\nfrom (select count(*) as count_a from a)\nfull outer join\n    (select count(*) as count_b from b)\non 1=1",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn macro_equality_join_text_is_not_unbounded() {
    let file = sql_file(
        "dbt_subprojects/daily_spellbook/macros/project/tesseract/tesseract_ictt_volume_events.sql",
        "select * from events e inner join recv r on r.evt_tx_hash = e.evt_tx_hash",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn subquery_left_join_with_distant_on_is_not_unbounded() {
    let file = sql_file(
        "models/marts/license_daily_details.sql",
        "select l.* from licenses l left join (select a.user_id, l.date from licenses l join activity a on l.server_id = a.user_id and a.timestamp <= l.date group by 1, 2) a on l.server_id = a.user_id and a.date = l.date",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn dbt_macros_path_skips_unbounded_join_rule() {
    let file = sql_file(
        "dbt_macros/dune/adapters.sql",
        "select * from a cross join b on a.id = b.id",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn coalesce_cast_both_sides_equality_join_is_not_unbounded() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select * from orders o join users u on coalesce(o.user_id, 0) = coalesce(u.user_id, 0)",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn regex_using_join_is_not_unbounded() {
    let file = sql_file(
        "models/marts/fct.sql",
        "{{ not_parsable }} select * from orders o join users u using (user_id)",
    );
    let doc = analyze(&file);
    assert!(!doc.feature_extraction_used_ast);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST006".to_string()));
}

#[test]
fn regex_only_unbounded_join_emits_low_confidence() {
    let file = sql_file(
        "models/marts/fct.sql",
        "{{ not_parsable }} select * from a join b",
    );
    let doc = analyze(&file);
    assert!(!doc.feature_extraction_used_ast);
    let diagnostics = run_diagnostics_for_file(&file, &[doc]);
    let sqlcost006 = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule_id == "SQLCOST006")
        .expect("expected SQLCOST006");
    assert_eq!(sqlcost006.confidence, Confidence::Low);
}

#[test]
fn ast_equality_join_emits_medium_confidence_when_rule_fires() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select * from a join b on a.id > b.id",
    );
    let doc = analyze(&file);
    assert!(doc.feature_extraction_used_ast);
    let diagnostics = run_diagnostics_for_file(&file, &[doc]);
    let sqlcost006 = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule_id == "SQLCOST006")
        .expect("expected SQLCOST006");
    assert_eq!(sqlcost006.confidence, Confidence::Medium);
}

#[test]
fn cross_join_confidence_matches_join_kind_and_extraction() {
    let cross_file = sql_file("models/marts/fct.sql", "select * from a cross join b");
    let cross_doc = analyze_for_rule("SQLCOST012", &cross_file);
    assert!(cross_doc.feature_extraction_used_ast);
    let cross = run_diagnostics_for_file(&cross_file, &[cross_doc])
        .into_iter()
        .find(|diagnostic| diagnostic.rule_id == "SQLCOST012")
        .expect("expected SQLCOST012 cross join");
    assert_eq!(cross.confidence, Confidence::High);

    let comma_file = sql_file("models/marts/fct.sql", "select * from a, b");
    let comma_doc = analyze_for_rule("SQLCOST012", &comma_file);
    assert!(comma_doc.feature_extraction_used_ast);
    let comma = run_diagnostics_for_file(&comma_file, &[comma_doc])
        .into_iter()
        .find(|diagnostic| diagnostic.rule_id == "SQLCOST012")
        .expect("expected SQLCOST012 comma join");
    assert_eq!(comma.confidence, Confidence::Medium);

    let regex_file = sql_file(
        "models/marts/fct.sql",
        "{{ not_parsable }} select * from a, b",
    );
    let regex_doc = analyze(&regex_file);
    assert!(!regex_doc.feature_extraction_used_ast);
    let regex = run_diagnostics_for_file(&regex_file, &[regex_doc])
        .into_iter()
        .find(|diagnostic| diagnostic.rule_id == "SQLCOST012")
        .expect("expected SQLCOST012 regex comma join");
    assert_eq!(regex.confidence, Confidence::Low);
}

#[test]
fn suppression_still_applies_after_rule_split() {
    let file = sql_file(
        "models/marts/fct.sql",
        "-- costguard: disable-next-line=SQLCOST001\nselect * from x",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST001".to_string()));
}

#[test]
fn representative_negative_cases_do_not_fire() {
    let file = sql_file("models/staging/stg.sql", "select id from t limit 1");
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST001".to_string()));
    assert!(!ids.contains(&"SQLCOST007".to_string()));
}

#[test]
fn append_incremental_without_unique_key_does_not_fire_sqlcost004() {
    let file = sql_file(
        "models/marts/fct.sql",
        "{{ config(materialized='incremental', incremental_strategy='append') }} select id from t",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST004".to_string()));
}

#[test]
fn incremental_config_predicates_do_not_fire_sqlcost005() {
    let file = sql_file(
        "models/marts/bridges.sql",
        "{{ config(materialized='incremental', unique_key=['transfer_id'], incremental_predicates=[incremental_predicate('DBT_INTERNAL_DEST.evt_block_time')]) }}
with source_data as (select evt_block_time from {{ source('dex', 'trades') }})
select * from source_data
{% if is_incremental() %}
where {{ incremental_predicate('p.minute') }}
{% endif %}",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST005".to_string()));
    assert!(!ids.contains(&"SQLCOST004".to_string()));
}

#[test]
fn inline_array_unique_key_does_not_fire_sqlcost004() {
    let file = sql_file(
        "models/marts/fct.sql",
        "{{ config(materialized='incremental', unique_key=['account_id', 'event_date']) }} select id from t",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST004".to_string()));
}

#[test]
fn block_time_incremental_predicate_does_not_fire_sqlcost005() {
    let file = sql_file(
        "models/marts/dex_trades.sql",
        "{{ config(materialized='incremental', unique_key='tx_hash') }}
select tx_hash, block_time from {{ source('dex', 'trades') }}
{% if is_incremental() %}
where block_time >= date_sub(current_date(), interval 3 day)
{% endif %}",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST005".to_string()));
}

#[test]
fn block_time_incremental_predicate_on_source_does_not_fire_sqlcost019() {
    let file = sql_file(
        "models/marts/dex_trades.sql",
        "{{ config(materialized='incremental', unique_key='tx_hash') }}
select tx_hash, block_time from {{ source('dex', 'trades') }}
{% if is_incremental() %}
where block_time >= date_sub(current_date(), interval 3 day)
{% endif %}",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST019".to_string()));
}

#[test]
fn fan_out_join_rule_uses_project_indexes() {
    let dim_users = sql_file(
        "models/marts/dim_users.sql",
        "{{ config(unique_key='user_id') }} select user_id, email from users",
    );
    let fct = sql_file(
        "models/marts/fct_orders.sql",
        "select * from orders join dim_users on orders.email = dim_users.email",
    );
    let docs = vec![analyze(&dim_users), analyze(&fct)];
    let ids = run_for_rule("SQLCOST038", &fct, &docs);
    assert!(ids.contains(&"SQLCOST038".to_string()));
}

#[test]
fn fan_out_join_rule_matches_ref_aliases() {
    let dim_users = sql_file(
        "models/marts/dim_users.sql",
        "{{ config(unique_key='user_id') }} select user_id, email from users",
    );
    let fct = sql_file(
        "models/marts/fct_orders.sql",
        "select * from orders join {{ ref('dim_users') }} u on orders.email = u.email",
    );
    let docs = vec![analyze(&dim_users), analyze(&fct)];
    let ids = run_for_rule("SQLCOST038", &fct, &docs);
    assert!(ids.contains(&"SQLCOST038".to_string()));
}

#[test]
fn heavily_referenced_view_rule_uses_project_indexes() {
    let shared = sql_file(
        "models/intermediate/int_shared.sql",
        "{{ config(materialized='view') }} select id, event_date from base",
    );
    let downstream: Vec<ProjectFile> = (1..=4)
        .map(|idx| {
            sql_file(
                &format!("models/marts/fct_{idx}.sql"),
                "select id from {{ ref('int_shared') }}",
            )
        })
        .collect();
    let docs = std::iter::once(analyze(&shared))
        .chain(downstream.iter().map(analyze))
        .collect::<Vec<_>>();
    let ids = run_for_rule("SQLCOST039", &shared, &docs);
    assert!(ids.contains(&"SQLCOST039".to_string()));
}

#[test]
fn bigquery_missing_partition_filter_has_positive_coverage() {
    let file = sql_file(
        "models/marts/events.sql",
        "select * from {{ ref('stg_events') }}",
    );
    let doc = analyze_bigquery(&file);
    let registry = RuleRegistry::default_rules();
    let indexes = ProjectIndexes::from_sql_documents(std::slice::from_ref(&doc));
    let diagnostics = registry.run(&RuleContext {
        warehouse: Platform::BigQuery,
        file: &file,
        sql: Some(&doc),
        dbt_model: None,
        all_sql: std::slice::from_ref(&doc),
        project_indexes: &indexes,
        overrides: &RuleOverrides::default(),
    });
    assert!(diagnostics.iter().any(|d| d.rule_id == "SQLCOST042"));
}

#[test]
fn snowflake_merge_without_target_pruning_has_positive_coverage() {
    let file = sql_file(
        "models/marts/fct.sql",
        "{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id') }}
select id, updated_at from events
{% if is_incremental() %}
where updated_at >= current_date - 3
{% endif %}",
    );
    let doc = analyze_for_rule("SQLCOST043", &file);
    let ids = run_for_rule("SQLCOST043", &file, &[doc]);
    assert!(ids.contains(&"SQLCOST043".to_string()));
}

#[test]
fn row_explosion_without_dedup_does_not_fire_sqlcost036() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select user_id, tag from orders cross join unnest(tag_list) as tag",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST036".to_string()));
}

#[test]
fn plain_in_subquery_does_not_fire_sqlcost037() {
    let file = sql_file(
        "models/marts/fct.sql",
        "select id from orders where id in (select order_id from returns)",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST037".to_string()));
}

#[test]
fn non_recursive_cte_does_not_fire_sqlcost044() {
    let file = sql_file(
        "models/marts/fct.sql",
        "with graph as (select 1 as n) select * from graph",
    );
    let doc = analyze(&file);
    let ids = run_for_file(&file, &[doc]);
    assert!(!ids.contains(&"SQLCOST044".to_string()));
}

#[test]
fn registry_matches_builtin_catalog() {
    let registry = RuleRegistry::default_rules();
    let metadata = registry.metadata();
    assert_eq!(metadata.len(), costguard_protocol::BUILTIN_RULE_IDS.len());
    for entry in &metadata {
        assert!(costguard_protocol::is_builtin_rule_id(entry.id));
    }
    for rule_id in costguard_protocol::BUILTIN_RULE_IDS {
        assert!(
            metadata.iter().any(|entry| entry.id == rule_id),
            "missing registry rule {rule_id}"
        );
    }
}
