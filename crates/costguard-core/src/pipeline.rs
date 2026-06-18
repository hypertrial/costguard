use crate::baseline::{apply_finding_baseline, FindingBaseline};
use crate::config::ScanConfig;
use costguard_diagnostics::{Confidence, Diagnostic, SourceProvenance, Suppressions};
use costguard_policy::{apply_governance, PolicyViolation, ResolvedPolicy};
use costguard_rules::{RuleOverride, RuleOverrides, RuleRegistry};
use costguard_scanner::ProjectFile;
use std::collections::HashMap;
use std::path::PathBuf;

/// Result of semantic identity assignment and duplicate collapse.
#[derive(Debug, Clone, Default)]
pub struct IdentityResult {
    pub diagnostics: Vec<Diagnostic>,
}

/// Whether a diagnostic is eligible for inline suppression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionScope {
    Eligible,
    Metadata,
}

pub fn suppression_scope(file: &ProjectFile) -> SuppressionScope {
    match file.kind {
        costguard_scanner::FileKind::Sql
        | costguard_scanner::FileKind::DbtSqlModel
        | costguard_scanner::FileKind::Python => SuppressionScope::Eligible,
        _ => SuppressionScope::Metadata,
    }
}

/// Stage 1: validate rule registration against the built-in catalog.
pub fn validate_rule_registration(registry: &RuleRegistry) -> anyhow::Result<()> {
    let metadata = registry.metadata();
    if metadata.len() != costguard_protocol::BUILTIN_RULE_IDS.len() {
        anyhow::bail!(
            "rule registry has {} rules; expected {}",
            metadata.len(),
            costguard_protocol::BUILTIN_RULE_IDS.len()
        );
    }
    for entry in &metadata {
        if !costguard_protocol::is_builtin_rule_id(entry.id) {
            anyhow::bail!("unknown built-in rule id '{}'", entry.id);
        }
    }
    Ok(())
}

/// Stage 2: merge per-path local and signed-policy overrides.
pub fn effective_rule_overrides(
    config: &ScanConfig,
    policy: Option<&ResolvedPolicy>,
) -> RuleOverrides {
    let mut overrides = if policy.is_none()
        || policy.is_some_and(|policy| {
            policy.document.permissions.allow_local_severity_overrides
                || policy.document.permissions.allow_cli_overrides
        }) {
        config.rule_overrides.clone()
    } else {
        RuleOverrides::default()
    };
    if let Some(policy) = policy {
        for (rule_id, settings) in &policy.rules {
            let entry = overrides
                .entry(rule_id.clone())
                .or_insert_with(RuleOverride::default);
            if let Some(enabled) = settings.enabled {
                entry.enabled = Some(enabled);
            }
            if let Some(severity) = settings.severity {
                entry.severity = Some(severity);
            }
            if let Some(threshold) = settings.threshold {
                entry.threshold = Some(threshold);
            }
        }
    }
    overrides
}

/// Stage 3: apply enabled filtering and severity replacement.
pub fn apply_enabled_and_severity(
    diagnostics: Vec<Diagnostic>,
    overrides: &RuleOverrides,
) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            overrides
                .get(&diagnostic.rule_id)
                .and_then(|settings| settings.enabled)
                .unwrap_or(true)
        })
        .map(|mut diagnostic| {
            if let Some(severity) = overrides
                .get(&diagnostic.rule_id)
                .and_then(|settings| settings.severity)
            {
                diagnostic.severity = severity;
            }
            diagnostic
        })
        .collect()
}

/// Stage 4: apply eligible inline suppressions.
pub fn apply_inline_suppressions(
    diagnostics: Vec<Diagnostic>,
    file_texts: &HashMap<PathBuf, String>,
    file_scopes: &HashMap<PathBuf, SuppressionScope>,
    allow_inline: bool,
) -> Vec<Diagnostic> {
    if !allow_inline {
        return diagnostics;
    }
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            let scope = file_scopes
                .get(&diagnostic.path)
                .copied()
                .unwrap_or(SuppressionScope::Metadata);
            if scope == SuppressionScope::Metadata {
                return true;
            }
            let text = file_texts
                .get(&diagnostic.path)
                .map(String::as_str)
                .unwrap_or("");
            !Suppressions::parse(text).is_suppressed(diagnostic)
        })
        .collect()
}

/// Stable ordering for diagnostics in output and repeat-run checks.
pub(crate) fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> std::cmp::Ordering {
    left.path
        .cmp(&right.path)
        .then(left.line.cmp(&right.line))
        .then(left.rule_id.cmp(&right.rule_id))
        .then(left.column.cmp(&right.column))
        .then(
            left.governance
                .evidence_key
                .cmp(&right.governance.evidence_key),
        )
        .then(left.governance.finding_id.cmp(&right.governance.finding_id))
}

/// Stage 5: assign semantic identities, fail on missing evidence, collapse duplicates.
pub fn assign_semantic_identities(diagnostics: Vec<Diagnostic>) -> anyhow::Result<IdentityResult> {
    let mut sorted = diagnostics;
    sorted.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.rule_id.cmp(&right.rule_id))
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });

    for diagnostic in &mut sorted {
        if diagnostic.governance.evidence_key.is_empty() {
            anyhow::bail!(
                "diagnostic {} at {} lacks semantic evidence",
                diagnostic.rule_id,
                diagnostic.path.display()
            );
        }
        let semantic_evidence = diagnostic.governance.evidence_key.clone();
        diagnostic.assign_identity(semantic_evidence);
    }

    Ok(IdentityResult {
        diagnostics: collapse_duplicates(sorted),
    })
}

