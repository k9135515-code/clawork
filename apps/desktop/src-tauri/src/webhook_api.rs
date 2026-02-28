use crate::{
    ingest_inbound_messages, telegram_update_to_messages, verify_telegram_webhook,
    verify_whatsapp_webhook_signature, whatsapp_payload_to_messages, AppState, CommandError,
    GenericOk, TelegramWebhookUpdate, WhatsAppWebhookVerifyQuery,
};
use axum::body::Bytes;
use axum::extract::{Query, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use std::sync::Arc;

pub(crate) async fn api_telegram_webhook(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(update): Json<TelegramWebhookUpdate>,
) -> Result<Json<GenericOk>, CommandError> {
    verify_telegram_webhook(&headers)?;
    let count = ingest_inbound_messages(
        &state,
        "telegram-webhook",
        telegram_update_to_messages(update),
    )
    .await;
    Ok(Json(GenericOk {
        ok: true,
        message: format!("accepted {count} telegram message(s)"),
    }))
}

pub(crate) async fn api_whatsapp_webhook_verify(
    Query(query): Query<WhatsAppWebhookVerifyQuery>,
) -> Result<Response, CommandError> {
    let expected = std::env::var("CLAWORK_WHATSAPP_VERIFY_TOKEN")
        .map_err(|_| CommandError::not_configured("CLAWORK_WHATSAPP_VERIFY_TOKEN is required"))?;
    if expected.trim().is_empty() {
        return Err(CommandError::not_configured(
            "CLAWORK_WHATSAPP_VERIFY_TOKEN is empty",
        ));
    }

    let is_valid = query.mode.as_deref() == Some("subscribe")
        && query.verify_token.as_deref() == Some(expected.as_str());
    if is_valid {
        let challenge = query.challenge.unwrap_or_default();
        Ok((StatusCode::OK, challenge).into_response())
    } else {
        Err(CommandError::denied("whatsapp webhook verification failed"))
    }
}

pub(crate) async fn api_whatsapp_webhook_event(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<GenericOk>, CommandError> {
    verify_whatsapp_webhook_signature(&headers, &body)?;
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|e| CommandError::validation(format!("invalid webhook JSON: {e}")))?;
    let inbound = whatsapp_payload_to_messages(&payload);
    let count = ingest_inbound_messages(&state, "whatsapp-webhook", inbound).await;
    Ok(Json(GenericOk {
        ok: true,
        message: format!("accepted {count} whatsapp message(s)"),
    }))
}
