#![forbid(unsafe_code)]

use anyhow::Context;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use askama::Template;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Form, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use chrono::{Duration, Utc};
use costguard_policy::{policy_digest, verify_policy, TrustStoreV1};
use costguard_protocol::{
    CostObservationBundleV1, EnforcementOutcome, ScanEnvelopeV1, SignedDocumentV1,
};
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Row, Transaction};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::Mutex;
use tower_http::{
    catch_panic::CatchPanicLayer, limit::RequestBodyLimitLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    config: Arc<Config>,
    client: reqwest::Client,
    metrics: Arc<Metrics>,
    installation_tokens: Arc<Mutex<HashMap<i64, CachedInstallationToken>>>,
}

struct Config {
    listen: String,
    database_url: String,
    public_url: String,
    bootstrap_secret: String,
    github_webhook_secret: String,
    github_app_id: Option<String>,
    github_private_key: Option<String>,
    github_api_url: String,
    _github_web_url: String,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            listen: env::var("COSTGUARD_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            database_url: required_env("DATABASE_URL")?,
            public_url: env::var("COSTGUARD_PUBLIC_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".into())
                .trim_end_matches('/')
                .to_string(),
            bootstrap_secret: required_env("COSTGUARD_BOOTSTRAP_SECRET")?,
            github_webhook_secret: env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            github_app_id: env::var("GITHUB_APP_ID").ok(),
            github_private_key: env::var("GITHUB_APP_PRIVATE_KEY").ok(),
            github_api_url: env::var("GITHUB_API_URL")
                .unwrap_or_else(|_| "https://api.github.com".into())
                .trim_end_matches('/')
                .to_string(),
            _github_web_url: env::var("GITHUB_WEB_URL")
                .unwrap_or_else(|_| "https://github.com".into())
                .trim_end_matches('/')
                .to_string(),
        })
    }
}

struct Metrics {
    registry: Registry,
    _requests: IntCounterVec,
    ingested_scans: IntCounterVec,
    webhook_deliveries: IntCounterVec,
    ready: IntGauge,
}

impl Metrics {
    fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("costguard_http_requests_total", "HTTP requests"),
            &["route", "status"],
        )?;
        let ingested_scans = IntCounterVec::new(
            Opts::new("costguard_scan_ingestions_total", "Scan ingestions"),
            &["status"],
        )?;
        let webhook_deliveries = IntCounterVec::new(
            Opts::new(
                "costguard_webhook_deliveries_total",
                "GitHub webhook deliveries",
            ),
            &["status"],
        )?;
        let ready = IntGauge::new("costguard_ready", "Database readiness")?;
        registry.register(Box::new(requests.clone()))?;
        registry.register(Box::new(ingested_scans.clone()))?;
        registry.register(Box::new(webhook_deliveries.clone()))?;
        registry.register(Box::new(ready.clone()))?;
        Ok(Self {
            registry,
            _requests: requests,
            ingested_scans,
            webhook_deliveries,
            ready,
        })
    }
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        error!(error = %error, "request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
struct AuthContext {
    organization_id: Uuid,
    organization_slug: String,
    actor: String,
}

#[derive(Deserialize)]
struct BootstrapRequest {
    organization_slug: String,
    organization_name: String,
    owner_email: String,
    owner_name: String,
    token_name: Option<String>,
}

#[derive(Serialize)]
struct BootstrapResponse {
    organization_id: Uuid,
    token: String,
}

#[derive(Deserialize)]
struct CreateRepositoryRequest {
    organization: String,
    full_name: String,
    external_id: Option<i64>,
    default_branch: Option<String>,
}

#[derive(Deserialize)]
struct UploadPolicyRequest {
    organization: String,
    bundle: SignedDocumentV1,
    trust_store: TrustStoreV1,
}

#[derive(Deserialize)]
struct AssignPolicyRequest {
    organization: String,
    repository: String,
    policy_digest: String,
    team: Option<String>,
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    organization: String,
    name: String,
    scopes: Vec<String>,
    expires_at: String,
}

#[derive(Deserialize)]
struct CreateExceptionRequest {
    organization: String,
    repository: String,
    id: String,
    finding_id: Option<String>,
    rule_id: Option<String>,
    path: String,
    owner: String,
    reason: String,
    ticket_url: String,
    approver: String,
    created_at: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct RegisterInstallationRequest {
    organization: String,
    installation_id: i64,
    account_login: String,
    api_url: Option<String>,
    web_url: Option<String>,
}

#[derive(Deserialize)]
struct PolicyQuery {
    organization: String,
    repository: String,
}

#[derive(Deserialize)]
struct ReportQuery {
    organization: String,
    format: Option<String>,
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    page: String,
    organization: String,
    rows: Vec<(String, String)>,
}

#[derive(Clone)]
struct CachedInstallationToken {
    token: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Serialize)]
struct GitHubJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    let config = Arc::new(Config::from_env()?);
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&config.database_url)
        .await
        .context("connect PostgreSQL")?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("run migrations")?;
    let metrics = Arc::new(Metrics::new()?);
    metrics.ready.set(1);
    let state = AppState {
        pool,
        config: config.clone(),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("costguard-server/2")
            .build()?,
        metrics,
        installation_tokens: Arc::new(Mutex::new(HashMap::new())),
    };
    spawn_check_timeout_job(state.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    info!(listen = %config.listen, "costguard server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/bootstrap", post(bootstrap))
        .route("/repositories", post(create_repository))
        .route("/policies", post(upload_policy))
        .route("/policy-assignments", post(assign_policy))
        .route("/tokens", post(create_token))
        .route("/tokens/{id}/revoke", post(revoke_token))
        .route("/exceptions", post(create_exception))
        .route("/github/installations", post(register_installation))
        .route("/policies/resolved", get(resolved_policy))
        .route("/scan-runs", post(ingest_scan))
        .route("/cost-observations", post(ingest_cost))
        .route("/reports/summary", get(report_summary));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/openapi.json", get(openapi))
        .route("/login", get(login_page).post(login))
        .route("/webhooks/github", post(github_webhook))
        .route("/ui/{page}", get(ui_page))
        .nest("/api/v1", api)
        .with_state(state)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
            header::AUTHORIZATION,
        )))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'; base-uri 'none'"),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => {
            state.metrics.ready.set(1);
            (StatusCode::OK, "ready\n")
        }
        Err(_) => {
            state.metrics.ready.set(0);
            (StatusCode::SERVICE_UNAVAILABLE, "not ready\n")
        }
    }
}

