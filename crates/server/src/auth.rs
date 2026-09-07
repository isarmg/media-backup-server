use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use uuid::Uuid;

use crate::{error::AppError, routes::AppState};

pub async fn require_secure_transport(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !state.config.require_https {
        return Ok(next.run(request).await);
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip());
    if peer.is_some_and(|peer| {
        crate::trusted_proxy::forwarded_as_https(
            peer,
            request.headers(),
            &state.config.trusted_proxy_cidrs,
        )
    }) {
        return Ok(next.run(request).await);
    }
    Err(AppError::new(
        StatusCode::UPGRADE_REQUIRED,
        "HTTPS is required",
    ))
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub account_id: Uuid,
    pub device_id: Uuid,
    pub actor_kind: String,
    pub actor_id: Uuid,
}

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(AppError::unauthorized)?;
    let token_hash = Sha256::digest(token.as_bytes()).to_vec();
    let device: Option<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        SELECT d.account_id, d.id
        FROM devices d
        JOIN accounts a ON a.id = d.account_id
        WHERE d.token_hash = ? AND a.enabled = TRUE
        "#,
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await?;
    let (account_id, device_id, actor_kind, actor_id, usage) =
        if let Some((account_id, device_id)) = device {
            (
                account_id,
                device_id,
                "device".to_owned(),
                device_id,
                AuthUsage::Device(device_id),
            )
        } else {
            let api_key: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
                r#"
            SELECT k.account_id, k.device_id, k.id
            FROM api_keys k
            JOIN accounts a ON a.id = k.account_id
            WHERE k.token_hash = ? AND k.revoked_at IS NULL AND a.enabled = TRUE
            "#,
            )
            .bind(Sha256::digest(token.as_bytes()).to_vec())
            .fetch_optional(&state.pool)
            .await?;
            let (account_id, device_id, api_key_id) = api_key.ok_or_else(AppError::unauthorized)?;
            (
                account_id,
                device_id,
                "api_key".to_owned(),
                api_key_id,
                AuthUsage::ApiKey(api_key_id),
            )
        };
    request.extensions_mut().insert(AuthContext {
        account_id,
        device_id,
        actor_kind,
        actor_id,
    });
    let response = next.run(request).await;
    if response.status().is_success() {
        let update = match usage {
            AuthUsage::Device(device_id) => {
                sqlx::query(
                    "UPDATE devices SET last_seen_at = datetime('now') \
                     WHERE id = ? AND last_seen_at <= datetime('now', '-300 seconds')",
                )
                .bind(device_id)
                .execute(&state.pool)
                .await
            }
            AuthUsage::ApiKey(api_key_id) => {
                sqlx::query(
                    "UPDATE api_keys SET last_used_at = datetime('now') \
                     WHERE id = ? AND (last_used_at IS NULL \
                     OR last_used_at <= datetime('now', '-300 seconds'))",
                )
                .bind(api_key_id)
                .execute(&state.pool)
                .await
            }
        };
        if let Err(error) = update {
            tracing::warn!(?error, "failed to record successful API authentication use");
        }
    }
    Ok(response)
}

enum AuthUsage {
    Device(Uuid),
    ApiKey(Uuid),
}
