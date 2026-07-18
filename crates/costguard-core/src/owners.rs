use crate::config::OwnersConfig;
use costguard_dbt::{DbtModel, DbtProject};
use costguard_diagnostics::Diagnostic;
use costguard_project::{ModelMetadata, ProjectGraph};
use globset::Glob;
use std::path::Path;

#[derive(Debug, Clone)]
struct CodeownersRule {
    pattern: String,
    owners: Vec<String>,
}

pub(crate) struct OwnerResolver {
    config: OwnersConfig,
    codeowners: Vec<CodeownersRule>,
}

impl OwnerResolver {
    pub(crate) fn load(root: &Path, config: &OwnersConfig) -> anyhow::Result<Self> {
        let codeowners = if config.codeowners {
            load_codeowners(root)?
        } else {
            Vec::new()
        };
        Ok(Self {
            config: config.clone(),
            codeowners,
        })
    }

    pub(crate) fn owners_for_model(&self, model: &DbtModel) -> Vec<String> {
        if !model.owners.is_empty() {
            return unique(model.owners.clone());
        }

        let mut configured = Vec::new();
        for tag in &model.tags {
            if let Some(owners) = self.config.tags.get(tag) {
                configured.extend(owners.values());
            }
        }
        if let Some(path) = &model.path {
            configured.extend(self.configured_path_owners(path));
        }
        if !configured.is_empty() {
            return unique(configured);
        }

        if let Some(path) = &model.path {
            let codeowners = self.codeowners_for_path(path);
            if !codeowners.is_empty() {
                return codeowners;
            }
        }
        if let Some(group) = &model.group {
            return vec![group.clone()];
        }
        unique(self.config.default.values())
    }

    pub(crate) fn owners_for_metadata(&self, model: &ModelMetadata) -> Vec<String> {
        if !model.owners.is_empty() {
            return unique(model.owners.clone());
        }
        let mut configured = Vec::new();
        for tag in &model.tags {
            if let Some(owners) = self.config.tags.get(tag) {
                configured.extend(owners.values());
            }
        }
        configured.extend(self.configured_path_owners(&model.path));
        if !configured.is_empty() {
            return unique(configured);
        }
        let codeowners = self.codeowners_for_path(&model.path);
        if !codeowners.is_empty() {
            return codeowners;
        }
        if let Some(group) = &model.group {
            return vec![group.clone()];
        }
        unique(self.config.default.values())
    }

    pub(crate) fn owners_for_path(&self, path: &Path, dbt: Option<&DbtProject>) -> Vec<String> {
        if let Some(model) = model_for_path(dbt, path) {
            return self.owners_for_model(model);
        }
        let configured = self.configured_path_owners(path);
        if !configured.is_empty() {
            return configured;
        }
        let codeowners = self.codeowners_for_path(path);
        if !codeowners.is_empty() {
            return codeowners;
        }
        unique(self.config.default.values())
    }

    pub(crate) fn owners_for_project_path(
        &self,
        path: &Path,
        dbt: Option<&DbtProject>,
        graph: &ProjectGraph,
    ) -> Vec<String> {
        if let Some(model) = graph.model_for_path(path) {
            return self.owners_for_metadata(model);
        }
        self.owners_for_path(path, dbt)
    }

    fn configured_path_owners(&self, path: &Path) -> Vec<String> {
        let path = posix(path);
        unique(
            self.config
                .paths
                .iter()
                .filter(|(pattern, _)| pattern_matches(pattern, &path))
                .flat_map(|(_, owners)| owners.values())
                .collect(),
        )
    }

    fn codeowners_for_path(&self, path: &Path) -> Vec<String> {
        let path = posix(path);
        self.codeowners
            .iter()
            .rev()
            .find(|rule| pattern_matches(&rule.pattern, &path))
            .map(|rule| rule.owners.clone())
            .unwrap_or_default()
    }
}

pub(crate) fn assign_diagnostic_owners(
    diagnostics: &mut [Diagnostic],
    resolver: &OwnerResolver,
    dbt: Option<&DbtProject>,
    graph: &ProjectGraph,
) {
    for diagnostic in diagnostics {
        diagnostic.governance.owners =
            resolver.owners_for_project_path(&diagnostic.path, dbt, graph);
    }
}

pub(crate) fn model_for_path<'a>(dbt: Option<&'a DbtProject>, path: &Path) -> Option<&'a DbtModel> {
    dbt?.models
        .values()
        .find(|model| model.path.as_deref() == Some(path))
}

fn load_codeowners(root: &Path) -> anyhow::Result<Vec<CodeownersRule>> {
    let Some(path) = [
        root.join(".github/CODEOWNERS"),
        root.join("CODEOWNERS"),
        root.join("docs/CODEOWNERS"),
    ]
    .into_iter()
    .find(|path| path.is_file()) else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    let mut rules = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(pattern) = fields.next() else {
            continue;
        };
        let owners = fields
            .take_while(|field| !field.starts_with('#'))
            .map(str::to_string)
            .collect::<Vec<_>>();
        if owners.is_empty() {
            continue;
        }
        let pattern = normalize_pattern(pattern);
        Glob::new(&pattern).map_err(|error| {
            anyhow::anyhow!(
                "invalid CODEOWNERS pattern '{}' in {}: {error}",
                pattern,
                path.display()
            )
        })?;
        rules.push(CodeownersRule { pattern, owners });
    }
    Ok(rules)
}

fn normalize_pattern(pattern: &str) -> String {
    let mut pattern = pattern.trim_start_matches('/').to_string();
    if pattern.ends_with('/') {
        pattern.push_str("**");
    }
    pattern
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = normalize_pattern(pattern);
    let direct = Glob::new(&pattern)
        .map(|glob| glob.compile_matcher().is_match(path))
        .unwrap_or(false);
    if direct || pattern.contains('/') {
        return direct;
    }
    Glob::new(&format!("**/{pattern}"))
        .map(|glob| glob.compile_matcher().is_match(path))
        .unwrap_or(false)
}

fn posix(path: &Path) -> String {
    costguard_diagnostics::posix_path(path)
}

fn unique(values: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OwnerValue;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn resolves_owner_precedence_and_codeowners_last_match() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".github")).unwrap();
        std::fs::write(
            root.path().join(".github/CODEOWNERS"),
            "models/** @analytics\nmodels/finance/** @finance\n",
        )
        .unwrap();
        let config = OwnersConfig {
            default: OwnerValue::One("@default".into()),
            codeowners: true,
            paths: BTreeMap::new(),
            tags: BTreeMap::from([("critical".into(), OwnerValue::One("@platform".into()))]),
        };
        let resolver = OwnerResolver::load(root.path(), &config).unwrap();
        let tagged = DbtModel {
            path: Some(PathBuf::from("models/finance/orders.sql")),
            tags: vec!["critical".into()],
            ..DbtModel::default()
        };
        assert_eq!(resolver.owners_for_model(&tagged), vec!["@platform"]);

        let codeowned = DbtModel {
            path: Some(PathBuf::from("models/finance/orders.sql")),
            ..DbtModel::default()
        };
        assert_eq!(resolver.owners_for_model(&codeowned), vec!["@finance"]);

        let dbt_owned = DbtModel {
            owners: vec!["data@example.com".into()],
            ..codeowned
        };
        assert_eq!(
            resolver.owners_for_model(&dbt_owned),
            vec!["data@example.com"]
        );
    }
}
