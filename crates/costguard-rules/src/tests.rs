use crate::{ProjectIndexes, RuleContext, RuleOverrides, RuleRegistry, Warehouse};
use costguard_diagnostics::{apply_suppressions, LineIndex};
use costguard_scanner::{FileKind, ProjectFile};
use costguard_sql::{analyze_sql, SqlDialect, SqlDocument};
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
        warehouse: Warehouse::Generic,
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
        SqlDialect::Generic,
        &file.line_index,
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
    ];

    for (rule_id, path, text) in cases {
        let file = sql_file(path, text);
        let doc = analyze(&file);
        let ids = run_for_file(&file, &[doc]);
        assert!(
            ids.contains(&rule_id.to_string()),
            "missing {rule_id}: {ids:?}"
        );
    }
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
