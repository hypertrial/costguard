use crate::{ProjectIndexes, RuleContext, RuleOverrides, RuleRegistry};
use costguard_diagnostics::{apply_suppressions, LineIndex};
use costguard_platform::Platform;
use costguard_scanner::{FileKind, ProjectFile};
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

fn run_for_file(file: &ProjectFile, docs: &[SqlDocument]) -> Vec<String> {
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
        .into_iter()
        .map(|diagnostic| diagnostic.rule_id)
        .collect()
}

fn analyze(file: &ProjectFile) -> SqlDocument {
    analyze_sql(
        file.path.clone(),
        &file.text,
        Platform::Generic,
        &file.line_index,
        None,
        true,
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
            "with c as (select * from t) select * from c join c b on c.id = b.id",
        ),
        (
            "SQLCOST016",
            "models/marts/fct.sql",
            "select id from t where date(block_time) = current_date",
        ),
        (
            "SQLCOST017",
            "models/marts/fct.sql",
            "select * from events e join users u on lower(e.email) = lower(u.email)",
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
            Some(analyze(&file))
        };
        let docs = doc.iter().cloned().collect::<Vec<_>>();
        let ids = run_for_file(&file, &docs);
        assert!(
            ids.contains(&rule_id.to_string()),
            "missing {rule_id}: {ids:?}"
        );
    }
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
    analyze_sql(
        file.path.clone(),
        &file.text,
        Platform::BigQuery,
        &file.line_index,
        None,
        true,
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
    let docs = vec![analyze(&first), analyze(&second)];
    let ids = run_for_file(&first, &docs);
    assert!(ids.contains(&"SQLCOST015".to_string()));
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
