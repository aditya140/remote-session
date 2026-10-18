use crate::{Error, Gateway};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::post,
};
use qgenie_remote_proto::{
    SessionId, decode_json,
    rest::{AdminHistoryQuery, HistoryQuery, HoldChange, HoldCommit, RecordingAuthorityRequest},
    wire::{LossRange, RecordingBatch, Timestamp},
};

pub(crate) fn routes() -> Router<Gateway> {
    Router::new()
        .route(
            "/v1/recordings/chunks",
            post(upload_content).layer(DefaultBodyLimit::max(128 * 1024)),
        )
        .route("/v1/recordings/gaps", post(upload_gap))
        .route(
            "/v1/recordings/sessions/{session}/authority",
            post(authority),
        )
        .route("/v1/sessions/{session}/history", post(owner_history))
        .route(
            "/v1/sessions/{session}/history/streams",
            post(owner_streams),
        )
        .route(
            "/v1/admin/sessions/{session}/history/streams",
            post(admin_streams),
        )
        .route("/v1/admin/sessions/{session}/history", post(admin_history))
}
fn private<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}
async fn owner_streams(
    State(state): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_client(&headers).await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let query: qgenie_remote_proto::rest::HistoryStreamsQuery =
        decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings
            .owner
            .owner_history_streams(identity, &actor, session, query)
            .await?,
    ))
}
async fn admin_streams(
    State(state): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_client(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let query: qgenie_remote_proto::rest::AdminHistoryStreamsQuery =
        decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings
            .admin
            .admin_history_streams(identity, &actor, session, query)
            .await?,
    ))
}
async fn upload_content(
    State(state): State<Gateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_host(&headers).await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let batch: RecordingBatch = decode_json(&body).map_err(|_| Error::Invalid)?;
    if batch.machine != actor.machine {
        return Err(Error::Forbidden);
    }
    // This cheap authority projection bounds preparation. Ingest rechecks the actual
    // immutable lifetime and primary scope inside its ordered transaction after CPU work.
    let started: serde_json::Value = sqlx::query_scalar(
        "SELECT body->'created_at' FROM qgr.sessions WHERE id=$1 AND machine=$2 AND owner=$3",
    )
    .bind(batch.session.to_string())
    .bind(actor.machine.to_string())
    .bind(actor.owner.to_string())
    .fetch_optional(&state.store.pool)
    .await
    .map_err(|_| Error::Database)?
    .ok_or(Error::Forbidden)?;
    let started: Timestamp = serde_json::from_value(started).map_err(|_| Error::Database)?;
    let now =
        Timestamp::from_unix_millis(crate::identity::now_ms()?).map_err(|_| Error::Unavailable)?;
    let validated = recordings
        .validation
        .prepare(batch, now, started, None)
        .await?;
    let protected = recordings.redaction.protect(validated).await?;
    Ok(private(
        recordings
            .ingest
            .commit_recording(identity, &actor, protected)
            .await?,
    ))
}
async fn upload_gap(
    State(state): State<Gateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_host(&headers).await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let gap: LossRange = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings.ingest.commit_gap(identity, &actor, gap).await?,
    ))
}
async fn authority(
    State(state): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_host(&headers).await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let request: RecordingAuthorityRequest = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings
            .ingest
            .recording_authority(identity, &actor, session, request.challenge)
            .await?,
    ))
}
async fn owner_history(
    State(state): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_client(&headers).await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let query: HistoryQuery = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings
            .owner
            .owner_history(identity, &actor, session, query)
            .await?,
    ))
}
async fn admin_history(
    State(state): State<Gateway>,
    Path(session): Path<SessionId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let actor = state.authenticate_client(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let recordings = state.recordings.as_ref().ok_or(Error::Configuration)?;
    let query: AdminHistoryQuery = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(private(
        recordings
            .admin
            .admin_history(identity, &actor, session, query)
            .await?,
    ))
}

pub(crate) async fn change_hold(
    State(state): State<Gateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<HoldCommit>, Error> {
    let actor = state.principal(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    state.ready().await?;
    let identity = state.identity.as_deref().ok_or(Error::Configuration)?;
    let lifecycle = state
        .recording_lifecycle
        .as_ref()
        .ok_or(Error::Configuration)?;
    let change: HoldChange = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(Json(lifecycle.change_hold(identity, &actor, change).await?))
}

#[cfg(test)]
pub(crate) mod tests;
