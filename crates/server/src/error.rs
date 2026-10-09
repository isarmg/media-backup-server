use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::fmt;
use xcss_error::{ErrorCode, ErrorEnvelope, HttpStatus as FoundationHttpStatus};

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    envelope: Box<ErrorEnvelope>,
    retry_after: Option<u64>,
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.envelope.message)
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            status,
            envelope: Box::new(error_envelope(status, message)),
            retry_after: None,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.envelope.message
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn upload_superseded() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            envelope: Box::new(
                ErrorEnvelope::with_code(
                    ErrorCode::new("upload_superseded")
                        .expect("built-in upload superseded code is valid"),
                    "upload was superseded by a newer committed resource",
                )
                .retryable(false),
            ),
            retry_after: None,
        }
    }

    pub fn too_many_requests(retry_after: u64) -> Self {
        Self::too_many_requests_with_message(retry_after, "too many login attempts")
    }

    pub fn too_many_requests_with_message(retry_after: u64, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            envelope: Box::new(ErrorEnvelope::new(
                FoundationHttpStatus::TooManyRequests,
                message,
            )),
            retry_after: Some(retry_after.max(1)),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(*self.envelope)).into_response();
        if let Some(retry_after) = self.retry_after
            && let Ok(value) = retry_after.to_string().parse()
        {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, private, max-age=0"),
        );
        response
    }
}

fn error_envelope(status: StatusCode, message: String) -> ErrorEnvelope {
    let foundation_status = match status {
        StatusCode::BAD_REQUEST => Some(FoundationHttpStatus::BadRequest),
        StatusCode::UNAUTHORIZED => Some(FoundationHttpStatus::Unauthorized),
        StatusCode::FORBIDDEN => Some(FoundationHttpStatus::Forbidden),
        StatusCode::NOT_FOUND => Some(FoundationHttpStatus::NotFound),
        StatusCode::CONFLICT => Some(FoundationHttpStatus::Conflict),
        StatusCode::UNPROCESSABLE_ENTITY => Some(FoundationHttpStatus::UnprocessableEntity),
        StatusCode::TOO_MANY_REQUESTS => Some(FoundationHttpStatus::TooManyRequests),
        StatusCode::INTERNAL_SERVER_ERROR => Some(FoundationHttpStatus::Internal),
        StatusCode::SERVICE_UNAVAILABLE => Some(FoundationHttpStatus::ServiceUnavailable),
        _ => None,
    };
    if let Some(status) = foundation_status {
        return ErrorEnvelope::new(status, message);
    }

    let (code, retryable) = match status {
        StatusCode::METHOD_NOT_ALLOWED => ("method_not_allowed", false),
        StatusCode::NOT_ACCEPTABLE => ("not_acceptable", false),
        StatusCode::PAYLOAD_TOO_LARGE => ("payload_too_large", false),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ("unsupported_media_type", false),
        StatusCode::UPGRADE_REQUIRED => ("secure_transport_required", false),
        StatusCode::REQUEST_TIMEOUT => ("request_timeout", true),
        _ if status.is_server_error() => ("server_error", true),
        _ => ("request_failed", false),
    };
    ErrorEnvelope::with_code(
        ErrorCode::new(code).expect("built-in Media Backup error code must be valid"),
        message,
    )
    .retryable(retryable)
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(?error, "database error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "database error")
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        tracing::error!(?error, "storage error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::bad_request(format!("invalid JSON: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::header};

    use super::*;

    #[tokio::test]
    async fn responses_use_the_exact_foundation_error_envelope() {
        let response = AppError::bad_request("invalid input").into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "code": "bad_request",
                "message": "invalid input",
                "retryable": false
            })
        );
    }

    #[tokio::test]
    async fn rate_limits_are_retryable_and_keep_retry_after() {
        let response =
            AppError::too_many_requests_with_message(7, "capacity is busy").into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "7");
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
        assert_eq!(value["code"], "too_many_requests");
        assert_eq!(value["retryable"], true);
        assert!(value.get("error").is_none());
    }
}