async fn metrics(State(state): State<AppState>) -> ApiResult<Response> {
    let families = state.metrics.registry.gather();
    let mut output = Vec::new();
    TextEncoder::new()
        .encode(&families, &mut output)
        .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        output,
    )
        .into_response())
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {"title": "Costguard API", "version": env!("CARGO_PKG_VERSION")},
        "paths": {
            "/api/v1/policies/resolved": {"get": {"summary": "Fetch assigned signed policy"}},
            "/api/v1/scan-runs": {"post": {"summary": "Ingest metadata-only scan envelope"}},
            "/api/v1/cost-observations": {"post": {"summary": "Ingest normalized offline cost observations"}},
            "/webhooks/github": {"post": {"summary": "Receive verified GitHub App webhooks"}}
        }
    }))
}

async fn bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BootstrapRequest>,
) -> ApiResult<Json<BootstrapResponse>> {
    verify_bootstrap_secret(&state.config.bootstrap_secret, &headers)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM organizations")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if count != 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "bootstrap is disabled after the first organization".into(),
        ));
    }
    let organization_id: Uuid =
        sqlx::query_scalar("INSERT INTO organizations (slug, name) VALUES ($1, $2) RETURNING id")
            .bind(&input.organization_slug)
            .bind(&input.organization_name)
            .fetch_one(&mut *tx)
            .await
            .map_err(conflict)?;
    let principal_id: Uuid = sqlx::query_scalar(
        "INSERT INTO principals (email, display_name) VALUES ($1, $2) RETURNING id",
    )
    .bind(&input.owner_email)
    .bind(&input.owner_name)
    .fetch_one(&mut *tx)
    .await
    .map_err(conflict)?;
    sqlx::query(
        "INSERT INTO memberships (organization_id, principal_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(principal_id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    let (token, prefix, hash) = new_service_token().await?;
    sqlx::query("INSERT INTO service_tokens (organization_id, name, prefix, token_hash, scopes, expires_at) VALUES ($1, $2, $3, $4, $5, now() + interval '365 days')")
        .bind(organization_id).bind(input.token_name.unwrap_or_else(|| "bootstrap-owner".into())).bind(prefix).bind(hash)
        .bind(vec!["policy:read", "policy:write", "scan:write", "cost:write", "admin"])
        .execute(&mut *tx).await.map_err(internal)?;
    append_audit(
        &mut tx,
        organization_id,
        &input.owner_email,
        "organization.bootstrap",
        "organization",
        &organization_id.to_string(),
        json!({"slug": input.organization_slug}),
        correlation_id(&headers),
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(BootstrapResponse {
        organization_id,
        token,
    }))
}

async fn create_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRepositoryRequest>,
) -> ApiResult<Json<Value>> {
    let auth = authenticate(&state, &headers, "admin").await?;
    require_org(&auth, &input.organization)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO repositories (organization_id, full_name, external_id, default_branch) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(auth.organization_id).bind(&input.full_name).bind(input.external_id).bind(input.default_branch.unwrap_or_else(|| "main".into()))
        .fetch_one(&state.pool).await.map_err(conflict)?;
    audit(
        &state,
        &auth,
        "repository.create",
        "repository",
        &id.to_string(),
        json!({"full_name": input.full_name}),
        &headers,
    )
    .await?;
    Ok(Json(json!({"id": id})))
}

async fn upload_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UploadPolicyRequest>,
) -> ApiResult<Json<Value>> {
    let auth = authenticate(&state, &headers, "policy:write").await?;
    require_org(&auth, &input.organization)?;
    let policy = verify_policy(&input.bundle, &input.trust_store, Utc::now())
        .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    if policy.organization != input.organization {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "policy organization mismatch".into(),
        ));
    }
    let digest = policy_digest(&policy).map_err(internal)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    sqlx::query("INSERT INTO organization_trust_stores (organization_id, trust_store) VALUES ($1, $2) ON CONFLICT (organization_id) DO UPDATE SET trust_store = EXCLUDED.trust_store, updated_at = now()")
        .bind(auth.organization_id).bind(serde_json::to_value(&input.trust_store).map_err(internal)?).execute(&mut *tx).await.map_err(internal)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO policy_versions (organization_id, policy_id, version, digest, signed_bundle) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (organization_id, digest) DO UPDATE SET signed_bundle = EXCLUDED.signed_bundle RETURNING id")
        .bind(auth.organization_id).bind(&policy.id).bind(&policy.version).bind(&digest).bind(serde_json::to_value(&input.bundle).map_err(internal)?)
        .fetch_one(&mut *tx).await.map_err(internal)?;
    append_audit(
        &mut tx,
        auth.organization_id,
        &auth.actor,
        "policy.upload",
        "policy_version",
        &id.to_string(),
        json!({"digest": digest}),
        correlation_id(&headers),
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"id": id, "digest": digest})))
}

async fn assign_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AssignPolicyRequest>,
) -> ApiResult<Json<Value>> {
    let auth = authenticate(&state, &headers, "policy:write").await?;
    require_org(&auth, &input.organization)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT id FROM repositories WHERE organization_id=$1 AND full_name=$2")
            .bind(auth.organization_id)
            .bind(&input.repository)
            .fetch_one(&mut *tx)
            .await
            .map_err(not_found)?;
    let policy_id: Uuid =
        sqlx::query_scalar("SELECT id FROM policy_versions WHERE organization_id=$1 AND digest=$2")
            .bind(auth.organization_id)
            .bind(&input.policy_digest)
            .fetch_one(&mut *tx)
            .await
            .map_err(not_found)?;
    sqlx::query("UPDATE policy_assignments SET active=false WHERE repository_id=$1 AND active")
        .bind(repository_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO policy_assignments (organization_id, repository_id, policy_version_id, team_slug) VALUES ($1,$2,$3,$4) RETURNING id")
        .bind(auth.organization_id).bind(repository_id).bind(policy_id).bind(input.team).fetch_one(&mut *tx).await.map_err(internal)?;
    append_audit(
        &mut tx,
        auth.organization_id,
        &auth.actor,
        "policy.assign",
        "policy_assignment",
        &id.to_string(),
        json!({"repository": input.repository, "digest": input.policy_digest}),
        correlation_id(&headers),
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"id": id})))
}

