use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const API_VERSION: &str = "v2";
pub const API_BASE_PATH: &str = "/v2";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRequest {
    pub username: String,
    pub password: String,
    pub device_name: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapResponse {
    pub account_id: Uuid,
    pub device_id: Uuid,
    pub bearer_token: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Video,
    Other,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StorageEncoding {
    PlainV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadPartSpec {
    pub index: u32,
    pub size: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUploadRequest {
    pub source_asset_id: String,
    pub source_resource_id: String,
    pub media_kind: MediaKind,
    pub role: String,
    pub filename: String,
    pub mime_type: String,
    pub source_created_at_ms: i64,
    pub storage_encoding: StorageEncoding,
    pub content_size: u64,
    pub content_blake3: String,
    pub metadata: Option<serde_json::Value>,
    pub parts: Vec<UploadPartSpec>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UploadDisposition {
    Upload,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUploadResponse {
    pub disposition: UploadDisposition,
    pub upload_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub missing_parts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadStatusResponse {
    pub upload_id: Uuid,
    pub state: String,
    pub missing_parts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteUploadResponse {
    pub resource_id: Uuid,
    pub asset_id: Uuid,
    pub deduplicated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSummary {
    pub resource_id: Uuid,
    pub role: String,
    pub filename: String,
    pub mime_type: String,
    pub content_size: u64,
    pub storage_encoding: StorageEncoding,
    pub metadata: Option<serde_json::Value>,
    pub manifest_path: String,
    pub content_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSummary {
    pub asset_id: Uuid,
    pub source_asset_id: String,
    pub media_kind: MediaKind,
    pub source_created_at_ms: i64,
    pub favorite: bool,
    pub archived: bool,
    pub trashed_at_ms: Option<i64>,
    pub tag_names: Vec<String>,
    pub resources: Vec<ResourceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelinePage {
    pub items: Vec<AssetSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncEvent {
    pub sequence: i64,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub operation: String,
    pub changed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncPage {
    pub events: Vec<SyncEvent>,
    pub next_sequence: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlbumRecord {
    pub album_id: Uuid,
    pub source_album_id: String,
    pub name: String,
    pub asset_count: u64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncAlbumRequest {
    pub source_album_id: String,
    pub name: String,
    pub source_asset_ids: Vec<String>,
    pub replace_members: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateAssetRequest {
    pub favorite: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagRecord {
    pub tag_id: Uuid,
    pub name: String,
    pub asset_count: u64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTagRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTagAssetsRequest {
    pub asset_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuplicateGroup {
    pub content_blake3: String,
    pub content_size: u64,
    pub assets: Vec<AssetSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateApiKeyRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyCreated {
    pub api_key_id: Uuid,
    pub name: String,
    pub token: String,
    pub prefix: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyRecord {
    pub api_key_id: Uuid,
    pub name: String,
    pub prefix: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    pub sequence: i64,
    pub actor_kind: String,
    pub action: String,
    pub entity_kind: Option<String>,
    pub entity_id: Option<Uuid>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPage {
    pub events: Vec<AuditEvent>,
    pub next_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceManifest {
    pub resource_id: Uuid,
    pub asset_id: Uuid,
    pub source_asset_id: String,
    pub source_resource_id: String,
    pub media_kind: MediaKind,
    pub role: String,
    pub filename: String,
    pub mime_type: String,
    pub source_created_at_ms: i64,
    pub content_size: u64,
    pub content_blake3: String,
    pub storage_encoding: StorageEncoding,
    pub metadata: Option<serde_json::Value>,
    pub parts: Vec<UploadPartSpec>,
    pub content_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_action_request_is_an_exact_dto() {
        assert_eq!(API_BASE_PATH, format!("/{API_VERSION}"));
        assert!(serde_json::from_str::<EmptyRequest>("{}").is_ok());
        assert!(serde_json::from_str::<EmptyRequest>(r#"{"unknown_field":true}"#).is_err());
        assert!(serde_json::from_str::<EmptyRequest>("").is_err());
    }

    #[test]
    fn upload_manifest_explicitly_describes_plain_v1_content() {
        let request = CreateUploadRequest {
            source_asset_id: "asset".to_owned(),
            source_resource_id: "resource".to_owned(),
            media_kind: MediaKind::Photo,
            role: "primary".to_owned(),
            filename: "photo.jpg".to_owned(),
            mime_type: "image/jpeg".to_owned(),
            source_created_at_ms: 1,
            storage_encoding: StorageEncoding::PlainV1,
            content_size: 3,
            content_blake3: "0".repeat(64),
            metadata: Some(serde_json::json!({"width": 10})),
            parts: vec![UploadPartSpec {
                index: 0,
                size: 3,
                blake3: "0".repeat(64),
            }],
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["storage_encoding"], "plain-v1");
        assert_eq!(value["content_size"], 3);
    }

    #[test]
    fn current_upload_protocol_rejects_unsupported_or_ambiguous_shapes() {
        let current = serde_json::json!({
            "source_asset_id": "asset",
            "source_resource_id": "resource",
            "media_kind": "photo",
            "role": "primary",
            "filename": "photo.jpg",
            "mime_type": "image/jpeg",
            "source_created_at_ms": 1,
            "storage_encoding": "plain-v1",
            "content_size": 3,
            "content_blake3": "0".repeat(64),
            "metadata": null,
            "parts": [{"index": 0, "size": 3, "blake3": "0".repeat(64)}]
        });
        assert!(serde_json::from_value::<CreateUploadRequest>(current.clone()).is_ok());

        let mut missing_encoding = current.clone();
        missing_encoding
            .as_object_mut()
            .unwrap()
            .remove("storage_encoding");
        assert!(serde_json::from_value::<CreateUploadRequest>(missing_encoding).is_err());

        let mut unsupported_encoding = current.clone();
        unsupported_encoding["storage_encoding"] = serde_json::json!("not-current");
        assert!(serde_json::from_value::<CreateUploadRequest>(unsupported_encoding).is_err());

        let mut unknown_field = current;
        unknown_field["unexpected_field"] = serde_json::json!("must-not-be-accepted");
        assert!(serde_json::from_value::<CreateUploadRequest>(unknown_field).is_err());
    }
}
