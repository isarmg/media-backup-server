// Generated from sarmg-foundation-server. Update via sync-platform-router.mjs.
use axum::{
    extract::{DefaultBodyLimit, Request, State as AxumState},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::FutureExt;
use sarmg_server_runtime::{
    new_request_id, RuntimeHandle, DEFAULT_REQUEST_BODY_BYTES, LIVENESS_PATH, READINESS_PATH,
};
use std::sync::Arc;
#[derive(Clone)]
struct PlatformState {
    handle: RuntimeHandle,
}

/// Compose the Foundation-owned auth, health, readiness and request-id routes.
/// No administrator diagnostics endpoint is provided. Product routes must be merged separately.
pub fn platform_router<Store>(
    handle: RuntimeHandle,
    product_id: impl Into<String>,
    mode: sarmg_admin_auth::AdministratorOriginMode,
    administrator: Arc<sarmg_admin_core::AdministratorService<Store>>,
) -> Result<Router, sarmg_admin_core::Error>
where
    Store: sarmg_admin_core::AdministratorStore + 'static,
{
    let product_id = product_id.into();
    let auth = sarmg_admin_axum::administrator_router(
        product_id.clone(),
        mode,
        Arc::clone(&administrator),
    )?;
    let state = PlatformState { handle };
    let runtime = Router::new()
        .route(LIVENESS_PATH, get(liveness))
        .route(READINESS_PATH, get(readiness))
        .with_state(state);
    Ok(Router::new()
        .merge(auth)
        .merge(runtime)
        .layer(DefaultBodyLimit::max(DEFAULT_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn(request_id_layer)))
}

async fn liveness(AxumState(state): AxumState<PlatformState>) -> StatusCode {
    if state.handle.health().await.live {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn readiness(AxumState(state): AxumState<PlatformState>) -> Response {
    let ready = state.handle.health().await.ready;
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"ready": ready})),
    )
        .into_response()
}

fn request_error(status: StatusCode, code: &'static str, request_id: &str) -> Response {
    let mut response = (
        status,
        Json(serde_json::json!({
            "code": code,
            "message": code.replace('.', " "),
            "request_id": request_id,
            "retryable": false,
            "details": {},
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private, max-age=0"),
    );
    response
        .extensions_mut()
        .insert(sarmg_admin_axum::FoundationErrorResponse);
    response
}

async fn request_id_layer(mut request: Request, next: Next) -> Response {
    static HEADER: axum::http::HeaderName = axum::http::HeaderName::from_static("x-request-id");
    let request_id = match request
        .headers()
        .get_all(&HEADER)
        .iter()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => new_request_id(),
        [value] => match value.to_str() {
            Ok(value) if sarmg_contracts::RequestId::new(value).is_ok() => value.to_owned(),
            _ => {
                return request_error(
                    StatusCode::BAD_REQUEST,
                    "request.invalid_id",
                    &new_request_id(),
                );
            }
        },
        _ => {
            return request_error(
                StatusCode::BAD_REQUEST,
                "request.duplicate_id",
                &new_request_id(),
            );
        }
    };
    request.extensions_mut().insert(request_id.clone());
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        request.headers_mut().insert(HEADER.clone(), value);
    }
    let mut response = match std::panic::AssertUnwindSafe(next.run(request))
        .catch_unwind()
        .await
    {
        Ok(response) => response,
        Err(_) => request_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "platform.internal",
            &request_id,
        ),
    };
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(HEADER.clone(), value);
    }
    response
}
