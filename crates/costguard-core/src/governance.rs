use crate::config::ScanConfig;
use crate::pipeline::apply_policy_governance;
use crate::PolicyMetadata;
use anyhow::{Context, Result};
use costguard_diagnostics::Diagnostic;
use costguard_policy::{
    read_signed_policy, read_trust_store, resolve_policy, verify_policy, PolicyDocumentV2,
    PolicyViolation, ResolutionContext, ResolvedPolicy,
};
use std::path::Path;

pub(crate) struct ManagedPolicy {
    pub(crate) document: PolicyDocumentV2,
    pub(crate) digest: String,
    organization: String,
    team: Option<String>,
    repository: String,
}

impl ManagedPolicy {
    pub(crate) fn resolve(&self, path: Option<&str>) -> Result<ResolvedPolicy> {
        resolve_policy(
            &self.document,
            &ResolutionContext {
                organization: &self.organization,
                team: self.team.as_deref(),
                repository: &self.repository,
                path,
            },
        )
    }

    pub(crate) fn metadata(&self) -> PolicyMetadata {
        PolicyMetadata {
            digest: self.digest.clone(),
            version: self.document.version.clone(),
            scope: "resolved-per-path".into(),
        }
    }
}

pub(crate) fn load_managed_policy(
    config: &ScanConfig,
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<ManagedPolicy>> {
    let Some(bundle_path) = &config.signed_policy.bundle_path else {
        return Ok(None);
    };
    let trust_path = config
        .signed_policy
        .trust_store_path
        .as_ref()
        .context("policy trust store is required")?;
    let resolve_path = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let signed = read_signed_policy(&resolve_path(bundle_path))?;
    let trust = read_trust_store(&resolve_path(trust_path))?;
    let document = verify_policy(&signed, &trust, now)?;
    let organization = config
        .signed_policy
        .organization
        .clone()
        .unwrap_or_else(|| document.organization.clone());
    let repository = config
        .signed_policy
        .repository
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .context("unable to determine policy repository")?;
    let resolved = resolve_policy(
        &document,
        &ResolutionContext {
            organization: &organization,
            team: config.signed_policy.team.as_deref(),
            repository: &repository,
            path: None,
        },
    )?;
    Ok(Some(ManagedPolicy {
        document,
        digest: resolved.digest,
        organization,
        team: config.signed_policy.team.clone(),
        repository,
    }))
}

pub(crate) fn validate_local_policy_controls(
    config: &ScanConfig,
    policy: Option<&ManagedPolicy>,
) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    let permissions = &policy.document.permissions;
    if (config.baseline_path.is_some() || config.write_baseline_path.is_some())
        && !permissions.allow_repository_baselines
    {
        anyhow::bail!("organization policy forbids repository baselines");
    }
    if !config.rule_overrides.is_empty()
        && !permissions.allow_local_severity_overrides
        && !permissions.allow_cli_overrides
    {
        anyhow::bail!("organization policy forbids local rule overrides");
    }
    if !config.waivers.is_empty() && !permissions.allow_local_waivers {
        anyhow::bail!("organization policy forbids local waivers");
    }
    Ok(())
}

pub(crate) fn apply_managed_governance(
    diagnostics: &mut [Diagnostic],
    managed: Option<&ManagedPolicy>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<PolicyViolation>> {
    let Some(managed) = managed else {
        return Ok(Vec::new());
    };
    apply_policy_governance(
        diagnostics,
        &|path| managed.resolve(Some(path)),
        &managed.repository,
        now,
    )
}

pub(crate) fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
