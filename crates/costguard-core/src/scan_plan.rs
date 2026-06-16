use costguard_scanner::{DiscoveryOptions, ProjectFile, SkippedFile};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Internal scan plan separating PR targets from full-project context.
#[derive(Debug, Clone)]
pub struct ScanPlan {
    pub targets: Vec<ProjectFile>,
    pub context: Vec<ProjectFile>,
    pub target_skips: Vec<SkippedFile>,
    pub context_skips: Vec<SkippedFile>,
    pub changed_paths: HashSet<PathBuf>,
    pub pr_mode: bool,
}

impl ScanPlan {
    pub fn union_files(&self) -> Vec<ProjectFile> {
        let mut files = self.context.clone();
        let context_paths: HashSet<_> = files.iter().map(|f| f.path.clone()).collect();
        for file in &self.targets {
            if !context_paths.contains(&file.path) {
                files.push(file.clone());
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        files
    }

    pub fn is_target(&self, path: &Path) -> bool {
        if !self.pr_mode {
            return true;
        }
        self.targets.iter().any(|file| file.path == path)
    }
}

pub(crate) fn build_scan_plan(
    root: &Path,
    config: &crate::config::ScanConfig,
    discovery_options: &DiscoveryOptions,
) -> anyhow::Result<ScanPlan> {
    if config.changed_only {
        if !crate::git::is_git_repository(root) {
            anyhow::bail!("{} is not a git repository", root.display());
        }
        let base = config.base_branch.as_deref().unwrap_or("main");
        let changed_files = crate::git::changed_files(root, base)
            .with_context(|| format!("failed to resolve changed files against base '{base}'"))?;
        let changed_discovery = costguard_scanner::read_existing_paths_with_options(
            root,
            &changed_files,
            discovery_options,
        )?;
        let context_discovery =
            costguard_scanner::discover_with_options(root, &config.paths, discovery_options)?;
        let context_files = context_discovery
            .files
            .into_iter()
            .filter(is_context_file)
            .collect::<Vec<_>>();
        Ok(ScanPlan {
            targets: changed_discovery.files,
            context: context_files,
            target_skips: changed_discovery.skipped_files,
            context_skips: context_discovery.skipped_files,
            changed_paths: changed_files.into_iter().collect(),
            pr_mode: true,
        })
    } else {
        let discovery =
            costguard_scanner::discover_with_options(root, &config.paths, discovery_options)?;
        Ok(ScanPlan {
            targets: discovery.files.clone(),
            context: discovery.files,
            target_skips: discovery.skipped_files.clone(),
            context_skips: discovery.skipped_files,
            changed_paths: HashSet::new(),
            pr_mode: false,
        })
    }
}

fn is_context_file(file: &ProjectFile) -> bool {
    matches!(
        file.kind,
        costguard_scanner::FileKind::Sql
            | costguard_scanner::FileKind::DbtSqlModel
            | costguard_scanner::FileKind::DbtYaml
            | costguard_scanner::FileKind::ManifestJson
    )
}

use anyhow::Context;
