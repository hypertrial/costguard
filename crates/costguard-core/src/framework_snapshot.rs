use crate::rocky::RockyProject;
use crate::{LoadedProject, Project, ProjectConfigurationError};
use anyhow::{Context, Result};
use costguard_dbt::DbtProject;
use costguard_scanner::{FileKind, ProjectFile};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub(crate) struct SnapshotFiles {
    pub context: Vec<ProjectFile>,
    pub targets: Vec<ProjectFile>,
}

pub(crate) struct FrameworkSnapshotInput<'a> {
    pub root: &'a Path,
    pub dbt: Option<DbtProject>,
    pub rocky: Option<RockyProject>,
    pub rocky_owned_paths: BTreeSet<PathBuf>,
    pub raw_rocky: BTreeMap<PathBuf, String>,
    pub files: SnapshotFiles,
    pub changed_paths: &'a HashSet<PathBuf>,
    pub pr_mode: bool,
    pub selected_rocky_paths: BTreeSet<PathBuf>,
}

pub(crate) struct FrameworkSnapshot {
    pub project: LoadedProject,
    pub analysis_files: Vec<ProjectFile>,
}

pub(crate) fn assemble_framework_snapshot(
    input: FrameworkSnapshotInput<'_>,
) -> Result<FrameworkSnapshot> {
    let FrameworkSnapshotInput {
        root,
        mut dbt,
        rocky,
        rocky_owned_paths,
        raw_rocky,
        mut files,
        changed_paths,
        pr_mode,
        selected_rocky_paths,
    } = input;

    if rocky.is_none() {
        if let Some(project) = &mut dbt {
            project.models.retain(|_, model| {
                model
                    .path
                    .as_ref()
                    .is_none_or(|path| !rocky_owned_paths.contains(path))
            });
        }
    }
    if dbt
        .as_ref()
        .is_some_and(|project| project.models.is_empty())
    {
        dbt = None;
    }

    for file in files.context.iter_mut().chain(files.targets.iter_mut()) {
        if rocky_owned_paths.contains(&file.root_relative_path) {
            file.kind = FileKind::Sql;
        }
    }

    let mut graph = dbt
        .as_ref()
        .map(DbtProject::to_project_graph)
        .transpose()
        .map_err(project_configuration_error)?
        .unwrap_or_default();
    let mut compiled_unmapped_paths = BTreeSet::new();
    let mut rocky_inputs = BTreeSet::new();

    if let Some(rocky) = rocky {
        files
            .targets
            .retain(|file| !rocky_owned_paths.contains(&file.root_relative_path));
        let direct_inputs = rocky
            .source_by_name
            .values()
            .flat_map(|path| [path.clone(), path.with_extension("toml")])
            .collect::<BTreeSet<_>>();
        let global_change = changed_paths
            .iter()
            .any(|path| rocky.sealed_inputs.contains(path) && !direct_inputs.contains(path));

        for (relative, compiled) in &rocky.compiled_by_path {
            let raw = raw_rocky
                .get(relative)
                .with_context(|| format!("Rocky source {} is missing", relative.display()))?;
            let compiled_unmapped = relative.extension().and_then(|value| value.to_str())
                == Some("rocky")
                || raw != compiled;
            let text = if compiled_unmapped {
                compiled.clone()
            } else {
                raw.clone()
            };
            let file = ProjectFile {
                kind: FileKind::Sql,
                path: root.join(relative),
                root_relative_path: relative.clone(),
                line_index: costguard_diagnostics::LineIndex::new(&text),
                text,
            };
            files
                .context
                .retain(|existing| existing.root_relative_path != *relative);
            files.context.push(file.clone());

            let selected = if pr_mode {
                global_change
                    || changed_paths.contains(relative)
                    || changed_paths.contains(&relative.with_extension("toml"))
            } else {
                selected_rocky_paths.contains(relative)
            };
            if selected {
                files
                    .targets
                    .retain(|existing| existing.root_relative_path != *relative);
                files.targets.push(file);
                if compiled_unmapped {
                    compiled_unmapped_paths.insert(relative.clone());
                }
            }
        }
        graph
            .merge(rocky.graph)
            .map_err(project_configuration_error)?;
        rocky_inputs = rocky.sealed_inputs;
    }

    files
        .context
        .sort_by(|left, right| left.path.cmp(&right.path));
    files
        .targets
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(FrameworkSnapshot {
        project: LoadedProject {
            project: Project {
                root: root.to_path_buf(),
                files: files.targets,
                dbt,
            },
            graph,
            compiled_unmapped_paths,
            rocky_owned_paths,
            rocky_inputs,
        },
        analysis_files: files.context,
    })
}

fn project_configuration_error(error: costguard_project::ProjectGraphError) -> anyhow::Error {
    anyhow::Error::new(ProjectConfigurationError(error.to_string()))
}
