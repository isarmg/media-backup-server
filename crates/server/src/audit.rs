use uuid::Uuid;

use crate::{auth::AuthContext, error::AppError};

pub async fn record_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    auth: &AuthContext,
    action: &str,
    entity_kind: Option<&str>,
    entity_id: Option<Uuid>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO audit_events(
            account_id, actor_kind, actor_id, action, entity_kind, entity_id, occurred_at
        )
        VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
        "#,
    )
    .bind(auth.account_id)
    .bind(&auth.actor_kind)
    .bind(auth.actor_id)
    .bind(action)
    .bind(entity_kind)
    .bind(entity_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub async fn record_change(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    operation: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO account_changes(
            account_id, entity_kind, entity_id, operation, changed_at
        )
        VALUES (?, ?, ?, ?, datetime('now'))
        "#,
    )
    .bind(account_id)
    .bind(entity_kind)
    .bind(entity_id)
    .bind(operation)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
