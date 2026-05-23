use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    #[default]
    High,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Low => "LOW",
            Self::Medium => "MED",
            Self::High => "HIGH",
            Self::Critical => "CRIT",
        }
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "info" => Ok(Self::Info),
            "low" => Ok(Self::Low),
            "medium" | "med" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" | "crit" => Ok(Self::Critical),
            other => Err(format!("unknown severity '{other}'")),
        }
    }
}

impl FromStr for Confidence {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" | "med" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            other => Err(format!("unknown confidence '{other}'")),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line: usize,
    pub column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provenance: Option<SourceProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceProvenance {
    Raw,
    Compiled,
    CompiledUnmapped,
}

impl Span {
    pub fn with_source_provenance(mut self, source: SourceProvenance) -> Self {
        self.source_provenance = Some(source);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule_id: String,
    pub severity: Severity,
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    pub confidence: Confidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warehouse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provenance: Option<SourceProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_column: Option<usize>,
}

impl Diagnostic {
    pub fn new(
        rule_id: impl Into<String>,
        severity: Severity,
        path: PathBuf,
        span: Option<Span>,
        message: impl Into<String>,
    ) -> Self {
        let source_provenance = span.and_then(|s| s.source_provenance);
        let (line, column) = if source_provenance == Some(SourceProvenance::CompiledUnmapped) {
            (1, 1)
        } else {
            span.map(|s| (s.line, s.column)).unwrap_or((1, 1))
        };
        let compiled_line = (source_provenance == Some(SourceProvenance::CompiledUnmapped))
            .then(|| span.map(|s| s.line))
            .flatten();
        let compiled_column = (source_provenance == Some(SourceProvenance::CompiledUnmapped))
            .then(|| span.map(|s| s.column))
            .flatten();
        let span = if source_provenance == Some(SourceProvenance::CompiledUnmapped) {
            None
        } else {
            span
        };
        Self {
            rule_id: rule_id.into(),
            severity,
            path,
            line,
            column,
            span,
            message: message.into(),
            risk: None,
            suggestion: None,
            confidence: Confidence::High,
            warehouse: None,
            source_provenance,
            compiled_line,
            compiled_column,
        }
    }

    pub fn with_risk(mut self, risk: impl Into<String>) -> Self {
        self.risk = Some(risk.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_warehouse(mut self, warehouse: impl Into<String>) -> Self {
        self.warehouse = Some(warehouse.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(idx + 1);
            }
        }
        Self { starts }
    }

    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let line_idx = match self.starts.binary_search(&byte) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let start = self.starts.get(line_idx).copied().unwrap_or(0);
        (line_idx + 1, byte.saturating_sub(start) + 1)
    }

    pub fn span(&self, byte_start: usize, byte_end: usize) -> Span {
        let (line, column) = self.line_col(byte_start);
        Span {
            byte_start,
            byte_end,
            line,
            column,
            source_provenance: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Suppressions {
    disable_file: HashSet<String>,
    disable_line: HashMap<usize, HashSet<String>>,
    allow_cross_join_lines: HashSet<usize>,
}

impl Suppressions {
    pub fn parse(text: &str) -> Self {
        let mut parsed = Self::default();
        for (idx, line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let lower = line.to_ascii_lowercase();
            if !lower.contains("costguard:") {
                continue;
            }
            if lower.contains("allow cross-join") {
                parsed.allow_cross_join_lines.insert(line_no);
                parsed.allow_cross_join_lines.insert(line_no + 1);
            }
            if let Some(rules) = value_after(line, "disable-file=") {
                parsed.disable_file.extend(split_rules(rules));
            }
            if let Some(rules) = value_after(line, "disable-next-line=") {
                parsed
                    .disable_line
                    .entry(line_no + 1)
                    .or_default()
                    .extend(split_rules(rules));
            }
            if let Some(rules) = value_after(line, "disable=") {
                parsed
                    .disable_line
                    .entry(line_no)
                    .or_default()
                    .extend(split_rules(rules));
            }
        }
        parsed
    }

    pub fn is_suppressed(&self, diagnostic: &Diagnostic) -> bool {
        if self.disable_file.contains(&diagnostic.rule_id) {
            return true;
        }
        if diagnostic.rule_id == "SQLCOST012"
            && (self.allow_cross_join_lines.contains(&diagnostic.line)
                || self
                    .allow_cross_join_lines
                    .contains(&diagnostic.line.saturating_sub(1)))
        {
            return true;
        }
        self.disable_line
            .get(&diagnostic.line)
            .is_some_and(|rules| rules.contains(&diagnostic.rule_id))
    }
}

pub fn apply_suppressions(text: &str, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let suppressions = Suppressions::parse(text);
    diagnostics
        .into_iter()
        .filter(|diagnostic| !suppressions.is_suppressed(diagnostic))
        .collect()
}

fn value_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let start = line.to_ascii_lowercase().find(marker)?;
    let value = &line[start + marker.len()..];
    Some(value.split_whitespace().next().unwrap_or(value))
}

fn split_rules(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(|rule| {
            rule.trim()
                .trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
        })
        .filter(|rule| !rule.is_empty())
        .map(|rule| rule.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_byte_to_line_column() {
        let index = LineIndex::new("a\nbc\n");
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(2), (2, 1));
        assert_eq!(index.line_col(3), (2, 2));
    }

    #[test]
    fn suppresses_next_line() {
        let text = "-- costguard: disable-next-line=SQLCOST001\nselect * from t";
        let span = LineIndex::new(text).span(text.find("select").unwrap(), text.len());
        let diag = Diagnostic::new(
            "SQLCOST001",
            Severity::Medium,
            PathBuf::from("x.sql"),
            Some(span),
            "bad",
        );
        assert!(apply_suppressions(text, vec![diag]).is_empty());
    }
}
