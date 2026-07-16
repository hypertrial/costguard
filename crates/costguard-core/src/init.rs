//! Scaffold Costguard into a dbt project (workflow + optional config).

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

const LOCAL_DUCKDB_PROFILE: &str = "local-duckdb";

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub warehouse: Option<String>,
    pub profile: Option<String>,
    pub dbt_dir: Option<PathBuf>,
    pub force: bool,
    pub no_workflow: bool,
    pub no_config: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitOutcome {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub warehouse: String,
}

/// ponytail: profiles.yml is often gitignored; detection is best-effort only.
pub fn detect_warehouse(root: &Path) -> String {
    let profile = read_dbt_profile_name(root);
    if let Some(profile_name) = profile {
        if let Some(adapter) = read_profiles_adapter(root, &profile_name) {
            if let Some(warehouse) = map_adapter_to_warehouse(&adapter) {
                return warehouse;
            }
        }
    }
    "generic".to_string()
}

pub fn init_project(root: &Path, opts: &InitOptions) -> Result<InitOutcome> {
    let profile = parse_profile(opts.profile.as_deref())?;
    let dbt_root = resolve_dbt_root(root, opts.dbt_dir.as_deref())?;
    let warehouse = match profile {
        Some(LOCAL_DUCKDB_PROFILE) => {
            if let Some(warehouse) = &opts.warehouse {
                if !warehouse.trim().eq_ignore_ascii_case("duckdb") {
                    anyhow::bail!("profile local-duckdb requires --warehouse duckdb");
                }
            }
            "duckdb".to_string()
        }
        _ => opts
            .warehouse
            .clone()
            .unwrap_or_else(|| detect_warehouse(&dbt_root)),
    };
    let version = env!("CARGO_PKG_VERSION");
    let mut outcome = InitOutcome {
        warehouse: warehouse.clone(),
        ..InitOutcome::default()
    };

    if !opts.no_workflow {
        let workflow_path = root.join(".github/workflows/costguard.yml");
        write_if_allowed(
            &workflow_path,
            &workflow_template(&warehouse, version, opts.dbt_dir.as_deref(), profile),
            opts.force,
            &mut outcome,
        )?;
    }

    if !opts.no_config {
        let config_path = dbt_root.join("costguard.toml");
        write_if_allowed(
            &config_path,
            &config_template(&warehouse, profile),
            opts.force,
            &mut outcome,
        )?;
    }

    Ok(outcome)
}

fn parse_profile(value: Option<&str>) -> Result<Option<&'static str>> {
    match value.map(|profile| profile.trim().to_ascii_lowercase()) {
        None => Ok(None),
        Some(profile) if profile == LOCAL_DUCKDB_PROFILE => Ok(Some(LOCAL_DUCKDB_PROFILE)),
        Some(profile) => anyhow::bail!("unknown init profile '{profile}'"),
    }
}

fn resolve_dbt_root(root: &Path, dbt_dir: Option<&Path>) -> Result<PathBuf> {
    let Some(dbt_dir) = dbt_dir else {
        return Ok(root.to_path_buf());
    };
    if dbt_dir.is_absolute() {
        anyhow::bail!("--dbt-dir must be a relative path inside the current repository");
    }
    for component in dbt_dir.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => anyhow::bail!("--dbt-dir must stay inside the current repository"),
        }
    }
    let target = root.join(dbt_dir);
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let existing_ancestor = deepest_existing_ancestor(&target);
    let canonical_ancestor = existing_ancestor
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", existing_ancestor.display()))?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        anyhow::bail!("--dbt-dir must stay inside the current repository");
    }
    Ok(target)
}

fn deepest_existing_ancestor(path: &Path) -> &Path {
    if path.exists() {
        return path;
    }
    path.ancestors()
        .skip(1)
        .find(|ancestor| ancestor.exists())
        .unwrap_or(path)
}

