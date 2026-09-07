use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use sqlx::Row;

use crate::{error::AppError, routes::AppState};

pub async fn prometheus(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let expected = state
        .config
        .metrics_token
        .as_deref()
        .ok_or_else(|| AppError::not_found("metrics endpoint is disabled"))?;
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if supplied != Some(expected) {
        return Err(AppError::unauthorized());
    }
    let row = sqlx::query(
        r#"
        SELECT
          (SELECT COUNT(*) FROM accounts WHERE enabled) AS accounts,
          (SELECT COUNT(*) FROM devices) AS devices,
          (SELECT COUNT(*) FROM assets WHERE deleted_at IS NULL) AS assets,
          (SELECT COUNT(*) FROM assets WHERE deleted_at IS NOT NULL) AS trashed_assets,
          (SELECT COUNT(*) FROM uploads WHERE state = 'uploading') AS active_uploads,
          (SELECT COALESCE(SUM(stored_size), 0) FROM blobs) AS stored_bytes,
          (SELECT COUNT(*) FROM api_keys WHERE revoked_at IS NULL) AS api_keys
        "#,
    )
    .fetch_one(&state.pool)
    .await?;
    let body = format!(
        concat!(
            "# HELP media_backup_accounts Enabled accounts.\n",
            "# TYPE media_backup_accounts gauge\nmedia_backup_accounts {}\n",
            "# TYPE media_backup_devices gauge\nmedia_backup_devices {}\n",
            "# TYPE media_backup_assets gauge\nmedia_backup_assets {}\n",
            "# TYPE media_backup_trashed_assets gauge\nmedia_backup_trashed_assets {}\n",
            "# TYPE media_backup_active_uploads gauge\nmedia_backup_active_uploads {}\n",
            "# TYPE media_backup_stored_bytes gauge\nmedia_backup_stored_bytes {}\n",
            "# TYPE media_backup_api_keys gauge\nmedia_backup_api_keys {}\n"
        ),
        row.get::<i64, _>("accounts"),
        row.get::<i64, _>("devices"),
        row.get::<i64, _>("assets"),
        row.get::<i64, _>("trashed_assets"),
        row.get::<i64, _>("active_uploads"),
        row.get::<i64, _>("stored_bytes"),
        row.get::<i64, _>("api_keys"),
    );
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    Ok(response)
}
