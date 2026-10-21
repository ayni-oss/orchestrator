use super::Console;
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse as _, Response},
};

pub(super) async fn guard(
    State(console): State<Console>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let origin = format!("http://{}", console.authority);
    let valid = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        == Some(console.authority.as_str())
        && headers
            .get(header::ORIGIN)
            .is_none_or(|value| value.as_bytes() == origin.as_bytes())
        && headers
            .get("sec-fetch-site")
            .is_none_or(|value| matches!(value.to_str(), Ok("same-origin" | "none")))
        && (request.method() == axum::http::Method::GET
            || headers
                .get("x-console-request")
                .is_some_and(|value| value == "1"));
    if !valid {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"));
    headers.insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "referrer-policy",
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}
