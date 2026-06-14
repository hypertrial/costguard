use costguard_diagnostics::{normalize_evidence_text, EvidenceBuilder};
use costguard_sql::{JoinFeature, JoinKind};

pub fn expr_key(key: &str) -> String {
    EvidenceBuilder::new("expr")
        .field("key", normalize_evidence_text(key))
        .build()
}

pub fn join_unbounded(join: &JoinFeature) -> String {
    let predicate = join
        .predicate
        .as_deref()
        .map(normalize_evidence_text)
        .unwrap_or_else(|| "none".into());
    EvidenceBuilder::new("join_unbounded")
        .field("kind", join_kind_label(join.kind))
        .field("predicate", predicate)
        .build()
}

pub fn join_cross(join: &JoinFeature) -> String {
    EvidenceBuilder::new("join_cross")
        .field("kind", join_kind_label(join.kind))
        .field(
            "right",
            join.right_relation
                .as_deref()
                .map(normalize_evidence_text)
                .unwrap_or_else(|| "unknown".into()),
        )
        .build()
}

pub fn join_fn_wrapped(join: &JoinFeature) -> String {
    let predicate = join
        .predicate
        .as_deref()
        .map(normalize_evidence_text)
        .unwrap_or_else(|| "none".into());
    EvidenceBuilder::new("join_fn_wrapped")
        .field("kind", join_kind_label(join.kind))
        .field("predicate", predicate)
        .build()
}

pub fn join_pattern(join: &JoinFeature) -> String {
    let predicate = join
        .predicate
        .as_deref()
        .map(normalize_evidence_text)
        .unwrap_or_else(|| "none".into());
    EvidenceBuilder::new("join_pattern")
        .field("predicate", predicate)
        .build()
}

pub fn join_cross_catalog(join: &JoinFeature) -> String {
    let predicate = join
        .predicate
        .as_deref()
        .map(normalize_evidence_text)
        .unwrap_or_else(|| "none".into());
    EvidenceBuilder::new("join_cross_catalog")
        .field("predicate", predicate)
        .build()
}

pub fn join_fan_out(model: &str, equality_keys: &[String], unique_key: &[String]) -> String {
    EvidenceBuilder::new("join_fan_out")
        .field("model", normalize_evidence_text(model))
        .field("keys", equality_keys.join(","))
        .field("grain", unique_key.join(","))
        .build()
}

pub fn literal(kind: &str) -> String {
    EvidenceBuilder::new("literal").field("kind", kind).build()
}

pub fn window_text(text: &str) -> String {
    EvidenceBuilder::new("window")
        .field("text", normalize_evidence_text(text))
        .build()
}

pub fn cte_name(name: &str) -> String {
    EvidenceBuilder::new("cte")
        .field("name", normalize_evidence_text(name))
        .build()
}

pub fn dbt_config(kind: &str, fields: &[(&str, &str)]) -> String {
    let mut builder = EvidenceBuilder::new(kind);
    for (key, value) in fields {
        builder = builder.field(*key, *value);
    }
    builder.build()
}

pub fn dbt_sources(sources: &[costguard_dbt::DbtSourceRef]) -> String {
    let mut names = sources
        .iter()
        .map(|s| format!("{}.{}", s.source_name, s.table_name))
        .collect::<Vec<_>>();
    names.sort();
    EvidenceBuilder::new("dbt_sources")
        .field("sources", names.join(","))
        .build()
}

pub fn heavy_view(model: &str, materialized: &str) -> String {
    EvidenceBuilder::new("heavy_view")
        .field("model", normalize_evidence_text(model))
        .field("materialized", materialized)
        .build()
}

pub fn python_pattern(pattern: &str) -> String {
    EvidenceBuilder::new("python")
        .field("pattern", pattern)
        .build()
}

#[allow(dead_code)]
pub fn metadata_warning(kind: &str, subject: &str) -> String {
    EvidenceBuilder::new("metadata")
        .field("kind", kind)
        .field("subject", subject)
        .build()
}

#[allow(dead_code)]
pub fn parse_failure(filename: &str) -> String {
    EvidenceBuilder::new("parse_failure")
        .field("file", filename)
        .build()
}

pub fn declarative(rule_id: &str, predicate_hash: &str) -> String {
    EvidenceBuilder::new("declarative")
        .field("rule", rule_id)
        .field("predicate", predicate_hash)
        .build()
}

fn join_kind_label(kind: JoinKind) -> String {
    match kind {
        JoinKind::Inner => "inner",
        JoinKind::Left => "left",
        JoinKind::Right => "right",
        JoinKind::Full => "full",
        JoinKind::Cross => "cross",
        JoinKind::Comma => "comma",
    }
    .into()
}