async fn create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateTokenRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let auth = authenticate(&state, &headers, "admin").await?;
    require_org(&auth, &input.organization)?;
    let allowed = [
        "policy:read",
        "policy:write",
        "scan:write",
        "cost:write",
        "admin",
    ];
    if input.scopes.is_empty()
        || input
            .scopes
            .iter()
            .any(|scope| !allowed.contains(&scope.as_str()))
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "token scopes are invalid".into(),
        ));
    }
    let expires_at = parse_time(&input.expires_at)?;
    if expires_at <= Utc::now() {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "token expiry must be in the future".into(),
        ));
    }
    let (token, prefix, hash) = new_service_token().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO service_tokens (organization_id,name,prefix,token_hash,scopes,expires_at) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id")
        .bind(auth.organization_id).bind(&input.name).bind(prefix).bind(hash).bind(&input.scopes).bind(expires_at)
        .fetch_one(&state.pool).await.map_err(internal)?;
    audit(
        &state,
        &auth,
        "token.create",
        "service_token",
        &id.to_string(),
        json!({"name":input.name,"scopes":input.scopes,"expires_at":input.expires_at}),
        &headers,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"token":token}))))
}

async fn revoke_token(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let auth = authenticate(&state, &headers, "admin").await?;
    let affected = sqlx::query("UPDATE service_tokens SET revoked_at=now() WHERE id=$1 AND organization_id=$2 AND revoked_at IS NULL")
        .bind(id).bind(auth.organization_id).execute(&state.pool).await.map_err(internal)?.rows_affected();
    if affected == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "token not found".into()));
    }
    audit(
        &state,
        &auth,
        "token.revoke",
        "service_token",
        &id.to_string(),
        json!({}),
        &headers,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_exception(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateExceptionRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let auth = authenticate(&state, &headers, "policy:write").await?;
    require_org(&auth, &input.organization)?;
    if input.finding_id.is_none() && input.rule_id.is_none() {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "finding_id or rule_id is required".into(),
        ));
    }
    for (label, value) in [
        ("id", &input.id),
        ("path", &input.path),
        ("owner", &input.owner),
        ("reason", &input.reason),
        ("ticket_url", &input.ticket_url),
        ("approver", &input.approver),
    ] {
        if value.trim().is_empty() {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{label} is required"),
            ));
        }
    }
    let created_at = parse_time(&input.created_at)?;
    let expires_at = parse_time(&input.expires_at)?;
    if expires_at <= created_at || expires_at <= Utc::now() {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "exception expiry must be after creation and in the future".into(),
        ));
    }
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT id FROM repositories WHERE organization_id=$1 AND full_name=$2")
            .bind(auth.organization_id)
            .bind(&input.repository)
            .fetch_one(&state.pool)
            .await
            .map_err(not_found)?;
    sqlx::query("INSERT INTO exceptions (id,organization_id,repository_id,finding_id,rule_id,path_glob,owner,reason,ticket_url,approver,created_at,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(&input.id).bind(auth.organization_id).bind(repository_id).bind(&input.finding_id).bind(&input.rule_id).bind(&input.path).bind(&input.owner).bind(&input.reason).bind(&input.ticket_url).bind(&input.approver).bind(created_at).bind(expires_at)
        .execute(&state.pool).await.map_err(conflict)?;
    audit(
        &state,
        &auth,
        "exception.create",
        "exception",
        &input.id,
        json!({"repository":input.repository,"expires_at":input.expires_at}),
        &headers,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":input.id}))))
}

async fn register_installation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterInstallationRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let auth = authenticate(&state, &headers, "admin").await?;
    require_org(&auth, &input.organization)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO github_installations (organization_id,installation_id,account_login,api_url,web_url) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (installation_id) DO UPDATE SET account_login=EXCLUDED.account_login,api_url=EXCLUDED.api_url,web_url=EXCLUDED.web_url,suspended_at=NULL RETURNING id")
        .bind(auth.organization_id).bind(input.installation_id).bind(&input.account_login)
        .bind(input.api_url.unwrap_or_else(|| state.config.github_api_url.clone()))
        .bind(input.web_url.unwrap_or_else(|| state.config._github_web_url.clone()))
        .fetch_one(&state.pool).await.map_err(internal)?;
    audit(
        &state,
        &auth,
        "github.installation.register",
        "github_installation",
        &id.to_string(),
        json!({"installation_id":input.installation_id,"account_login":input.account_login}),
        &headers,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}

async fn resolved_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PolicyQuery>,
) -> ApiResult<Json<SignedDocumentV1>> {
    let auth = authenticate(&state, &headers, "policy:read").await?;
    require_org(&auth, &query.organization)?;
    let value: Value = sqlx::query_scalar("SELECT pv.signed_bundle FROM policy_assignments pa JOIN repositories r ON r.id=pa.repository_id JOIN policy_versions pv ON pv.id=pa.policy_version_id WHERE pa.organization_id=$1 AND r.full_name=$2 AND pa.active")
        .bind(auth.organization_id).bind(&query.repository).fetch_one(&state.pool).await.map_err(not_found)?;
    let bundle = serde_json::from_value(value).map_err(internal)?;
    Ok(Json(bundle))
}

