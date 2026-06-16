use anyhow::{Context, Result};
use costguard_diagnostics::Diagnostic;
use costguard_sql::Platform;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Versioned baseline of accepted findings, keyed by stable [`finding_id`](BaselinedFinding::finding_id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FindingBaseline {
    pub version: u8,
    pub identity_scheme: String,
    pub platform: String,
    pub generated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    pub findings: Vec<BaselinedFinding>,
}

/// A single baselined finding identified by rule, path, and evidence key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BaselinedFinding {
    pub finding_id: String,
    pub rule_id: String,
    pub path: String,
    pub evidence_key: String,
}

impl FindingBaseline {
    pub fn from_diagnostics(
        diagnostics: &[Diagnostic],
        platform: Platform,
        policy_digest: Option<String>,
    ) -> Self {
        let mut findings = diagnostics
            .iter()
            .map(|diagnostic| BaselinedFinding {
                finding_id: diagnostic.governance.finding_id.clone(),
                rule_id: diagnostic.rule_id.clone(),
                path: normalize_path(&diagnostic.path),
                evidence_key: diagnostic.governance.evidence_key.clone(),
            })
            .collect::<Vec<_>>();
        sort_and_deduplicate(&mut findings);
        Self {
            version: costguard_protocol::BASELINE_SCHEMA_VERSION,
            identity_scheme: costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1.into(),
            platform: platform.to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            policy_digest,
            findings,
        }
    }

    pub fn finding_id_set(&self) -> HashSet<&str> {
        self.findings
            .iter()
            .map(|finding| finding.finding_id.as_str())
            .collect()
    }
}

pub fn load_finding_baseline(path: &Path) -> Result<FindingBaseline> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read baseline {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid baseline JSON {}", path.display()))
}

pub fn validate_finding_baseline(baseline: &FindingBaseline, platform: Platform) -> Result<()> {
    if baseline.version != costguard_protocol::BASELINE_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported baseline schema version {}; expected {}",
            baseline.version,
            costguard_protocol::BASELINE_SCHEMA_VERSION
        );
    }
    if baseline.identity_scheme != costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1 {
        anyhow::bail!(
            "baseline identity_scheme '{}' is unsupported; expected '{}'",
            baseline.identity_scheme,
            costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1
        );
    }
    let expected = baseline
        .platform
        .parse::<Platform>()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid baseline platform '{}'", baseline.platform))?;
    if expected != platform {
        anyhow::bail!("baseline platform '{expected}' does not match scan platform '{platform}'");
    }
    if baseline
        .findings
        .iter()
        .any(|finding| finding.finding_id.is_empty())
    {
        anyhow::bail!("baseline contains an empty finding_id");
    }
    Ok(())
}

pub fn write_finding_baseline(path: &Path, baseline: &FindingBaseline) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(baseline)?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write baseline {}", path.display()))
}

pub fn apply_finding_baseline(
    diagnostics: Vec<Diagnostic>,
    baseline: &FindingBaseline,
) -> (Vec<Diagnostic>, usize) {
    let known = baseline.finding_id_set();
    let mut baselined = 0;
    let filtered = diagnostics
        .into_iter()
        .filter(|diagnostic| {
            if known.contains(diagnostic.governance.finding_id.as_str()) {
                baselined += 1;
                false
            } else {
                true
            }
        })
        .collect();
    (filtered, baselined)
}

pub fn merge_baseline_findings(
    existing: &FindingBaseline,
    diagnostics: &[Diagnostic],
    platform: Platform,
    policy_digest: Option<String>,
) -> FindingBaseline {
    let mut baseline = FindingBaseline::from_diagnostics(diagnostics, platform, policy_digest);
    baseline.findings.extend(existing.findings.clone());
    sort_and_deduplicate(&mut baseline.findings);
    baseline
}

fn sort_and_deduplicate(findings: &mut Vec<BaselinedFinding>) {
    findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.rule_id.cmp(&right.rule_id))
            .then(left.finding_id.cmp(&right.finding_id))
    });
    findings.dedup_by(|left, right| left.finding_id == right.finding_id);
}

fn normalize_path(path: &Path) -> String {
    costguard_diagnostics::posix_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, DiagnosticGovernance, Severity};
    use std::path::PathBuf;

    fn sample_diagnostic(message: &str) -> Diagnostic {
        let mut diagnostic = Diagnostic {
            governance: DiagnosticGovernance::default(),
            rule_id: "SQLCOST001".into(),
            severity: Severity::Medium,
            path: PathBuf::from("models/marts/foo.sql"),
            line: 3,
            column: 5,
            span: None,
            message: message.into(),
            risk: None,
            suggestion: None,
            confidence: Confidence::High,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: None,
        };
        diagnostic.assign_identity("primary");
        diagnostic
    }

    #[test]
    fn finding_identity_ignores_message_and_location() {
        let mut left = sample_diagnostic("first wording");
        let mut right = sample_diagnostic("completely different wording");
        right.line = 500;
        left.assign_identity("same-evidence");
        right.assign_identity("same-evidence");
        assert_eq!(left.governance.finding_id, right.governance.finding_id);
    }

    #[test]
    fn baseline_filters_known_findings() {
        let diagnostics = vec![sample_diagnostic("known"), sample_diagnostic("new finding")];
        let baseline =
            FindingBaseline::from_diagnostics(&[diagnostics[0].clone()], Platform::Generic, None);
        let (filtered, baselined) = apply_finding_baseline(diagnostics, &baseline);
        assert_eq!(baselined, 2);
        assert!(filtered.is_empty());
    }

    #[test]
    fn baseline_rejects_version_and_platform_mismatch() {
        let mut baseline = FindingBaseline::from_diagnostics(&[], Platform::Snowflake, None);
        baseline.version = 2;
        assert!(validate_finding_baseline(&baseline, Platform::Snowflake).is_err());
        baseline.version = costguard_protocol::BASELINE_SCHEMA_VERSION;
        assert!(validate_finding_baseline(&baseline, Platform::BigQuery).is_err());
    }
}
