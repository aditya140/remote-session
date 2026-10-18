use crate::{
    AuditRecord, Config, Error, IdentityService, JwtConfig, JwtVerifier, MachinePrincipal, Store,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
};
use qgenie_remote_proto::{Counter, UserId};
use serde::Deserialize;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[derive(Debug, Clone)]
pub struct Principal {
    pub user: UserId,
    pub platform_admin: bool,
    pub expires_at: u64,
}
/// Only the composition's verified identity adapter may construct authenticated principals.
pub trait Authenticator: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, Error>> + Send + 'a>>;
}
pub struct RejectAuthentication;
impl Authenticator for RejectAuthentication {
    fn authenticate<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, Error>> + Send + 'a>> {
        Box::pin(async { Err(Error::Unauthenticated) })
    }
}
#[derive(Clone)]
pub struct Gateway {
    pub(crate) store: Store,
    pub(crate) audit: Option<Store>,
    pub(crate) auth: Arc<dyn Authenticator>,
    pub(crate) identity: Option<Arc<IdentityService>>,
    pub(crate) routing: Option<Arc<crate::routing::Routing>>,
    pub(crate) recording_lifecycle: Option<crate::RecordingDatabase>,
    pub(crate) recordings: Option<crate::transcripts::service::Recordings>,
    pub(crate) revocation_observations: Arc<crate::revocation::Propagation>,
    draining: Arc<AtomicBool>,
}
impl Gateway {
    pub async fn connect(config: &Config, auth: Arc<dyn Authenticator>) -> Result<Self, Error> {
        config.validate()?;
        let store = Store::connect(&config.database, config.pool_size, false).await?;
        let audit = match &config.audit_database {
            Some(secret) => Some(Store::connect(secret, 2, true).await?),
            None => None,
        };
        Ok(Self {
            store,
            audit,
            auth,
            identity: None,
            routing: None,
            recording_lifecycle: None,
            recordings: None,
            revocation_observations: Arc::new(crate::revocation::Propagation::default()),
            draining: Arc::new(AtomicBool::new(false)),
        })
    }
    pub async fn enable_routing(mut self, config: crate::RoutingConfig) -> Result<Self, Error> {
        if self.routing.is_some() {
            return Err(Error::Configuration);
        }
        self.routing = Some(Arc::new(
            crate::routing::Routing::start(self.clone(), config).await?,
        ));
        Ok(self)
    }
    pub async fn enable_recording_lifecycle(
        mut self,
        secret: &crate::SecretDsn,
    ) -> Result<Self, Error> {
        if self.routing.is_some()
            || self.recording_lifecycle.is_some()
            || self.identity.as_ref().is_none_or(|i| i.directory.is_none())
        {
            return Err(Error::Configuration);
        }
        let database =
            crate::RecordingDatabase::connect(secret, crate::RecordingRole::Lifecycle, 2).await?;
        let same = database.matches_authority(&self.store).await;
        if !matches!(same, Ok(true)) {
            database.close().await;
            return Err(same.err().unwrap_or(Error::Configuration));
        }
        self.recording_lifecycle = Some(database);
        Ok(self)
    }
    /// Install all recording HTTP purposes together, before routing clones the composition.
    /// Production ingest and audit writes still require qualified synchronous durability.
    pub async fn enable_recordings(
        mut self,
        ingest: &crate::SecretDsn,
        owner: &crate::SecretDsn,
        admin: &crate::SecretDsn,
        redaction: crate::RecordingRedaction,
    ) -> Result<Self, Error> {
        if self.routing.is_some()
            || self.recordings.is_some()
            || self.identity.as_ref().is_none_or(|i| i.directory.is_none())
        {
            return Err(Error::Configuration);
        }
        self.recordings = Some(
            crate::transcripts::service::Recordings::connect(
                &self.store,
                ingest,
                owner,
                admin,
                redaction,
            )
            .await?,
        );
        Ok(self)
    }
    pub async fn enable_identity(
        mut self,
        config: JwtConfig,
        origin: String,
        key_file: &std::path::Path,
    ) -> Result<Self, Error> {
        if self.identity.is_some() || self.routing.is_some() {
            return Err(Error::Configuration);
        }
        let identity = IdentityService::from_file(self.store.clone(), origin, key_file).await?;
        self.auth = Arc::new(JwtVerifier::connect(config, self.store.clone()).await?);
        self.identity = Some(Arc::new(identity));
        Ok(self)
    }
    /// Configure directory verification before starting routing or cloning the
    /// composition, so every admission and connected-principal check shares it.
    pub async fn enable_directory(mut self, config: crate::DirectoryConfig) -> Result<Self, Error> {
        if self.routing.is_some() {
            return Err(Error::Configuration);
        }
        let identity = self
            .identity
            .as_mut()
            .and_then(Arc::get_mut)
            .ok_or(Error::Configuration)?;
        if identity.directory.is_some() {
            return Err(Error::Configuration);
        }
        identity.directory = Some(Arc::new(
            crate::identity::freshness::DirectoryFreshness::start(
                self.store.clone(),
                crate::GraphDirectory::connect(config)?,
            )
            .await?,
        ));
        Ok(self)
    }
    pub async fn authenticate_client(&self, headers: &HeaderMap) -> Result<Principal, Error> {
        let actor = self.principal(headers).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Unauthenticated)?
            .as_secs();
        if actor.expires_at <= now {
            return Err(Error::Unauthenticated);
        }
        self.ready().await?;
        self.identity
            .as_ref()
            .ok_or(Error::Configuration)?
            .assert_user(actor.user)
            .await?;
        Ok(actor)
    }
    pub async fn authenticate_host(&self, headers: &HeaderMap) -> Result<MachinePrincipal, Error> {
        self.ready().await?;
        self.identity
            .as_ref()
            .ok_or(Error::Configuration)?
            .authenticate_machine(bearer(headers)?)
            .await
    }
    pub fn router(&self) -> Router {
        Router::new()
            .merge(crate::transcripts::api::routes())
            .route("/v1/connect/client", get(crate::routing::client_upgrade))
            .route("/v1/connect/host", get(crate::routing::host_upgrade))
            .route("/v1/admin/audit", get(read_audit))
            .route(
                "/v1/admin/holds",
                axum::routing::post(crate::transcripts::api::change_hold),
            )
            .route(
                "/v1/admin/revocations",
                axum::routing::post(crate::revocation::commit),
            )
            .route(
                "/v1/admin/revocations/{request}",
                get(crate::revocation::status),
            )
            .route(
                "/v1/enrollments/challenge",
                axum::routing::post(crate::identity::api::enrollment_challenge),
            )
            .route(
                "/v1/enrollments",
                axum::routing::post(crate::identity::api::enroll),
            )
            .route("/v1/machines", get(crate::identity::api::machines))
            .route(
                "/v1/machines/{machine}",
                get(crate::identity::api::machine).patch(crate::identity::api::patch),
            )
            .route(
                "/v1/machines/{machine}/challenges",
                axum::routing::post(crate::identity::api::challenge),
            )
            .route(
                "/v1/machines/{machine}/revoke",
                axum::routing::post(crate::identity::api::revoke),
            )
            .route(
                "/v1/machine-tokens",
                axum::routing::post(crate::identity::api::token),
            )
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
            .layer(
                tower::ServiceBuilder::new()
                    .layer(axum::error_handling::HandleErrorLayer::new(
                        |_: axum::BoxError| async { axum::http::StatusCode::SERVICE_UNAVAILABLE },
                    ))
                    .load_shed()
                    .concurrency_limit(256)
                    .timeout(Duration::from_secs(10)),
            )
            .with_state(self.clone())
    }
    pub fn health_router(&self) -> Router {
        Router::new()
            .route("/live", get(|| async { "live" }))
            .route("/ready", get(ready))
            .route("/metrics", get(metrics))
            .with_state(self.clone())
    }
    pub fn drain(&self) {
        self.draining.store(true, Ordering::Release);
        if let Some(routing) = &self.routing {
            routing.drain();
        }
    }
    pub async fn ready(&self) -> Result<(), Error> {
        if self.draining.load(Ordering::Acquire) {
            return Err(Error::Draining);
        }
        tokio::time::timeout(Duration::from_secs(2), self.store.compatible())
            .await
            .map_err(|_| Error::Database)?
    }
    pub async fn shutdown(&self) {
        self.drain();
        if let Some(routing) = &self.routing {
            routing.shutdown().await;
        }
        if let Some(identity) = &self.identity
            && let Some(directory) = &identity.directory
        {
            directory.shutdown().await;
        }
        self.store.close().await;
        if let Some(audit) = &self.audit {
            audit.close().await;
        }
        if let Some(lifecycle) = &self.recording_lifecycle {
            lifecycle.close().await;
        }
        if let Some(recordings) = &self.recordings {
            recordings.close().await;
        }
    }
    pub(crate) async fn principal(&self, headers: &HeaderMap) -> Result<Principal, Error> {
        let actor = self.auth.authenticate(bearer(headers)?).await?;
        if let Some(identity) = &self.identity {
            if actor.platform_admin {
                identity.assert_admin(actor.user).await?;
            } else {
                identity.assert_user(actor.user).await?;
            }
        }
        // HTTP authentication also has a strict expiry boundary after provider waits.
        if actor.expires_at <= crate::identity::jwt::unix_seconds()? {
            return Err(Error::Unauthenticated);
        }
        Ok(actor)
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str, Error> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| {
            !token.is_empty()
                && token.len() <= 16384
                && !token.bytes().any(|byte| byte.is_ascii_whitespace())
        })
        .ok_or(Error::Unauthenticated)
}
#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    after: Option<Counter>,
    #[serde(default = "default_limit")]
    limit: u16,
}
fn default_limit() -> u16 {
    100
}
async fn read_audit(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditRecord>>, Error> {
    let actor = state.principal(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    state.ready().await?;
    Ok(Json(
        state
            .audit
            .as_ref()
            .ok_or(Error::Forbidden)?
            .read_audit(actor.user, query.after.unwrap_or(Counter(0)), query.limit)
            .await?,
    ))
}
async fn ready(State(state): State<Gateway>) -> Result<&'static str, Error> {
    state.ready().await?;
    Ok(if routing_available(&state).await {
        "ready"
    } else {
        "ready degraded"
    })
}
async fn routing_available(state: &Gateway) -> bool {
    match &state.routing {
        Some(routing) => routing.broker_available().await,
        None => true,
    }
}
/// Private operational gauges. No owner/session identifiers, tokens or content labels.
async fn metrics(State(state): State<Gateway>) -> impl axum::response::IntoResponse {
    let ready = u8::from(state.ready().await.is_ok());
    let draining = u8::from(state.draining.load(Ordering::Acquire));
    let redis_available = u8::from(routing_available(&state).await);
    let (clients, hosts, pending, cross_node) = state
        .routing
        .as_ref()
        .map(|routing| routing.counts())
        .unwrap_or_default();
    let directory = state
        .identity
        .as_ref()
        .and_then(|identity| identity.directory.as_ref());
    let directory_enabled = u8::from(directory.is_some());
    let directory_sample = directory.map(|directory| directory.metrics()).transpose();
    let directory_collected = u8::from(directory_sample.is_ok());
    let sample = directory_sample.ok().flatten().unwrap_or_default();
    let (directory_active, directory_stale, directory_age) =
        (sample.active, sample.stale, sample.max_age_seconds);
    let operational = state
        .routing
        .as_ref()
        .map(|routing| routing.operational_metrics())
        .unwrap_or_default();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        format!(
            "# TYPE qgr_authority_ready gauge\nqgr_authority_ready {ready}\n# TYPE qgr_routing_available gauge\nqgr_routing_available {redis_available}\n# TYPE qgr_draining gauge\nqgr_draining {draining}\n# TYPE qgr_client_connections gauge\nqgr_client_connections {clients}\n# TYPE qgr_host_connections gauge\nqgr_host_connections {hosts}\n# TYPE qgr_pending_routes gauge\nqgr_pending_routes {pending}\n# TYPE qgr_cross_node_requests_sent_total counter\nqgr_cross_node_requests_sent_total {cross_node}\n# TYPE qgr_directory_enabled gauge\nqgr_directory_enabled {directory_enabled}\n# TYPE qgr_directory_collection_success gauge\nqgr_directory_collection_success {directory_collected}\n# TYPE qgr_directory_active_principals gauge\nqgr_directory_active_principals {directory_active}\n# TYPE qgr_directory_stale_principals gauge\nqgr_directory_stale_principals {directory_stale}\n# TYPE qgr_directory_max_evidence_age_seconds gauge\nqgr_directory_max_evidence_age_seconds {directory_age}\n"
        ) + &operational,
    )
}
