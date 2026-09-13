use std::sync::Arc;

use sarmg_secret::{SecretBytes, SecretKey};
use sarmg_secret_envelope::EnvelopeDomain;

use crate::error::AppError;

struct ClientAuthorizationEnvelope;

impl EnvelopeDomain for ClientAuthorizationEnvelope {
    const DOMAIN: &'static [u8] = b"media-backup/client-authorization";
    const REVISION: u16 = 1;
}

#[derive(Clone)]
pub struct SecretBox {
    master: Arc<SecretKey<32>>,
}

impl SecretBox {
    pub fn new(master: &[u8; 32]) -> Self {
        Self {
            master: Arc::new(SecretKey::new(*master)),
        }
    }

    pub fn encrypt_client_authorization(
        &self,
        instance_id: &str,
        value: &str,
    ) -> Result<Vec<u8>, AppError> {
        sarmg_secret_envelope::seal::<ClientAuthorizationEnvelope>(
            &self.master,
            instance_id.as_bytes(),
            &SecretBytes::new(value.as_bytes().to_vec()),
        )
        .map_err(|_| {
            AppError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "authorization encryption failed",
            )
        })
    }

    pub fn decrypt_client_authorization(
        &self,
        instance_id: &str,
        encoded: &[u8],
    ) -> Result<String, AppError> {
        let value = sarmg_secret_envelope::open::<ClientAuthorizationEnvelope>(
            &self.master,
            instance_id.as_bytes(),
            encoded,
        )
        .map_err(|_| {
            AppError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "authorization decryption failed",
            )
        })?;
        String::from_utf8(value.expose().to_vec()).map_err(|_| {
            AppError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "authorization plaintext is invalid",
            )
        })
    }
}
