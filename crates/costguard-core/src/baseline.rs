use anyhow::{Context, Result};
use costguard_diagnostics::Diagnostic;
use costguard_platform::Platform;
use costguard_policy::{IdentityMap, IdentityMapEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::config::ScanConfig;
use crate::pipeline::LegacyAlias;

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

/// Baseline schema version 2 (ordinal finding IDs, pre-semantic identity).
#[derive(Debug, Clone, Deserialize)]
pub struct FindingBaselineV2 {
    pub version: u8,
    pub platform: String,
    pub generated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    pub findings: Vec<BaselinedFinding>,
}

#[derive(Debug, Deserialize)]
pub struct LegacyFindingBaselineV1 {
    pub version: u8,
    #[serde(default)]
    pub warehouse: Option<String>,
    pub findings: Vec<LegacyBaselinedFindingV1>,
}

#[derive(Debug, Deserialize)]
pub struct LegacyBaselinedFindingV1 {
    pub rule_id: String,
    pub path: String,
    #[serde(default)]
    pub message: Option<String>,
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

pub fn load_identity_map(path: &Path) -> Result<IdentityMap> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read identity map {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("invalid identity map JSON {}", path.display()))
}

pub fn load_baseline_v2(path: &Path) -> Result<FindingBaselineV2> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read baseline {}", path.display()))?;
    let baseline: FindingBaselineV2 = serde_json::from_str(&text)
        .with_context(|| format!("invalid baseline JSON {}", path.display()))?;
    if baseline.version != 2 {
        anyhow::bail!(
            "baseline must have version 2 for migration; found {}",
            baseline.version
        );
    }
    Ok(baseline)
}

pub fn generate_identity_map(
    config: &ScanConfig,
    legacy_aliases: Vec<LegacyAlias>,
    policy_digest: Option<String>,
) -> Result<IdentityMap> {
    let entries = legacy_aliases
        .into_iter()
        .map(|alias| IdentityMapEntry {
            rule_id: alias.rule_id,
            path: alias.path,
            old_evidence_key: alias.old_evidence_key,
            new_evidence_key: alias.new_evidence_key,
            old_finding_id: alias.old_finding_id,
            new_finding_id: alias.new_finding_id,
        })
        .collect::<Vec<_>>();
    validate_identity_map_entries(&entries)?;
    Ok(IdentityMap {
        schema_version: costguard_protocol::IDENTITY_MAP_SCHEMA_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").into(),
        platform: config.platform.to_string(),
        source_policy_digest: policy_digest.filter(|digest| digest != "local-unmanaged"),
        source_identity_scheme: costguard_protocol::IDENTITY_SCHEME_ORDINAL_V1.into(),
        target_identity_scheme: costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1.into(),
        entries,
    })
}

pub fn migrate_baseline_v2(
    baseline: &FindingBaselineV2,
    map: &IdentityMap,
    policy_digest: Option<String>,
) -> Result<FindingBaseline> {
    if map.target_identity_scheme != costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1 {
        anyhow::bail!(
            "identity map target scheme '{}' is unsupported; expected '{}'",
            map.target_identity_scheme,
            costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1
        );
    }
    if map.platform != baseline.platform {
        anyhow::bail!(
            "identity map platform '{}' does not match baseline platform '{}'",
            map.platform,
            baseline.platform
        );
    }
    let finding_map = identity_map_by_finding_id(map)?;
    let mut findings = Vec::with_capacity(baseline.findings.len());
    for old in &baseline.findings {
        let entry = finding_map.get(&old.finding_id).with_context(|| {
            format!("no identity map entry for finding_id '{}'", old.finding_id)
        })?;
        if entry.rule_id != old.rule_id
            || normalize_string_path(&entry.path) != normalize_string_path(&old.path)
        {
            anyhow::bail!(
                "identity map entry for finding_id '{}' does not match baseline rule_id/path",
                old.finding_id
            );
        }
        if entry.old_evidence_key != old.evidence_key {
            anyhow::bail!(
                "identity map entry for finding_id '{}' does not match baseline evidence_key",
                old.finding_id
            );
        }
        findings.push(BaselinedFinding {
            finding_id: entry.new_finding_id.clone(),
            rule_id: old.rule_id.clone(),
            path: normalize_string_path(&old.path),
            evidence_key: entry.new_evidence_key.clone(),
        });
    }
    sort_and_deduplicate(&mut findings);
    Ok(FindingBaseline {
        version: costguard_protocol::BASELINE_SCHEMA_VERSION,
        identity_scheme: costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1.into(),
        platform: baseline.platform.clone(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        policy_digest: policy_digest.or_else(|| baseline.policy_digest.clone()),
        findings,
    })
}

pub fn load_finding_baseline(path: &Path) -> Result<FindingBaseline> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read baseline {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid baseline JSON {}", path.display()))
}

pub fn load_legacy_baseline_v1(path: &Path) -> Result<LegacyFindingBaselineV1> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read legacy baseline {}", path.display()))?;
    let baseline: LegacyFindingBaselineV1 = serde_json::from_str(&text)
        .with_context(|| format!("invalid legacy baseline JSON {}", path.display()))?;
    if baseline.version != 1 {
        anyhow::bail!("legacy baseline must have version 1");
    }
    Ok(baseline)
}