async fn ingest_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(envelope): Json<ScanEnvelopeV1>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let auth = authenticate(&state, &headers, "scan:write").await?;
    require_org(&auth, &envelope.repository.organization)?;
    validate_scan_envelope(&envelope)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT id FROM repositories WHERE organization_id=$1 AND full_name=$2")
            .bind(auth.organization_id)
            .bind(&envelope.repository.repository)
            .fetch_one(&mut *tx)
            .await
            .map_err(not_found)?;
    let existing = sqlx::query_scalar::<_, Uuid>("SELECT id FROM scan_runs WHERE repository_id=$1 AND commit_sha=$2 AND policy_digest=$3 AND attempt=$4")
        .bind(repository_id).bind(&envelope.repository.commit_sha).bind(&envelope.policy_digest).bind(envelope.run.attempt as i32)
        .fetch_optional(&mut *tx).await.map_err(internal)?;
    if let Some(id) = existing {
        tx.rollback().await.ok();
        state
            .metrics
            .ingested_scans
            .with_label_values(&["duplicate"])
            .inc();
        return Ok((StatusCode::OK, Json(json!({"id": id, "duplicate": true}))));
    }
    let started_at = parse_time(&envelope.run.started_at)?;
    let completed_at = parse_time(&envelope.run.completed_at)?;
    let blocked = envelope
        .findings
        .iter()
        .any(|finding| finding.enforcement == EnforcementOutcome::Blocked);
    let analysis_passed = envelope
        .analysis
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let status = if !analysis_passed {
        "incomplete"
    } else if blocked {
        "failed"
    } else {
        "passed"
    };
    let run_key = hex_sha(
        format!(
            "{}:{}:{}:{}",
            repository_id,
            envelope.repository.commit_sha,
            envelope.policy_digest,
            envelope.run.attempt
        )
        .as_bytes(),
    );
    let id: Uuid = sqlx::query_scalar("INSERT INTO scan_runs (organization_id,repository_id,run_key,external_run_id,commit_sha,pull_request,policy_digest,attempt,status,analysis,metrics,cost,files,pr_summary,started_at,completed_at,tool_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) RETURNING id")
        .bind(auth.organization_id).bind(repository_id).bind(run_key).bind(&envelope.run.id).bind(&envelope.repository.commit_sha)
        .bind(envelope.repository.pull_request.map(|value| value as i64)).bind(&envelope.policy_digest).bind(envelope.run.attempt as i32).bind(status)
        .bind(&envelope.analysis).bind(&envelope.metrics).bind(&envelope.cost).bind(serde_json::to_value(&envelope.files).map_err(internal)?).bind(&envelope.pr_summary)
        .bind(started_at).bind(completed_at).bind(&envelope.run.tool_version).fetch_one(&mut *tx).await.map_err(internal)?;
    sqlx::query("UPDATE scan_runs SET superseded_by=$1 WHERE repository_id=$2 AND commit_sha=$3 AND policy_digest=$4 AND attempt < $5 AND superseded_by IS NULL")
        .bind(id).bind(repository_id).bind(&envelope.repository.commit_sha).bind(&envelope.policy_digest).bind(envelope.run.attempt as i32).execute(&mut *tx).await.map_err(internal)?;
    let finding_ids = envelope
        .findings
        .iter()
        .map(|finding| finding.finding_id.clone())
        .collect::<Vec<_>>();
    sqlx::query("UPDATE findings SET resolved_at=$1 WHERE organization_id=$2 AND repository_id=$3 AND resolved_at IS NULL AND NOT (finding_id = ANY($4))")
        .bind(completed_at).bind(auth.organization_id).bind(repository_id).bind(&finding_ids).execute(&mut *tx).await.map_err(internal)?;
    for finding in &envelope.findings {
        sqlx::query("INSERT INTO findings (organization_id,repository_id,finding_id,rule_id,first_seen_at,last_seen_at,latest_scan_run_id,latest_payload) VALUES ($1,$2,$3,$4,$5,$5,$6,$7) ON CONFLICT (organization_id,repository_id,finding_id) DO UPDATE SET last_seen_at=EXCLUDED.last_seen_at, resolved_at=NULL, recurrence_count=findings.recurrence_count + CASE WHEN findings.resolved_at IS NULL THEN 0 ELSE 1 END, latest_scan_run_id=EXCLUDED.latest_scan_run_id, latest_payload=EXCLUDED.latest_payload")
            .bind(auth.organization_id).bind(repository_id).bind(&finding.finding_id).bind(&finding.rule_id).bind(completed_at).bind(id)
            .bind(serde_json::to_value(finding).map_err(internal)?).execute(&mut *tx).await.map_err(internal)?;
    }
    append_audit(&mut tx, auth.organization_id, &auth.actor, "scan.ingest", "scan_run", &id.to_string(), json!({"commit_sha": envelope.repository.commit_sha, "policy_digest": envelope.policy_digest, "attempt": envelope.run.attempt}), correlation_id(&headers)).await?;
    tx.commit().await.map_err(internal)?;
    state
        .metrics
        .ingested_scans
        .with_label_values(&[status])
        .inc();
    if let Err(error) = complete_matching_check(&state, repository_id, &envelope, status).await {
        warn!(error=%error, "failed to complete GitHub check");
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "duplicate": false, "status": status})),
    ))
}

async fn ingest_cost(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(bundle): Json<CostObservationBundleV1>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let auth = authenticate(&state, &headers, "cost:write").await?;
    require_org(&auth, &bundle.organization)?;
    validate_cost_bundle(&bundle)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT id FROM repositories WHERE organization_id=$1 AND full_name=$2")
            .bind(auth.organization_id)
            .bind(&bundle.repository)
            .fetch_one(&mut *tx)
            .await
            .map_err(not_found)?;
    let mut inserted = 0u64;
    for item in &bundle.observations {
        let result = sqlx::query("INSERT INTO cost_observations (organization_id,repository_id,model_id,window_start,window_end,executions,bytes_processed,compute_seconds,credits,cost_usd,currency,provenance) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING")
            .bind(auth.organization_id).bind(repository_id).bind(&item.model_id).bind(parse_time(&item.window_start)?).bind(parse_time(&item.window_end)?)
            .bind(item.executions as i64).bind(item.bytes_processed).bind(item.compute_seconds).bind(item.credits).bind(item.cost_usd).bind(&bundle.currency).bind(&bundle.provenance)
            .execute(&mut *tx).await.map_err(internal)?;
        inserted += result.rows_affected();
    }
    append_audit(
        &mut tx,
        auth.organization_id,
        &auth.actor,
        "cost.ingest",
        "repository",
        &repository_id.to_string(),
        json!({"inserted": inserted, "provenance": bundle.provenance}),
        correlation_id(&headers),
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({"inserted": inserted}))))
}

