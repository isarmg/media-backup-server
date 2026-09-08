use crate::{auth::AuthContext, error::AppError, routes::AppState};
use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

fn range(value: &str, size: u64) -> Result<(u64, u64), ()> {
    let value = value.strip_prefix("bytes=").ok_or(())?;
    if value.contains(',') || size == 0 {
        return Err(());
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let length = suffix.min(size);
        return Ok((size - length, length));
    }
    let start: u64 = start.parse().map_err(|_| ())?;
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(size - 1)
    };
    if start >= size || end < start {
        return Err(());
    }
    Ok((start, end - start + 1))
}

pub async fn content(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let row=sqlx::query("SELECT ac.storage_path AS account_path,b.storage_path,b.stored_size,b.content_blake3,r.mime_type
        FROM resources r JOIN assets a ON a.id=r.asset_id JOIN accounts ac ON ac.id=a.account_id JOIN blobs b ON b.id=r.blob_id
        WHERE r.id=? AND a.account_id=?").bind(id).bind(auth.account_id).fetch_optional(&state.pool).await?
        .ok_or_else(||AppError::not_found("resource not found"))?;
    let mut file = state
        .storage
        .open_blob(row.get("account_path"), row.get("storage_path"))
        .await?;
    let size = file.metadata().await?.len();
    if size != row.get::<i64, _>("stored_size") as u64 {
        return Err(AppError::conflict("stored resource size mismatch"));
    }
    let etag = format!("\"{}\"", row.get::<String, _>("content_blake3"));
    let mut response = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|part| part.trim() == "*" || part.trim().trim_start_matches("W/") == etag)
        }) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let requested = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
        let honor = headers
            .get(header::IF_RANGE)
            .is_none_or(|v| v.to_str().ok() == Some(etag.as_str()));
        let selected = if honor {
            requested.map(|v| range(v, size))
        } else {
            None
        };
        match selected {
            Some(Err(())) => {
                let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                r.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{size}")).unwrap(),
                );
                r
            }
            value => {
                let (start, length) = value.and_then(Result::ok).unwrap_or((0, size));
                file.seek(std::io::SeekFrom::Start(start)).await?;
                let mut r = Body::from_stream(ReaderStream::new(file.take(length))).into_response();
                if value.is_some() {
                    *r.status_mut() = StatusCode::PARTIAL_CONTENT;
                    r.headers_mut().insert(
                        header::CONTENT_RANGE,
                        HeaderValue::from_str(&format!(
                            "bytes {start}-{}/{size}",
                            start + length - 1
                        ))
                        .unwrap(),
                    );
                }
                r.headers_mut().insert(
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(&length.to_string()).unwrap(),
                );
                r
            }
        }
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(|_| AppError::conflict("invalid content hash"))?,
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&row.get::<String, _>("mime_type"))
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        "x-media-backup-storage-encoding",
        HeaderValue::from_static("plain-v1"),
    );
    Ok(response)
}

static PREVIEW_WORK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
struct CachedPreview {
    key: String,
    bytes: Vec<u8>,
}
static PREVIEWS: std::sync::Mutex<Vec<CachedPreview>> = std::sync::Mutex::new(Vec::new());

pub async fn preview(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let row=sqlx::query("SELECT ac.storage_path AS account_path,b.storage_path,b.content_blake3,r.mime_type
        FROM resources r JOIN assets a ON a.id=r.asset_id JOIN accounts ac ON ac.id=a.account_id JOIN blobs b ON b.id=r.blob_id
        WHERE r.id=? AND a.account_id=?").bind(id).bind(auth.account_id).fetch_optional(&state.pool).await?
        .ok_or_else(||AppError::not_found("resource not found"))?;
    if !row.get::<String, _>("mime_type").starts_with("image/") {
        return Err(AppError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "preview requires an image",
        ));
    }
    let key = format!(
        "{}:{}:preview1600-v1",
        auth.account_id,
        row.get::<String, _>("content_blake3")
    );
    let etag = format!("\"{key}\"");
    let mut response = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(&etag)
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let _permit = PREVIEW_WORK
            .acquire()
            .await
            .map_err(|_| AppError::conflict("preview worker unavailable"))?;
        let cached = {
            let cache = PREVIEWS.lock().unwrap_or_else(|p| p.into_inner());
            cache.iter().find(|c| c.key == key).map(|c| c.bytes.clone())
        };
        let bytes = if let Some(bytes) = cached {
            bytes
        } else {
            let file = state
                .storage
                .open_blob(row.get("account_path"), row.get("storage_path"))
                .await?
                .into_std()
                .await;
            let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
                use image::ImageDecoder;
                let mut reader = image::ImageReader::new(std::io::BufReader::new(file))
                    .with_guessed_format()
                    .map_err(|e| e.to_string())?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(256 * 1024 * 1024);
                limits.max_image_width = Some(20000);
                limits.max_image_height = Some(20000);
                reader.limits(limits);
                let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
                let orientation = decoder.orientation().map_err(|e| e.to_string())?;
                let mut decoded =
                    image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
                decoded.apply_orientation(orientation);
                let small = decoded.thumbnail(1600, 1600).to_rgb8();
                let mut bytes = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85)
                    .encode_image(&small)
                    .map_err(|e| e.to_string())?;
                Ok(bytes)
            })
            .await
            .map_err(|_| AppError::conflict("preview worker failed"))?
            .map_err(|_| {
                AppError::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "image format or dimensions unsupported; use original",
                )
            })?;
            let mut cache = PREVIEWS.lock().unwrap_or_else(|p| p.into_inner());
            cache.push(CachedPreview {
                key: key.clone(),
                bytes: bytes.clone(),
            });
            while cache.iter().map(|c| c.bytes.len()).sum::<usize>() > 64 * 1024 * 1024 {
                cache.remove(0);
            }
            bytes
        };
        ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(|_| AppError::conflict("invalid preview hash"))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_ranges_cover_seek_suffix_and_rejection() {
        assert_eq!(range("bytes=2-5", 10), Ok((2, 4)));
        assert_eq!(range("bytes=6-", 10), Ok((6, 4)));
        assert_eq!(range("bytes=-3", 10), Ok((7, 3)));
        assert_eq!(range("bytes=0-99", 10), Ok((0, 10)));
        for value in [
            "bytes=10-",
            "bytes=5-2",
            "bytes=-0",
            "bytes=0-1,3-4",
            "bytes=x-y",
        ] {
            assert!(range(value, 10).is_err());
        }
        assert!(range("bytes=0-", 0).is_err());
    }
}
