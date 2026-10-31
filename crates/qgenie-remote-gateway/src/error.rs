use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("required native capability is unavailable")]
    Unsupported,
    #[error("invalid gateway configuration or unreadable secret file")]
    Configuration,
    #[error("routing transport unavailable or delivery uncertain")]
    Unavailable,
    #[error("PostgreSQL authority unavailable")]
    Database,
    #[error("database schema is incompatible or migration checksum differs")]
    Schema,
    #[error("database connection uses an unsafe or incorrect application role")]
    Role,
    #[error("authentication required")]
    Unauthenticated,
    #[error("operation forbidden")]
    Forbidden,
    #[error("gateway is draining")]
    Draining,
    #[error("identity conflicts with existing enrollment")]
    Conflict,
    #[error("session name is ambiguous; use a stable session ID")]
    AmbiguousName,
    #[error("another controller holds the session; confirm takeover")]
    ControlHeld,
    #[error("controller proposal or generation is stale")]
    StaleGeneration,
    #[error("resource not found")]
    NotFound,
    #[error("historical cursor belongs to an earlier recovery epoch")]
    HistoryTruncated,
    #[error("request capacity exceeded")]
    Overloaded,
    #[error("invalid or oversized request")]
    Invalid,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unsupported => (StatusCode::NOT_IMPLEMENTED, "unsupported_capability"),
            Self::ControlHeld => (StatusCode::CONFLICT, "control_held"),
            Self::StaleGeneration => (StatusCode::CONFLICT, "stale_generation"),
            Self::Conflict => (StatusCode::CONFLICT, "identity_conflict"),
            Self::AmbiguousName => (StatusCode::CONFLICT, "ambiguous_name"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::HistoryTruncated => (StatusCode::CONFLICT, "history_truncated"),
            Self::Overloaded => (StatusCode::TOO_MANY_REQUESTS, "overloaded"),
            Self::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::Configuration
            | Self::Database
            | Self::Schema
            | Self::Role
            | Self::Draining
            | Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        };
        let public_code = match self {
            Self::Unsupported => qgenie_remote_proto::ErrorCode::UnsupportedCapability,
            Self::ControlHeld => qgenie_remote_proto::ErrorCode::ControlHeld,
            Self::StaleGeneration => qgenie_remote_proto::ErrorCode::StaleGeneration,
            Self::Conflict => qgenie_remote_proto::ErrorCode::IdempotencyConflict,
            Self::AmbiguousName => qgenie_remote_proto::ErrorCode::AmbiguousName,
            Self::NotFound => qgenie_remote_proto::ErrorCode::NotFound,
            Self::HistoryTruncated => qgenie_remote_proto::ErrorCode::HistoryTruncated,
            Self::Overloaded => qgenie_remote_proto::ErrorCode::Overloaded,
            Self::Unauthenticated => qgenie_remote_proto::ErrorCode::Unauthenticated,
            Self::Forbidden => qgenie_remote_proto::ErrorCode::Forbidden,
            Self::Invalid => qgenie_remote_proto::ErrorCode::InvalidFrame,
            _ => qgenie_remote_proto::ErrorCode::AccessStale,
        };
        let error = qgenie_remote_proto::RemoteError {
            code: public_code,
            outcome: if matches!(self, Self::Unsupported) {
                qgenie_remote_proto::OperationOutcome::NotSent
            } else if matches!(self, Self::Database | Self::Unavailable) {
                qgenie_remote_proto::OperationOutcome::Unknown
            } else {
                qgenie_remote_proto::OperationOutcome::Rejected
            },
            message: code.into(),
            retryable: false,
        };
        (status, Json(serde_json::json!({"error":error}))).into_response()
    }
}