async fn report_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReportQuery>,
) -> ApiResult<Response> {
    let auth = authenticate(&state, &headers, "policy:read").await?;
    require_org(&auth, &query.organization)?;
    let row = sqlx::query("SELECT (SELECT count(*) FROM repositories WHERE organization_id=$1) repositories, (SELECT count(*) FROM scan_runs WHERE organization_id=$1) scans, (SELECT count(*) FROM findings WHERE organization_id=$1 AND resolved_at IS NULL) open_findings, (SELECT count(*) FROM exceptions WHERE organization_id=$1 AND expires_at < now()) expired_exceptions, (SELECT coalesce(sum(cost_usd),0) FROM cost_observations WHERE organization_id=$1) validated_cost_usd")
        .bind(auth.organization_id).fetch_one(&state.pool).await.map_err(internal)?;
    let report = json!({
        "repositories": row.get::<i64,_>("repositories"), "scans": row.get::<i64,_>("scans"),
        "open_findings": row.get::<i64,_>("open_findings"), "expired_exceptions": row.get::<i64,_>("expired_exceptions"),
        "validated_cost_usd": row.get::<f64,_>("validated_cost_usd"),
        "measures": {"projected_savings": "estimated", "prevented_cost": "requires earlier PR revision disappearance", "validated_savings": "requires imported before/after observations"}
    });
    if query.format.as_deref() == Some("csv") {
        let csv = format!("metric,value\nrepositories,{}\nscans,{}\nopen_findings,{}\nexpired_exceptions,{}\nvalidated_cost_usd,{}\n", report["repositories"], report["scans"], report["open_findings"], report["expired_exceptions"], report["validated_cost_usd"]);
        Ok(([(header::CONTENT_TYPE, "text/csv")], csv).into_response())
    } else {
        Ok(Json(report).into_response())
    }
}

async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    if state.config.github_webhook_secret.is_empty() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub webhook is not configured".into(),
        ));
    }
    verify_webhook_signature(&state.config.github_webhook_secret, &headers, &body)?;
    let delivery = required_header(&headers, "x-github-delivery")?;
    let event_type = required_header(&headers, "x-github-event")?;
    let payload_hash = hex_sha(&body);
    let inserted = sqlx::query("INSERT INTO webhook_deliveries (delivery_id,event_type,payload_hash,status) VALUES ($1,$2,$3,'processing') ON CONFLICT DO NOTHING")
        .bind(&delivery).bind(&event_type).bind(&payload_hash).execute(&state.pool).await.map_err(internal)?.rows_affected();
    if inserted == 0 {
        state
            .metrics
            .webhook_deliveries
            .with_label_values(&["duplicate"])
            .inc();
        return Ok(StatusCode::ACCEPTED);
    }
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let result = process_github_event(&state, &event_type, &payload).await;
    match result {
        Ok(()) => {
            sqlx::query("UPDATE webhook_deliveries SET status='completed', completed_at=now() WHERE delivery_id=$1").bind(&delivery).execute(&state.pool).await.map_err(internal)?;
            state
                .metrics
                .webhook_deliveries
                .with_label_values(&["completed"])
                .inc();
            Ok(StatusCode::ACCEPTED)
        }
        Err(error) => {
            sqlx::query("UPDATE webhook_deliveries SET status='failed', error=$2, completed_at=now() WHERE delivery_id=$1").bind(&delivery).bind(truncate(&error.to_string(), 2000)).execute(&state.pool).await.map_err(internal)?;
            state
                .metrics
                .webhook_deliveries
                .with_label_values(&["failed"])
                .inc();
            Err(ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webhook processing failed".into(),
            ))
        }
    }
}