fn collapse_duplicates(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut groups: HashMap<(String, String, String), Vec<Diagnostic>> = HashMap::new();
    for diagnostic in diagnostics {
        let path = costguard_diagnostics::posix_path(&diagnostic.path);
        let key = (
            diagnostic.rule_id.to_ascii_uppercase(),
            path,
            diagnostic.governance.evidence_key.clone(),
        );
        groups.entry(key).or_default().push(diagnostic);
    }
    let mut result = Vec::new();
    for (_, mut group) in groups {
        group.sort_by(|left, right| {
            right
                .confidence
                .cmp(&left.confidence)
                .then(span_precision(right.span).cmp(&span_precision(left.span)))
                .then(left.line.cmp(&right.line))
                .then(left.column.cmp(&right.column))
        });
        result.push(group.remove(0));
    }
    result.sort_by(compare_diagnostics);
    result
}

fn span_precision(span: Option<costguard_diagnostics::Span>) -> (u8, usize) {
    let provenance = match span.and_then(|s| s.source_provenance) {
        Some(SourceProvenance::Raw) => 2,
        Some(SourceProvenance::Compiled) => 1,
        Some(SourceProvenance::CompiledUnmapped) | None => 0,
    };
    let width = span
        .map(|s| s.byte_end.saturating_sub(s.byte_start))
        .unwrap_or(usize::MAX);
    (provenance, width)
}

/// Stage 6: apply signed-policy governance and exceptions.
pub fn apply_policy_governance(
    diagnostics: &mut [Diagnostic],
    resolve: &dyn Fn(&str) -> anyhow::Result<ResolvedPolicy>,
    repository: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<PolicyViolation>> {
    let mut violations = Vec::new();
    for diagnostic in diagnostics {
        let path = costguard_diagnostics::posix_path(&diagnostic.path);
        let resolved = resolve(&path)?;
        violations.extend(apply_governance(
            std::slice::from_mut(diagnostic),
            &resolved,
            repository,
            now,
        )?);
    }
    violations.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then(left.message.cmp(&right.message))
    });
    violations.dedup_by(|left, right| left.code == right.code && left.message == right.message);
    Ok(violations)
}

/// Stage 7: baseline filtering (cost attribution happens separately).
pub fn apply_baseline_filter(
    diagnostics: Vec<Diagnostic>,
    baseline: Option<&FindingBaseline>,
) -> (Vec<Diagnostic>, usize) {
    if let Some(baseline) = baseline {
        apply_finding_baseline(diagnostics, baseline)
    } else {
        (diagnostics, 0)
    }
}

/// Drop diagnostics below `min_confidence` when output filtering is enabled.
pub fn filter_by_min_confidence(
    diagnostics: Vec<Diagnostic>,
    min_confidence: Option<Confidence>,
    enabled: bool,
) -> Vec<Diagnostic> {
    if !enabled {
        return diagnostics;
    }
    let Some(threshold) = min_confidence else {
        return diagnostics;
    };
    diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.confidence >= threshold)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, DiagnosticGovernance, Severity};
    use std::path::PathBuf;

    fn sample(rule_id: &str, evidence: &str, line: usize, confidence: Confidence) -> Diagnostic {
        let mut diagnostic = Diagnostic {
            governance: DiagnosticGovernance {
                evidence_key: evidence.into(),
                ..Default::default()
            },
            rule_id: rule_id.into(),
            severity: Severity::Medium,
            path: PathBuf::from("models/a.sql"),
            line,
            column: 1,
            span: None,
            message: "test".into(),
            risk: None,
            suggestion: None,
            confidence,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: None,
            rule_precision_tier: None,
        };
        diagnostic.assign_identity(evidence);
        diagnostic
    }

    #[test]
    fn collapses_duplicate_semantic_findings() {
        let left = sample("SQLCOST001", "sem-v1:literal:abc", 1, Confidence::High);
        let right = sample("SQLCOST001", "sem-v1:literal:abc", 5, Confidence::Medium);
        let result = assign_semantic_identities(vec![left, right]).expect("identity");
        assert_eq!(result.diagnostics.len(), 1);
    }

    #[test]
    fn rejects_missing_evidence() {
        let mut diagnostic = sample("SQLCOST001", "sem-v1:literal:abc", 1, Confidence::High);
        diagnostic.governance.evidence_key.clear();
        let err = assign_semantic_identities(vec![diagnostic]).unwrap_err();
        assert!(err.to_string().contains("lacks semantic evidence"));
    }

    #[test]
    fn filter_by_min_confidence_drops_below_threshold_when_enabled() {
        let low = sample("SQLCOST012", "sem-v1:join:comma", 1, Confidence::Low);
        let medium = sample("SQLCOST012", "sem-v1:join:comma2", 2, Confidence::Medium);
        let high = sample("SQLCOST012", "sem-v1:join:cross", 3, Confidence::High);
        let inputs = vec![low, medium, high];
        let filtered =
            super::filter_by_min_confidence(inputs.clone(), Some(Confidence::High), true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].confidence, Confidence::High);

        let kept = super::filter_by_min_confidence(inputs.clone(), Some(Confidence::High), false);
        assert_eq!(kept.len(), 3);

        let unset = super::filter_by_min_confidence(inputs, None, true);
        assert_eq!(unset.len(), 3);
    }
}
