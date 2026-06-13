CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TYPE organization_role AS ENUM ('owner', 'admin', 'policy_admin', 'analyst', 'viewer');

CREATE TABLE organizations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,62}$'),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE principals (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email text NOT NULL UNIQUE,
    display_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    principal_id uuid NOT NULL REFERENCES principals(id) ON DELETE CASCADE,
    role organization_role NOT NULL,
    PRIMARY KEY (organization_id, principal_id)
);

CREATE TABLE teams (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    slug text NOT NULL,
    name text NOT NULL,
    UNIQUE (organization_id, slug)
);

CREATE TABLE repositories (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    team_id uuid REFERENCES teams(id) ON DELETE SET NULL,
    external_id bigint,
    full_name text NOT NULL,
    default_branch text NOT NULL DEFAULT 'main',
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (organization_id, full_name)
);

CREATE TABLE github_installations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    installation_id bigint NOT NULL UNIQUE,
    account_login text NOT NULL,
    api_url text NOT NULL,
    web_url text NOT NULL,
    suspended_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE policy_versions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    policy_id text NOT NULL,
    version text NOT NULL,
    digest text NOT NULL,
    signed_bundle jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (organization_id, digest)
);

CREATE TABLE organization_trust_stores (
    organization_id uuid PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    trust_store jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE policy_assignments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    policy_version_id uuid NOT NULL REFERENCES policy_versions(id) ON DELETE RESTRICT,
    team_slug text,
    active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX policy_assignments_one_active
    ON policy_assignments(repository_id) WHERE active;

CREATE TABLE exceptions (
    id text NOT NULL,
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    finding_id text,
    rule_id text,
    path_glob text NOT NULL,
    owner text NOT NULL,
    reason text NOT NULL,
    ticket_url text NOT NULL,
    approver text NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (organization_id, id),
    CHECK (finding_id IS NOT NULL OR rule_id IS NOT NULL),
    CHECK (expires_at > created_at)
);

CREATE TABLE service_tokens (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name text NOT NULL,
    prefix text NOT NULL UNIQUE,
    token_hash text NOT NULL,
    scopes text[] NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

CREATE TABLE ui_sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    principal_id uuid REFERENCES principals(id) ON DELETE CASCADE,
    session_hash text NOT NULL UNIQUE,
    csrf_token text NOT NULL,
    role organization_role NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE scan_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    run_key text NOT NULL,
    external_run_id text NOT NULL,
    commit_sha text NOT NULL,
    pull_request bigint,
    policy_digest text NOT NULL,
    attempt integer NOT NULL CHECK (attempt > 0),
    status text NOT NULL,
    analysis jsonb NOT NULL,
    metrics jsonb NOT NULL,
    cost jsonb,
    files jsonb NOT NULL,
    pr_summary jsonb,
    started_at timestamptz NOT NULL,
    completed_at timestamptz NOT NULL,
    tool_version text NOT NULL,
    superseded_by uuid REFERENCES scan_runs(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, policy_digest, attempt)
);

CREATE TABLE findings (
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    finding_id text NOT NULL,
    rule_id text NOT NULL,
    first_seen_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    resolved_at timestamptz,
    recurrence_count integer NOT NULL DEFAULT 0,
    latest_scan_run_id uuid NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
    latest_payload jsonb NOT NULL,
    PRIMARY KEY (organization_id, repository_id, finding_id)
);

CREATE TABLE cost_observations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    model_id text NOT NULL,
    window_start timestamptz NOT NULL,
    window_end timestamptz NOT NULL,
    executions bigint NOT NULL CHECK (executions >= 0),
    bytes_processed double precision,
    compute_seconds double precision,
    credits double precision,
    cost_usd double precision,
    currency text NOT NULL,
    provenance text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, model_id, window_start, window_end, provenance),
    CHECK (window_end > window_start)
);

CREATE TABLE check_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    head_sha text NOT NULL,
    policy_digest text NOT NULL,
    github_check_run_id bigint,
    status text NOT NULL,
    conclusion text,
    details_url text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, head_sha, policy_digest)
);

CREATE TABLE webhook_deliveries (
    delivery_id text PRIMARY KEY,
    event_type text NOT NULL,
    payload_hash text NOT NULL,
    status text NOT NULL,
    error text,
    received_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz
);

CREATE TABLE audit_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    actor text NOT NULL,
    action text NOT NULL,
    target_type text NOT NULL,
    target_id text NOT NULL,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    correlation_id text NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    previous_hash text,
    event_hash text NOT NULL
);

CREATE OR REPLACE FUNCTION reject_audit_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit events are append-only';
END $$;
CREATE TRIGGER audit_events_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION reject_audit_mutation();

DO $$
DECLARE table_name text;
BEGIN
    FOREACH table_name IN ARRAY ARRAY[
        'teams','memberships','repositories','github_installations','policy_versions','organization_trust_stores',
        'policy_assignments','exceptions','service_tokens','ui_sessions','scan_runs','findings',
        'cost_observations','check_runs','audit_events'
    ] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', table_name);
        EXECUTE format(
            'CREATE POLICY organization_isolation ON %I USING (organization_id = nullif(current_setting(''app.organization_id'', true), '''')::uuid) WITH CHECK (organization_id = nullif(current_setting(''app.organization_id'', true), '''')::uuid)',
            table_name
        );
    END LOOP;
END $$;