async fn process_github_event(
    state: &AppState,
    event_type: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    if event_type == "installation" {
        let installation_id = payload
            .pointer("/installation/id")
            .and_then(Value::as_i64)
            .context("missing installation.id")?;
        let account = payload
            .pointer("/installation/account/login")
            .and_then(Value::as_str)
            .context("missing installation.account.login")?;
        let action = payload.get("action").and_then(Value::as_str).unwrap_or("");
        if action == "deleted" || action == "suspend" {
            sqlx::query(
                "UPDATE github_installations SET suspended_at=now() WHERE installation_id=$1",
            )
            .bind(installation_id)
            .execute(&state.pool)
            .await?;
        } else {
            sqlx::query("INSERT INTO github_installations (organization_id,installation_id,account_login,api_url,web_url) SELECT id,$1,$2,$3,$4 FROM organizations WHERE lower(slug)=lower($2) ON CONFLICT (installation_id) DO UPDATE SET account_login=EXCLUDED.account_login,api_url=EXCLUDED.api_url,web_url=EXCLUDED.web_url,suspended_at=NULL")
                .bind(installation_id).bind(account).bind(&state.config.github_api_url).bind(&state.config._github_web_url).execute(&state.pool).await?;
        }
        return Ok(());
    }
    if event_type == "installation_repositories" {
        let installation_id = payload
            .pointer("/installation/id")
            .and_then(Value::as_i64)
            .context("missing installation.id")?;
        let organization_id: Uuid = sqlx::query_scalar(
            "SELECT organization_id FROM github_installations WHERE installation_id=$1",
        )
        .bind(installation_id)
        .fetch_one(&state.pool)
        .await?;
        for repository in payload
            .get("repositories_added")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(full_name)) = (
                repository.get("id").and_then(Value::as_i64),
                repository.get("full_name").and_then(Value::as_str),
            ) {
                sqlx::query("INSERT INTO repositories (organization_id,external_id,full_name) VALUES ($1,$2,$3) ON CONFLICT (organization_id,full_name) DO UPDATE SET external_id=EXCLUDED.external_id")
                    .bind(organization_id).bind(id).bind(full_name).execute(&state.pool).await?;
            }
        }
        return Ok(());
    }
    if event_type != "pull_request" && event_type != "check_run" {
        return Ok(());
    }
    if event_type == "check_run" {
        return Ok(());
    }
    let repository = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .context("missing repository.full_name")?;
    let repository_external_id = payload.pointer("/repository/id").and_then(Value::as_i64);
    let head_sha = payload
        .pointer("/pull_request/head/sha")
        .and_then(Value::as_str)
        .context("missing pull_request.head.sha")?;
    let installation_id = payload
        .pointer("/installation/id")
        .and_then(Value::as_i64)
        .context("missing installation.id")?;
    let row = sqlx::query("SELECT r.id repository_id,r.organization_id,pv.digest,gi.api_url FROM repositories r JOIN policy_assignments pa ON pa.repository_id=r.id AND pa.active JOIN policy_versions pv ON pv.id=pa.policy_version_id JOIN github_installations gi ON gi.organization_id=r.organization_id AND gi.installation_id=$2 WHERE r.full_name=$1 OR r.external_id=$3")
        .bind(repository).bind(installation_id).bind(repository_external_id).fetch_one(&state.pool).await?;
    let repository_id: Uuid = row.get("repository_id");
    let organization_id: Uuid = row.get("organization_id");
    let digest: String = row.get("digest");
    let api_url: String = row.get("api_url");
    let details_url = format!("{}/ui/scans", state.config.public_url);
    let id: Uuid = sqlx::query_scalar("INSERT INTO check_runs (organization_id,repository_id,head_sha,policy_digest,status,details_url) VALUES ($1,$2,$3,$4,'queued',$5) ON CONFLICT (repository_id,head_sha,policy_digest) DO UPDATE SET updated_at=now() RETURNING id")
        .bind(organization_id).bind(repository_id).bind(head_sha).bind(&digest).bind(&details_url).fetch_one(&state.pool).await?;
    let github_id = create_github_check(
        state,
        installation_id,
        &api_url,
        repository,
        head_sha,
        &details_url,
    )
    .await?;
    sqlx::query("UPDATE check_runs SET github_check_run_id=$2,status='in_progress',updated_at=now() WHERE id=$1").bind(id).bind(github_id).execute(&state.pool).await?;
    Ok(())
}

async fn create_github_check(
    state: &AppState,
    installation_id: i64,
    api_url: &str,
    repository: &str,
    head_sha: &str,
    details_url: &str,
) -> anyhow::Result<i64> {
    let token = installation_token(state, installation_id, api_url).await?;
    let response = state.client.post(format!("{api_url}/repos/{repository}/check-runs"))
        .bearer_auth(token).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28")
        .json(&json!({"name":"Costguard","head_sha":head_sha,"status":"in_progress","started_at":Utc::now().to_rfc3339(),"details_url":details_url}))
        .send().await?.error_for_status()?;
    response
        .json::<Value>()
        .await?
        .get("id")
        .and_then(Value::as_i64)
        .context("GitHub check response missing id")
}

async fn complete_matching_check(
    state: &AppState,
    repository_id: Uuid,
    envelope: &ScanEnvelopeV1,
    status: &str,
) -> anyhow::Result<()> {
    let Some(row) = sqlx::query("SELECT cr.id,cr.github_check_run_id,r.full_name,gi.installation_id,gi.api_url FROM check_runs cr JOIN repositories r ON r.id=cr.repository_id JOIN github_installations gi ON gi.organization_id=cr.organization_id WHERE cr.repository_id=$1 AND cr.head_sha=$2 AND cr.policy_digest=$3")
        .bind(repository_id).bind(&envelope.repository.commit_sha).bind(&envelope.policy_digest).fetch_optional(&state.pool).await? else { return Ok(()); };
    let Some(github_id) = row.get::<Option<i64>, _>("github_check_run_id") else {
        return Ok(());
    };
    let repository: String = row.get("full_name");
    let installation_id: i64 = row.get("installation_id");
    let api_url: String = row.get("api_url");
    let token = installation_token(state, installation_id, &api_url).await?;
    let conclusion = match status {
        "incomplete" => "action_required",
        "failed" => "failure",
        _ => "success",
    };
    let annotations = envelope.findings.iter().map(|finding| json!({
        "path": finding.path, "start_line": finding.line.max(1), "end_line": finding.line.max(1),
        "annotation_level": match finding.enforcement { EnforcementOutcome::Blocked => "failure", EnforcementOutcome::Warned => "warning", _ => "notice" },
        "message": truncate(&finding.message, 64_000), "title": finding.rule_id
    })).collect::<Vec<_>>();
    let chunks = if annotations.is_empty() {
        vec![Vec::new()]
    } else {
        annotations.chunks(50).map(|chunk| chunk.to_vec()).collect()
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let final_chunk = index + 1 == chunks.len();
        let mut body = json!({"output":{"title":"Costguard","summary":format!("{} findings", envelope.findings.len()),"annotations":chunk}});
        if final_chunk {
            body["status"] = json!("completed");
            body["conclusion"] = json!(conclusion);
            body["completed_at"] = json!(Utc::now().to_rfc3339());
        }
        state
            .client
            .patch(format!(
                "{api_url}/repos/{repository}/check-runs/{github_id}"
            ))
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
    }
    sqlx::query(
        "UPDATE check_runs SET status='completed',conclusion=$2,updated_at=now() WHERE id=$1",
    )
    .bind(row.get::<Uuid, _>("id"))
    .bind(conclusion)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn installation_token(
    state: &AppState,
    installation_id: i64,
    api_url: &str,
) -> anyhow::Result<String> {
    if let Some(cached) = state
        .installation_tokens
        .lock()
        .await
        .get(&installation_id)
        .cloned()
    {
        if cached.expires_at > Utc::now() + Duration::minutes(2) {
            return Ok(cached.token);
        }
    }
    let app_id = state
        .config
        .github_app_id
        .as_ref()
        .context("GITHUB_APP_ID is not configured")?;
    let private_key = state
        .config
        .github_private_key
        .as_ref()
        .context("GITHUB_APP_PRIVATE_KEY is not configured")?
        .replace("\\n", "\n");
    let now = Utc::now();
    let jwt = jsonwebtoken::encode(
        &Header::new(Algorithm::RS256),
        &GitHubJwtClaims {
            iat: (now - Duration::seconds(30)).timestamp(),
            exp: (now + Duration::minutes(9)).timestamp(),
            iss: app_id.clone(),
        },
        &EncodingKey::from_rsa_pem(private_key.as_bytes())?,
    )?;
    let value: Value = state
        .client
        .post(format!(
            "{api_url}/app/installations/{installation_id}/access_tokens"
        ))
        .bearer_auth(jwt)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .context("GitHub installation token missing")?
        .to_string();
    let expires_at = value
        .get("expires_at")
        .and_then(Value::as_str)
        .map(parse_time_any)
        .transpose()?
        .unwrap_or(now + Duration::minutes(50));
    state.installation_tokens.lock().await.insert(
        installation_id,
        CachedInstallationToken {
            token: token.clone(),
            expires_at,
        },
    );
    Ok(token)
}

fn spawn_check_timeout_job(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = sqlx::query("UPDATE check_runs SET status='completed',conclusion='action_required',updated_at=now() WHERE status IN ('queued','in_progress') AND created_at < now() - interval '20 minutes'").execute(&state.pool).await { warn!(error=%error, "check timeout job failed"); }
        }
    });
}

