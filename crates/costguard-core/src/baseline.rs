use anyhow::{Context, Result};
use costguard_diagnostics::Diagnostic;
use costguard_platform::Platform;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingBaseline {
    pub version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warehouse: Option<String>,
    pub findings: Vec<BaselinedFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselinedFinding {
    pub fingerprint: String,
    pub rule_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl FindingBaseline {
    pub fn from_diagnostics(diagnostics: &[Diagnostic], warehouse: Option<&str>) -> Self {
        let mut findings = diagnostics
            .iter()
            .map(|diagnostic| BaselinedFinding {
                fingerprint: diagnostic_fingerprint(diagnostic),
                rule_id: diagnostic.rule_id.clone(),
                path: normalize_path(&diagnostic.path),
                message: Some(normalize_message(&diagnostic.message)),
            })
            .collect::<Vec<_>>();
        findings.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.rule_id.cmp(&right.rule_id))
                .then(left.fingerprint.cmp(&right.fingerprint))
        });
        findings.dedup_by(|left, right| left.fingerprint == right.fingerprint);
        Self {
            version: 1,
            warehouse: warehouse.map(str::to_string),
            findings,
        }
    }

    pub fn fingerprint_set(&self) -> HashSet<&str> {
        self.findings
            .iter()
            .map(|finding| finding.fingerprint.as_str())
            .collect()
    }
}

pub fn diagnostic_fingerprint(diagnostic: &Diagnostic) -> String {
    let path = normalize_path(&diagnostic.path);
    let message = normalize_message(&diagnostic.message);
    stable_hash(&format!(
        "{}|{}|{}",
        diagnostic.rule_id.to_ascii_uppercase(),
        path,
        message
    ))
}

pub fn load_finding_baseline(path: &Path) -> Result<FindingBaseline> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read baseline {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid baseline JSON {}", path.display()))
}

pub fn validate_finding_baseline(baseline: &FindingBaseline, warehouse: Platform) -> Result<()> {
    if baseline.version != 1 {
        anyhow::bail!(
            "unsupported baseline schema version {}; expected 1",
            baseline.version
        );
    }
    if let Some(expected) = &baseline.warehouse {
        let expected_platform = expected
            .parse::<Platform>()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("invalid baseline warehouse '{expected}'"))?;
        if expected_platform != warehouse {
            anyhow::bail!(
                "baseline warehouse '{}' does not match scan warehouse '{}'",
                expected_platform,
                warehouse
            );
        }
    }
    Ok(())
}

pub fn write_finding_baseline(path: &Path, baseline: &FindingBaseline) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(baseline)?;
    std::fs::write(path, text)
        .with_context(|| format!("failed to write baseline {}", path.display()))
}

pub fn apply_finding_baseline(
    diagnostics: Vec<Diagnostic>,
    baseline: &FindingBaseline,
) -> (Vec<Diagnostic>, usize) {
    let known = baseline.fingerprint_set();
    let mut baselined = 0;
    let filtered = diagnostics
        .into_iter()
        .filter(|diagnostic| {
            let fingerprint = diagnostic_fingerprint(diagnostic);
            if known.contains(fingerprint.as_str()) {
                baselined += 1;
                false
            } else {
                true
            }
        })
        .collect();
    (filtered, baselined)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_message(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn stable_hash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = fnv1a_hasher::FnvHasher::default();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

mod fnv1a_hasher {
    use std::hash::{Hash, Hasher};

    #[derive(Default)]
    pub struct FnvHasher {
        state: u64,
    }

    impl Hasher for FnvHasher {
        fn finish(&self) -> u64 {
            self.state
        }

        fn write(&mut self, bytes: &[u8]) {
            const PRIME: u64 = 0x100000001b3;
            let mut hash = self.state;
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
            self.state = hash;
        }

        fn write_u64(&mut self, value: u64) {
            value.hash(self);
        }
    }

    impl FnvHasher {
        pub fn default() -> Self {
            Self {
                state: 0xcbf29ce484222325,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, Severity};
    use std::path::PathBuf;

    fn sample_diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
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
        }
    }

    #[test]
    fn fingerprint_is_stable_for_whitespace_variants() {
        let left = diagnostic_fingerprint(&sample_diagnostic("SELECT *  in   a model."));
        let right = diagnostic_fingerprint(&sample_diagnostic("select *\nin a model."));
        assert_eq!(left, right);
    }

    #[test]
    fn baseline_filters_known_findings() {
        let diagnostics = vec![sample_diagnostic("known"), sample_diagnostic("new finding")];
        let baseline = FindingBaseline::from_diagnostics(&[diagnostics[0].clone()], None);
        let (filtered, baselined) = apply_finding_baseline(diagnostics, &baseline);
        assert_eq!(baselined, 1);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "new finding");
    }

    #[test]
    fn baseline_rejects_version_and_warehouse_mismatch() {
        let mut baseline = FindingBaseline::from_diagnostics(&[], Some("snowflake"));
        baseline.version = 2;
        assert!(validate_finding_baseline(&baseline, Platform::Snowflake).is_err());
        baseline.version = 1;
        assert!(validate_finding_baseline(&baseline, Platform::BigQuery).is_err());
    }
}