fn write_if_allowed(
    path: &Path,
    contents: &str,
    force: bool,
    outcome: &mut InitOutcome,
) -> Result<()> {
    if path.exists() && !force {
        outcome.skipped.push(path.to_path_buf());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    outcome.created.push(path.to_path_buf());
    Ok(())
}

fn read_dbt_profile_name(root: &Path) -> Option<String> {
    let path = root.join("dbt_project.yml");
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    value
        .get("profile")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn read_profiles_adapter(root: &Path, profile_name: &str) -> Option<String> {
    for profiles_path in [root.join("profiles.yml"), home_dbt_profiles()] {
        if let Some(adapter) = read_adapter_from_profiles(&profiles_path, profile_name) {
            return Some(adapter);
        }
    }
    None
}

fn home_dbt_profiles() -> PathBuf {
    dirs_or_home().join(".dbt/profiles.yml")
}

fn dirs_or_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_adapter_from_profiles(path: &Path, profile_name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let profile = value.get(profile_name)?;
    let outputs = profile.get("outputs")?.as_mapping()?;
    let target = profile
        .get("target")
        .and_then(|v| v.as_str())
        .or_else(|| outputs.keys().next()?.as_str())?;
    outputs
        .get(target)
        .and_then(|output| output.get("type"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub fn map_adapter_to_warehouse(adapter: &str) -> Option<String> {
    match adapter.to_ascii_lowercase().as_str() {
        "snowflake" => Some("snowflake".into()),
        "bigquery" => Some("bigquery".into()),
        "databricks" => Some("databricks".into()),
        "redshift" => Some("redshift".into()),
        "postgres" => Some("postgres".into()),
        "duckdb" => Some("duckdb".into()),
        "trino" => Some("trino".into()),
        _ => None,
    }
}

fn workflow_template(
    warehouse: &str,
    version: &str,
    dbt_dir: Option<&Path>,
    profile: Option<&str>,
) -> String {
    let version_tag = format!("v{version}");
    let dbt_dir = dbt_dir
        .filter(|path| !path.as_os_str().is_empty() && *path != Path::new("."))
        .map(posix_path);
    let dbt_working_dir = dbt_dir
        .as_ref()
        .map(|dir| format!("          working-directory: {dir}\n"))
        .unwrap_or_default();
    let manifest_input = if profile == Some(LOCAL_DUCKDB_PROFILE) {
        "          manifest: target/manifest.json\n"
    } else {
        ""
    };
    let dbt_hooks = dbt_dir
        .as_ref()
        .map(|dir| {
            format!(
                "      # - run: dbt deps\n      #   working-directory: {dir}\n      # - run: dbt compile --target dev\n      #   working-directory: {dir}\n"
            )
        })
        .unwrap_or_else(|| {
            "      # - run: dbt deps\n      # - run: dbt compile --target dev\n".to_string()
        });
    format!(
        r#"# Generated by costguard init. Commit this workflow to enable PR cost checks.
name: Costguard

on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  costguard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0

      # Uncomment and adjust when your CI already has dbt + credentials:
{dbt_hooks}

      - uses: hypertrial/costguard/.github/actions/costguard@{version_tag}
        with:
          base: origin/${{{{ github.event.pull_request.base.ref }}}}
{dbt_working_dir}{manifest_input}          warehouse: {warehouse}
          fail-on: high
          min-confidence: high
          block-only-new: true
          pr-comment: true
          github-token: ${{{{ github.token }}}}
          # fail-on-pr-cost-increase: "500" # requires priced [cost] configuration
"#
    )
}

fn posix_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn config_template(warehouse: &str, profile: Option<&str>) -> String {
    if profile == Some(LOCAL_DUCKDB_PROFILE) {
        return r#"# Generated by costguard init --profile local-duckdb. See https://github.com/hypertrial/costguard/blob/main/docs/book/reference/configuration.md
warehouse = "duckdb"

[scan]
paths = ["models"]
ignore = ["target", "dbt_packages"]

[output]
fail_on = "high"
min_confidence = "high"
min_confidence_filter = true

[gate]
block_only_new = true
# fail_on_pr_cost_increase = 500
# Requires enabled [cost] and [cost.pricing].model = "scan" or "compute".

[dbt]
manifest_path = "target/manifest.json"
max_manifest_bytes = 536870912

# [analysis]
# policy = "strict"

# [cost]
# enabled = true
#
# [cost.inputs]
# observations = "costguard-observations.json"
"#
        .to_string();
    }
    format!(
        r#"# Generated by costguard init. See https://github.com/hypertrial/costguard/blob/main/docs/book/reference/configuration.md
warehouse = "{warehouse}"

# [scan]
# paths = ["models"]
# ignore = ["target", "dbt_packages"]

# [output]
# fail_on = "high"
# min_confidence = "high"
# min_confidence_filter = true

[gate]
block_only_new = true
# fail_on_pr_cost_increase = 500
# Requires enabled [cost] and [cost.pricing].model = "scan" or "compute".

# [dbt]
# manifest_path = "target/manifest.json"
# max_manifest_bytes = 536870912
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn maps_known_adapters() {
        assert_eq!(
            map_adapter_to_warehouse("snowflake").as_deref(),
            Some("snowflake")
        );
        assert_eq!(
            map_adapter_to_warehouse("BigQuery").as_deref(),
            Some("bigquery")
        );
        assert_eq!(map_adapter_to_warehouse("unknown"), None);
    }

    #[test]
    fn init_writes_and_skips_without_force() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("dbt_project.yml"),
            "name: demo\nprofile: dev\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("profiles.yml"),
            r#"
dev:
  target: prod
  outputs:
    prod:
      type: snowflake
"#,
        )
        .unwrap();

        let first = init_project(
            temp.path(),
            &InitOptions {
                force: false,
                ..InitOptions::default()
            },
        )
        .unwrap();
        assert_eq!(first.warehouse, "snowflake");
        assert_eq!(first.created.len(), 2);
        assert!(first.skipped.is_empty());

        let second = init_project(
            temp.path(),
            &InitOptions {
                force: false,
                ..InitOptions::default()
            },
        )
        .unwrap();
        assert!(second.created.is_empty());
        assert_eq!(second.skipped.len(), 2);
    }

    #[test]
    fn init_respects_warehouse_override() {
        let temp = tempdir().unwrap();
        let outcome = init_project(
            temp.path(),
            &InitOptions {
                warehouse: Some("trino".into()),
                ..InitOptions::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.warehouse, "trino");
        let workflow =
            fs::read_to_string(temp.path().join(".github/workflows/costguard.yml")).unwrap();
        assert!(workflow.contains("warehouse: trino"));
        assert!(workflow.contains("pull-requests: write"));
        assert!(workflow.contains("pr-comment: true"));
        assert!(workflow.contains("github-token: ${{ github.token }}"));
        assert!(workflow.contains(&format!("@v{}", env!("CARGO_PKG_VERSION"))));
        assert!(!workflow.contains("@vv"));
        assert!(!workflow.contains("manifest: target/manifest.json"));
    }

    #[test]
    fn local_duckdb_profile_writes_config_under_dbt_dir() {
        let temp = tempdir().unwrap();
        let outcome = init_project(
            temp.path(),
            &InitOptions {
                profile: Some(LOCAL_DUCKDB_PROFILE.into()),
                dbt_dir: Some(PathBuf::from("dbt")),
                ..InitOptions::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.warehouse, "duckdb");

        let workflow =
            fs::read_to_string(temp.path().join(".github/workflows/costguard.yml")).unwrap();
        assert!(workflow.contains("working-directory: dbt"));
        assert!(workflow.contains("manifest: target/manifest.json"));
        assert!(workflow.contains("warehouse: duckdb"));

        let config = fs::read_to_string(temp.path().join("dbt/costguard.toml")).unwrap();
        assert!(config.contains("warehouse = \"duckdb\""));
        assert!(config.contains("[scan]\npaths = [\"models\"]"));
        assert!(config.contains("[dbt]\nmanifest_path = \"target/manifest.json\""));
        assert!(config.contains("# policy = \"strict\""));
        assert!(config.contains("# observations = \"costguard-observations.json\""));
    }

    #[test]
    fn local_duckdb_profile_rejects_incompatible_warehouse() {
        let temp = tempdir().unwrap();
        let error = init_project(
            temp.path(),
            &InitOptions {
                profile: Some(LOCAL_DUCKDB_PROFILE.into()),
                warehouse: Some("trino".into()),
                ..InitOptions::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("requires --warehouse duckdb"), "{error}");
    }

    #[test]
    fn init_rejects_dbt_dir_escape() {
        let temp = tempdir().unwrap();
        let error = init_project(
            temp.path(),
            &InitOptions {
                dbt_dir: Some(PathBuf::from("../dbt")),
                ..InitOptions::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("inside the current repository"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn init_rejects_dbt_dir_symlink_escape() {
        let temp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("outside")).unwrap();

        let error = init_project(
            temp.path(),
            &InitOptions {
                dbt_dir: Some(PathBuf::from("outside/dbt")),
                ..InitOptions::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("inside the current repository"), "{error}");
    }
}