async fn login_page() -> Html<&'static str> {
    Html("<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; frame-ancestors 'none'\"></head><body><h1>Costguard login</h1><form method=\"post\"><label>Service token <input type=\"password\" name=\"token\" required></label><button>Sign in</button></form></body></html>")
}

async fn login(State(state): State<AppState>, Form(input): Form<LoginForm>) -> ApiResult<Response> {
    let auth = authenticate_token(&state, &input.token, "policy:read").await?;
    let session = random_token("cgs");
    let hash = hex_sha(session.as_bytes());
    let csrf = random_token("csrf");
    sqlx::query("INSERT INTO ui_sessions (organization_id,session_hash,csrf_token,role,expires_at) VALUES ($1,$2,$3,'viewer',now()+interval '30 minutes')")
        .bind(auth.organization_id).bind(hash).bind(csrf).execute(&state.pool).await.map_err(internal)?;
    let cookie = format!(
        "costguard_session={session}; Path=/; Max-Age=1800; HttpOnly; Secure; SameSite=Strict"
    );
    let mut response = Redirect::to("/ui/organizations").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?,
    );
    Ok(response)
}

async fn ui_page(
    State(state): State<AppState>,
    Path(page): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Html<String>> {
    let session = ui_session(&state, &headers).await?;
    let table = match page.as_str() {
        "organizations" => "organizations",
        "repositories" => "repositories",
        "policies" => "policy_versions",
        "assignments" => "policy_assignments",
        "exceptions" => "exceptions",
        "tokens" => "service_tokens",
        "scans" => "scan_runs",
        "findings" => "findings",
        "costs" => "cost_observations",
        "reports" => "scan_runs",
        "audit" => "audit_events",
        _ => return Err(ApiError(StatusCode::NOT_FOUND, "page not found".into())),
    };
    let query = if table == "organizations" {
        "SELECT count(*)::bigint FROM organizations WHERE id=$1".to_string()
    } else {
        format!("SELECT count(*)::bigint FROM {table} WHERE organization_id=$1")
    };
    let count: i64 = sqlx::query_scalar(&query)
        .bind(session.0)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let template = DashboardTemplate {
        page,
        organization: session.1,
        rows: vec![
            ("records".into(), count.to_string()),
            ("version".into(), env!("CARGO_PKG_VERSION").into()),
        ],
    };
    Ok(Html(template.render().map_err(internal)?))
}

async fn ui_session(state: &AppState, headers: &HeaderMap) -> ApiResult<(Uuid, String)> {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let token = cookie
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("costguard_session="))
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "login required".into()))?;
    let row = sqlx::query("SELECT s.organization_id,o.slug FROM ui_sessions s JOIN organizations o ON o.id=s.organization_id WHERE s.session_hash=$1 AND s.expires_at>now()")
        .bind(hex_sha(token.as_bytes())).fetch_optional(&state.pool).await.map_err(internal)?.ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED,"session expired".into()))?;
    Ok((row.get("organization_id"), row.get("slug")))
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    scope: &str,
) -> ApiResult<AuthContext> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "bearer token required".into()))?;
    authenticate_token(state, value, scope).await
}

async fn authenticate_token(state: &AppState, token: &str, scope: &str) -> ApiResult<AuthContext> {
    let prefix = token
        .split('_')
        .nth(1)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "invalid token".into()))?;
    let row = sqlx::query("SELECT t.organization_id,o.slug,t.token_hash,t.scopes FROM service_tokens t JOIN organizations o ON o.id=t.organization_id WHERE t.prefix=$1 AND t.revoked_at IS NULL AND t.expires_at>now()")
        .bind(prefix).fetch_optional(&state.pool).await.map_err(internal)?.ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED,"invalid token".into()))?;
    let hash: String = row.get("token_hash");
    let candidate = token.to_string();
    let verified = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).ok().is_some_and(|parsed| {
            Argon2::default()
                .verify_password(candidate.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .map_err(internal)?;
    if !verified {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "invalid token".into()));
    }
    let scopes: Vec<String> = row.get("scopes");
    if !scopes.iter().any(|item| item == scope || item == "admin") {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            format!("token lacks {scope} scope"),
        ));
    }
    let organization_id: Uuid = row.get("organization_id");
    sqlx::query("UPDATE service_tokens SET last_used_at=now() WHERE prefix=$1")
        .bind(prefix)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(AuthContext {
        organization_id,
        organization_slug: row.get("slug"),
        actor: format!("token:{prefix}"),
    })
}

