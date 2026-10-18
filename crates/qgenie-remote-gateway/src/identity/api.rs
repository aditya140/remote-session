use crate::{Error, Gateway, IdentityService};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use qgenie_remote_proto::{
    MachineId, decode_json,
    rest::{
        Challenge, ChallengeRequest, Enrolled, EnrollmentRequest, MachinePatch, MachineToken, Proof,
    },
    wire::Machine,
};
fn identity(state: &Gateway) -> Result<&IdentityService, Error> {
    state.identity.as_deref().ok_or(Error::Configuration)
}
pub(crate) async fn enrollment_challenge(
    State(state): State<Gateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Challenge>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    let request: EnrollmentRequest = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(Json(
        identity(&state)?
            .enrollment_challenge(actor.user, request)
            .await?,
    ))
}
pub(crate) async fn enroll(
    State(state): State<Gateway>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Enrolled>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    Ok(Json(
        identity(&state)?
            .enroll(actor.user, decode_json(&body).map_err(|_| Error::Invalid)?)
            .await?,
    ))
}
pub(crate) async fn machines(
    State(state): State<Gateway>,
    headers: HeaderMap,
) -> Result<Json<Vec<Machine>>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    Ok(Json(if let Some(routing) = &state.routing {
        routing.machines(actor.user).await?
    } else {
        identity(&state)?.machines(actor.user).await?
    }))
}
pub(crate) async fn patch(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Path(machine): Path<String>,
    body: Bytes,
) -> Result<Json<Machine>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    let id: MachineId = machine.parse().map_err(|_| Error::Invalid)?;
    let patch: MachinePatch = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(Json(identity(&state)?.patch(actor.user, id, patch).await?))
}
pub(crate) async fn machine(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Path(machine): Path<String>,
) -> Result<Json<Machine>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    Ok(Json(
        identity(&state)?
            .machine(actor.user, machine.parse().map_err(|_| Error::Invalid)?)
            .await?,
    ))
}
pub(crate) async fn challenge(
    State(state): State<Gateway>,
    Path(machine): Path<String>,
    body: Bytes,
) -> Result<Json<Challenge>, Error> {
    state.ready().await?;
    let request: ChallengeRequest = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(Json(
        identity(&state)?
            .machine_challenge(machine.parse().map_err(|_| Error::Invalid)?, request.key)
            .await?,
    ))
}
pub(crate) async fn token(
    State(state): State<Gateway>,
    body: Bytes,
) -> Result<Json<MachineToken>, Error> {
    state.ready().await?;
    let proof: Proof = decode_json(&body).map_err(|_| Error::Invalid)?;
    Ok(Json(identity(&state)?.machine_token(proof).await?))
}
pub(crate) async fn revoke(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Path(machine): Path<String>,
    body: Bytes,
) -> Result<Json<qgenie_remote_proto::rest::Revoked>, Error> {
    let actor = state.principal(&headers).await?;
    state.ready().await?;
    let request: qgenie_remote_proto::rest::MachineRevoke =
        decode_json(&body).map_err(|_| Error::Invalid)?;
    if !request.confirmed {
        return Err(Error::Invalid);
    }
    let machine = machine.parse().map_err(|_| Error::Invalid)?;
    identity(&state)?.revoke(actor.user, machine).await?;
    if let Some(routing) = &state.routing {
        routing.fence(&qgenie_remote_proto::rest::RevocationTarget::Machine { machine });
    }
    Ok(Json(qgenie_remote_proto::rest::Revoked { machine }))
}