pub fn migrate_legacy_baseline_v1(
    legacy: &LegacyFindingBaselineV1,
    diagnostics: &[Diagnostic],
    platform: Platform,
    policy_digest: Option<String>,
) -> Result<FindingBaseline> {
    if let Some(expected) = &legacy.warehouse {
        let expected = expected.parse::<Platform>().map_err(anyhow::Error::msg)?;
        if expected != platform {
            anyhow::bail!("legacy baseline platform '{expected}' does not match '{platform}'");
        }
    }
    let mut selected = Vec::new();
    for old in &legacy.findings {
        let candidates = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.rule_id.eq_ignore_ascii_case(&old.rule_id)
                    && normalize_path(&diagnostic.path) == normalize_string_path(&old.path)
            })
            .collect::<Vec<_>>();
        let matched = old
            .message
            .as_deref()
            .and_then(|message| {
                candidates
                    .iter()
                    .find(|diagnostic| {
                        normalize_message(&diagnostic.message) == normalize_message(message)
                    })
                    .copied()
            })
            .or_else(|| (candidates.len() == 1).then(|| candidates[0]));
        if let Some(diagnostic) = matched {
            selected.push(diagnostic.clone());
        }
    }
    Ok(FindingBaseline::from_diagnostics(
        &selected,
        platform,
        policy_digest,
    ))
}

pub fn validate_finding_baseline(baseline: &FindingBaseline, platform: Platform) -> Result<()> {
    if baseline.version == 2 {
        anyhow::bail!(
            "baseline schema version 2 is no longer supported; run: costguard baseline migrate-v2 INPUT MAP OUTPUT"
        );
    }
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

fn identity_map_by_finding_id(map: &IdentityMap) -> Result<BTreeMap<String, IdentityMapEntry>> {
    validate_identity_map_entries(&map.entries)?;
    Ok(map
        .entries
        .iter()
        .map(|entry| (entry.old_finding_id.clone(), entry.clone()))
        .collect())
}

fn validate_identity_map_entries(entries: &[IdentityMapEntry]) -> Result<()> {
    let mut by_finding_id = BTreeMap::<&str, &IdentityMapEntry>::new();
    for entry in entries {
        if let Some(existing) = by_finding_id.get(entry.old_finding_id.as_str()) {
            if existing.new_finding_id != entry.new_finding_id
                || existing.new_evidence_key != entry.new_evidence_key
            {
                anyhow::bail!(
                    "conflicting identity map entries for finding_id '{}'",
                    entry.old_finding_id
                );
            }
        } else {
            by_finding_id.insert(entry.old_finding_id.as_str(), entry);
        }
    }
    Ok(())
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
    normalize_string_path(&path.to_string_lossy())
}

fn normalize_string_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn normalize_message(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
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
        baseline.version = 3;
        assert!(validate_finding_baseline(&baseline, Platform::BigQuery).is_err());
    }

    #[test]
    fn migrate_baseline_v2_remaps_finding_ids() {
        let baseline = FindingBaselineV2 {
            version: 2,
            platform: "generic".into(),
            generated_at: "2026-01-01T00:00:00Z".into(),
            policy_digest: None,
            findings: vec![BaselinedFinding {
                finding_id: "cgf_old".into(),
                rule_id: "SQLCOST001".into(),
                path: "models/a.sql".into(),
                evidence_key: "occurrence:0".into(),
            }],
        };
        let map = IdentityMap {
            schema_version: 1,
            tool_version: "test".into(),
            platform: "generic".into(),
            source_policy_digest: None,
            source_identity_scheme: costguard_protocol::IDENTITY_SCHEME_ORDINAL_V1.into(),
            target_identity_scheme: costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1.into(),
            entries: vec![IdentityMapEntry {
                rule_id: "SQLCOST001".into(),
                path: "models/a.sql".into(),
                old_evidence_key: "occurrence:0".into(),
                new_evidence_key: "join:users.id".into(),
                old_finding_id: "cgf_old".into(),
                new_finding_id: "cgf_new".into(),
            }],
        };
        let migrated = migrate_baseline_v2(&baseline, &map, None).expect("migrate");
        assert_eq!(
            migrated.version,
            costguard_protocol::BASELINE_SCHEMA_VERSION
        );
        assert_eq!(migrated.findings[0].finding_id, "cgf_new");
        assert_eq!(migrated.findings[0].evidence_key, "join:users.id");
    }
}
