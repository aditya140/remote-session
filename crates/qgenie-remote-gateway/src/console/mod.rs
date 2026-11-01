//! Bounded metadata read models for the browser console. Native execution stays
//! behind the existing owner-authorized wire protocol.
mod health;
mod read;
mod telemetry;
#[cfg(test)]
pub(crate) mod tests;
mod types;
use crate::{Error, Gateway, Principal};
use axum::{
    Json, Router,
    extract::{Path, Query as ExtractQuery, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::get,
};
use qgenie_remote_proto::{MachineId, UserId};
use types::*;

pub(crate) fn routes() -> Router<Gateway> {
    Router::new()
        .route("/v1/me", get(me))
        .route("/v1/me/overview", get(owner_overview))
        .route("/v1/me/sessions", get(owner_sessions))
        .route("/v1/me/machines", get(owner_machines))
        .route("/v1/admin/overview", get(admin_overview))
        .route("/v1/admin/health", get(health::get))
        .route("/v1/admin/users", get(admin_users))
        .route("/v1/admin/users/{user}", get(admin_user))
        .route("/v1/admin/users/{user}/devices", get(admin_user_devices))
        .route("/v1/admin/users/{user}/sessions", get(admin_user_sessions))
        .route("/v1/admin/machines", get(admin_machines))
        .route(
            "/v1/admin/machines/{machine}/sessions",
            get(admin_machine_sessions),
        )
        .route("/v1/admin/sessions", get(admin_sessions))
        .route("/v1/admin/telemetry/summary", get(telemetry::summary))
        .route("/v1/admin/telemetry/series", get(telemetry::series))
        .route("/v1/admin/telemetry/runs", get(telemetry::runs))
        .route("/v1/admin/activity-signals", get(telemetry::signals))
}
fn private<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    response
}
pub(crate) async fn admin(gateway: &Gateway, headers: &HeaderMap) -> Result<Principal, Error> {
    let actor = gateway.principal(headers).await?;
    if actor.expires_at <= read::now_ms()? / 1000 {
        return Err(Error::Unauthenticated);
    }
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    gateway.ready().await?;
    Ok(actor)
}
async fn me(State(g): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    let principal = g.principal(&headers).await?;
    let actor = if principal.platform_admin {
        admin(&g, &headers).await?
    } else {
        g.authenticate_client(&headers).await?
    };
    Ok(private(Me {
        user: actor.user.to_string(),
        platform_admin: actor.platform_admin,
        expires_at: actor.expires_at,
        capabilities: [
            ("owner_inventory", true),
            ("admin_inventory", actor.platform_admin),
            ("session_stop", true),
            ("model_reporting", false),
            ("usage_reporting", false),
        ]
        .into_iter()
        .collect(),
    }))
}
async fn owner_overview(State(g): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    let actor = g.authenticate_client(&headers).await?;
    let mut snapshot = read::snapshot(&g, Some(actor.user)).await?;
    read::last_connections(&g, actor.user, &mut snapshot.machines).await?;
    Ok(private(snapshot))
}
async fn admin_overview(State(g): State<Gateway>, headers: HeaderMap) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    let mut snapshot = read::snapshot(&g, None).await?;
    read::last_connections(&g, actor.user, &mut snapshot.machines).await?;
    Ok(private(snapshot))
}
async fn admin_user(
    State(g): State<Gateway>,
    headers: HeaderMap,
    Path(user): Path<UserId>,
) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    let mut snapshot = read::snapshot(&g, Some(user)).await?;
    read::last_connections(&g, actor.user, &mut snapshot.machines).await?;
    if snapshot.users.is_empty() {
        return Err(Error::NotFound);
    }
    Ok(private(snapshot))
}
async fn admin_users(
    State(g): State<Gateway>,
    headers: HeaderMap,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    q.validate()?;
    let s = read::snapshot(&g, q.owner).await?;
    Ok(private(read::paginate(
        s.users,
        &q,
        |u| &u.id,
        s.observed_at_ms,
        s.coverage,
    )))
}
async fn owner_machines(
    State(g): State<Gateway>,
    headers: HeaderMap,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    let actor = g.authenticate_client(&headers).await?;
    q.validate()?;
    if q.owner.is_some_and(|u| u != actor.user) {
        return Err(Error::Forbidden);
    }
    let mut s = read::snapshot(&g, Some(actor.user)).await?;
    read::last_connections(&g, actor.user, &mut s.machines).await?;
    Ok(private(read::paginate(
        s.machines,
        &q,
        |m| &m.id,
        s.observed_at_ms,
        s.coverage,
    )))
}
async fn admin_machines(
    State(g): State<Gateway>,
    headers: HeaderMap,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    q.validate()?;
    let mut s = read::snapshot(&g, q.owner).await?;
    read::last_connections(&g, actor.user, &mut s.machines).await?;
    Ok(private(read::paginate(
        s.machines,
        &q,
        |m| &m.id,
        s.observed_at_ms,
        s.coverage,
    )))
}
async fn owner_sessions(
    State(g): State<Gateway>,
    headers: HeaderMap,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    let actor = g.authenticate_client(&headers).await?;
    q.validate()?;
    if q.owner.is_some_and(|u| u != actor.user) {
        return Err(Error::Forbidden);
    }
    sessions(&g, Some(actor.user), q).await
}
async fn admin_sessions(
    State(g): State<Gateway>,
    headers: HeaderMap,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    q.validate()?;
    sessions(&g, q.owner, q).await
}
async fn admin_user_devices(
    State(g): State<Gateway>,
    headers: HeaderMap,
    Path(user): Path<UserId>,
    ExtractQuery(mut q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    let actor = admin(&g, &headers).await?;
    q.validate()?;
    q.owner = Some(user);
    let mut s = read::snapshot(&g, Some(user)).await?;
    read::last_connections(&g, actor.user, &mut s.machines).await?;
    Ok(private(read::paginate(
        s.machines,
        &q,
        |m| &m.id,
        s.observed_at_ms,
        s.coverage,
    )))
}
async fn admin_user_sessions(
    State(g): State<Gateway>,
    headers: HeaderMap,
    Path(user): Path<UserId>,
    ExtractQuery(q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    q.validate()?;
    sessions(&g, Some(user), q).await
}
async fn admin_machine_sessions(
    State(g): State<Gateway>,
    headers: HeaderMap,
    Path(machine): Path<MachineId>,
    ExtractQuery(mut q): ExtractQuery<Query>,
) -> Result<Response, Error> {
    admin(&g, &headers).await?;
    q.validate()?;
    q.machine = Some(machine);
    sessions(&g, q.owner, q).await
}
async fn sessions(g: &Gateway, owner: Option<UserId>, q: Query) -> Result<Response, Error> {
    let mut s = read::snapshot(g, owner).await?;
    if let Some(machine) = q.machine {
        s.sessions.retain(|s| s.machine == machine.to_string());
    }
    Ok(private(read::paginate(
        s.sessions,
        &q,
        |s| &s.id,
        s.observed_at_ms,
        s.coverage,
    )))
}
