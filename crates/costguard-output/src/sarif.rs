use anyhow::Result;
use costguard_core::ScanResult;
use costguard_diagnostics::{posix_path, Severity, SourceProvenance};

pub(crate) fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low | Severity::Info => "note",
    }
}

pub(crate) fn render_sarif(result: &ScanResult) -> Result<String> {
    let rules = costguard_core::rules();
    let payload = serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "costguard",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/hypertrial/costguard",
                    "rules": sarif_rule_definitions(&rules)
                }
            },
            "results": sarif_results(result),
            "properties": {
                "costguard": {
                    "run": result.run,
                    "policy": result.policy,
                    "analysis": result.analysis,
                    "metrics": result.metrics,
                    "pr_summary": result.pr_summary,
                    "context": result.context,
                    "identity_scheme": result.identity_scheme
                }
            }
        }]
    });
    Ok(serde_json::to_string_pretty(&payload)?)
}

pub(crate) fn sarif_results(result: &ScanResult) -> Vec<serde_json::Value> {
    result
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let mut result = serde_json::json!({
                "ruleId": diagnostic.rule_id,
                "level": sarif_level(diagnostic.severity),
                "message": {
                    "text": diagnostic.message
                },
                "properties": {
                    "findingId": diagnostic.governance.finding_id,
                    "evidenceKey": diagnostic.governance.evidence_key,
                    "confidence": diagnostic.confidence,
                    "enforcementOutcome": diagnostic.governance.enforcement,
                    "policyProvenance": diagnostic.governance.policy,
                    "appliedException": diagnostic.governance.exception
                    ,"owners": diagnostic.governance.owners
                }
            });
            if diagnostic.source_provenance != Some(SourceProvenance::CompiledUnmapped) {
                result["locations"] = serde_json::json!([{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": posix_path(&diagnostic.path)
                        },
                        "region": {
                            "startLine": diagnostic.line,
                            "startColumn": diagnostic.column
                        }
                    }
                }]);
            } else {
                result["properties"]["sourcePath"] =
                    serde_json::json!(posix_path(&diagnostic.path));
                result["properties"]["compiledLine"] = serde_json::json!(diagnostic.compiled_line);
                result["properties"]["compiledColumn"] =
                    serde_json::json!(diagnostic.compiled_column);
            }
            if let Some(cost) = &diagnostic.cost_estimate {
                result["properties"]["costEstimate"] = serde_json::json!(cost);
            }
            result
        })
        .collect()
}

pub(crate) fn sarif_rule_definitions(
    rules: &[costguard_core::RuleMetadata],
) -> Vec<serde_json::Value> {
    rules
        .iter()
        .map(|rule| {
            serde_json::json!({
                "id": rule.id,
                "name": rule.name,
                "shortDescription": {
                    "text": rule.description
                },
                "defaultConfiguration": {
                    "level": sarif_level(rule.severity)
                }
            })
        })
        .collect()
}
