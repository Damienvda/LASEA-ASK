use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("provider request failed: {0}")]
    Provider(String),
    #[error("bad request: {0}")]
    BadRequest(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::UnknownProvider(_) | AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Provider(_) => StatusCode::BAD_GATEWAY,
        };
        let body = Json(json!({ "error": self.to_string() }));
        (status, body).into_response()
    }
}