async fn new_service_token() -> ApiResult<(String, String, String)> {
    let prefix = Uuid::new_v4().simple().to_string()[..10].to_string();
    let token = format!("cg_{prefix}_{}", random_token("secret"));
    let candidate = token.clone();
    let hash = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(candidate.as_bytes(), &salt)
            .map(|value| value.to_string())
    })
    .await
    .map_err(internal)?
    .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok((token, prefix, hash))
}

#[allow(clippy::too_many_arguments)]
async fn append_audit(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    actor: &str,
    action: &str,
    target_type: &str,
    target_id: &str,
    details: Value,
    correlation_id: String,
) -> ApiResult<()> {
    let previous: Option<String> = sqlx::query_scalar("SELECT event_hash FROM audit_events WHERE organization_id=$1 ORDER BY occurred_at DESC,id DESC LIMIT 1 FOR UPDATE")
        .bind(organization_id).fetch_optional(&mut **tx).await.map_err(internal)?;
    let occurred_at = Utc::now();
    let material = json!({"organization_id":organization_id,"actor":actor,"action":action,"target_type":target_type,"target_id":target_id,"occurred_at":occurred_at,"correlation_id":correlation_id,"details":details,"previous_hash":previous});
    let event_hash = hex_sha(serde_json::to_vec(&material).map_err(internal)?.as_slice());
    sqlx::query("INSERT INTO audit_events (organization_id,actor,action,target_type,target_id,occurred_at,correlation_id,details,previous_hash,event_hash) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(organization_id).bind(actor).bind(action).bind(target_type).bind(target_id).bind(occurred_at).bind(correlation_id).bind(details).bind(previous).bind(event_hash)
        .execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

async fn audit(
    state: &AppState,
    auth: &AuthContext,
    action: &str,
    target_type: &str,
    target_id: &str,
    details: Value,
    headers: &HeaderMap,
) -> ApiResult<()> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    append_audit(
        &mut tx,
        auth.organization_id,
        &auth.actor,
        action,
        target_type,
        target_id,
        details,
        correlation_id(headers),
    )
    .await?;
    tx.commit().await.map_err(internal)
}

fn validate_scan_envelope(envelope: &ScanEnvelopeV1) -> ApiResult<()> {
    if envelope.schema_version != costguard_protocol::SCAN_SCHEMA_VERSION {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported scan schema".into(),
        ));
    }
    if envelope.policy_digest.is_empty() || envelope.run.attempt == 0 {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "policy digest and positive attempt are required".into(),
        ));
    }
    if envelope
        .findings
        .iter()
        .any(|finding| finding.finding_id.is_empty() || finding.evidence_key.is_empty())
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "findings require stable identities".into(),
        ));
    }
    Ok(())
}

fn validate_cost_bundle(bundle: &CostObservationBundleV1) -> ApiResult<()> {
    if bundle.schema_version != 1 || bundle.currency != "USD" {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "cost bundle must use schema 1 and USD".into(),
        ));
    }
    let mut keys = std::collections::HashSet::new();
    for item in &bundle.observations {
        let start = parse_time(&item.window_start)?;
        let end = parse_time(&item.window_end)?;
        if end <= start
            || [
                item.bytes_processed,
                item.compute_seconds,
                item.credits,
                item.cost_usd,
            ]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value < 0.0)
        {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid cost observation".into(),
            ));
        }
        if !keys.insert((&item.model_id, &item.window_start, &item.window_end)) {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "duplicate cost observation".into(),
            ));
        }
    }
    Ok(())
}

fn verify_webhook_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> ApiResult<()> {
    let supplied = required_header(headers, "x-hub-signature-256")?;
    let signature = supplied.strip_prefix("sha256=").ok_or_else(|| {
        ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid webhook signature format".into(),
        )
    })?;
    let decoded = decode_hex(signature)
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "invalid webhook signature".into()))?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    mac.update(body);
    mac.verify_slice(&decoded)
        .map_err(|_| ApiError(StatusCode::UNAUTHORIZED, "invalid webhook signature".into()))
}

fn verify_bootstrap_secret(expected: &str, headers: &HeaderMap) -> ApiResult<()> {
    let supplied = required_header(headers, "x-costguard-bootstrap-secret")?;
    if expected.len() != supplied.len()
        || !bool::from(expected.as_bytes().ct_eq(supplied.as_bytes()))
    {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid bootstrap secret".into(),
        ));
    }
    Ok(())
}

fn require_org(auth: &AuthContext, slug: &str) -> ApiResult<()> {
    if auth.organization_slug != slug {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "cross-organization access denied".into(),
        ));
    }
    Ok(())
}

fn required_header(headers: &HeaderMap, name: &str) -> ApiResult<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("missing {name} header")))
}

fn parse_time(value: &str) -> ApiResult<chrono::DateTime<Utc>> {
    parse_time_any(value)
        .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
}
fn parse_time_any(value: &str) -> anyhow::Result<chrono::DateTime<Utc>> {
    Ok(chrono::DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}
fn correlation_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}
fn required_env(name: &str) -> anyhow::Result<String> {
    env::var(name).with_context(|| format!("{name} is required"))
}
fn internal<E: std::fmt::Display>(error: E) -> ApiError {
    error!(error=%error,"internal error");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".into(),
    )
}
fn conflict<E: std::fmt::Display>(error: E) -> ApiError {
    ApiError(StatusCode::CONFLICT, error.to_string())
}
fn not_found<E: std::fmt::Display>(_error: E) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "resource not found".into())
}
fn random_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "{prefix}_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}
fn hex_sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}
fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}

use subtle::ConstantTimeEq;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_signature_rejects_tampering() {
        let body = br#"{"action":"opened"}"#;
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = mac.finalize().into_bytes();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            HeaderValue::from_str(&format!("sha256={}", hex_bytes(&signature))).unwrap(),
        );
        assert!(verify_webhook_signature("secret", &headers, body).is_ok());
        assert!(verify_webhook_signature("secret", &headers, b"tampered").is_err());
    }

    #[test]
    fn hex_decoder_rejects_invalid_input() {
        assert_eq!(decode_hex("00ff"), Some(vec![0, 255]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
    }

    fn hex_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
