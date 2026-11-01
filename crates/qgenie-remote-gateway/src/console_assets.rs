//! Compile-time console assets. Paths never resolve arbitrary filesystem content.
use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};

pub(crate) fn routes() -> Router<crate::Gateway> {
    Router::new()
        .route(
            "/console",
            get(|| async { Redirect::permanent("/console/") }),
        )
        .route("/console/", get(index))
        .route("/console/index.html", get(index))
        .route("/console/assets/{name}", get(asset))
}
fn response(value: &'static str, content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(value));
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"));
    response
}
async fn index() -> Response {
    response(
        include_str!("../../../apps/qgenie-console/index.html"),
        "text/html; charset=utf-8",
    )
}
async fn asset(Path(name): Path<String>) -> Response {
    let (body, content_type) = match name.as_str() {
        "base.css" => (
            include_str!("../../../apps/qgenie-console/assets/base.css"),
            "text/css; charset=utf-8",
        ),
        "shell.css" => (
            include_str!("../../../apps/qgenie-console/assets/shell.css"),
            "text/css; charset=utf-8",
        ),
        "admin-visual.css" => (
            include_str!("../../../apps/qgenie-console/assets/admin-visual.css"),
            "text/css; charset=utf-8",
        ),
        "telemetry-visual.css" => (
            include_str!("../../../apps/qgenie-console/assets/telemetry-visual.css"),
            "text/css; charset=utf-8",
        ),
        "user-visual.css" => (
            include_str!("../../../apps/qgenie-console/assets/user-visual.css"),
            "text/css; charset=utf-8",
        ),
        "app.css" => (
            include_str!("../../../apps/qgenie-console/assets/app.css"),
            "text/css; charset=utf-8",
        ),
        "app.js" => (
            include_str!("../../../apps/qgenie-console/assets/app.js"),
            "text/javascript; charset=utf-8",
        ),
        "wire.js" => (
            include_str!("../../../apps/qgenie-console/assets/wire.js"),
            "text/javascript; charset=utf-8",
        ),
        "views.js" => (
            include_str!("../../../apps/qgenie-console/assets/views.js"),
            "text/javascript; charset=utf-8",
        ),
        "boot.js" => (
            include_str!("../../../apps/qgenie-console/assets/boot.js"),
            "text/javascript; charset=utf-8",
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    response(body, content_type)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn assets_are_allowlisted_and_scripts_cannot_execute_inline() {
        let page = index().await;
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CACHE_CONTROL], "private, no-store");
        assert!(
            page.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("script-src 'self';")
        );
        assert_eq!(asset(Path("app.js".into())).await.status(), StatusCode::OK);
        assert_eq!(
            asset(Path("../../Cargo.toml".into())).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
