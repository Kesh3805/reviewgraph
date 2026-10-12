//! HTTP errors of the internal API.
//!
//! Bodies are `application/problem+json`-style objects whose `detail` the TS `EngineClient`
//! forwards for 400s; 401/403 bodies stay generic so a misconfigured key is not an oracle.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::backend::BackendError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(&'static str),
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    /// The snapshot is still loading after the wait budget: retry in `Retry-After` seconds.
    #[error("the graph is loading")]
    Loading,
    #[error("unavailable: {0}")]
    Unavailable(String),
}

impl From<BackendError> for ApiError {
    fn from(error: BackendError) -> Self {
        match error {
            BackendError::NotFound => Self::NotFound,
            BackendError::Unavailable(detail) => Self::Unavailable(detail),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title, detail) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden", None),
            Self::NotFound => (StatusCode::NOT_FOUND, "not found", None),
            Self::BadRequest(detail) => {
                (StatusCode::BAD_REQUEST, "bad request", Some(detail.clone()))
            }
            Self::Loading => (
                StatusCode::SERVICE_UNAVAILABLE,
                "the graph is loading",
                None,
            ),
            Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", None),
        };
        if let Self::Unavailable(detail) = &self {
            tracing::error!(detail = %detail, "engine backend unavailable");
        }
        let body = serde_json::json!({
            "type": "about:blank",
            "title": title,
            "status": status.as_u16(),
            "detail": detail,
        });
        let mut response = (status, Json(body)).into_response();
        if matches!(self, Self::Loading) {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
        }
        response
    }
}
