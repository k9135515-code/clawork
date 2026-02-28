use crate::local_auth::require_auth;
use crate::{
    connectors_google_drive_create_inner, connectors_google_sheets_append_inner,
    connectors_notion_page_create_inner, connectors_notion_search_inner,
    connectors_oauth_callback_inner, connectors_oauth_start_inner, connectors_slack_history_inner,
    connectors_slack_post_inner, connectors_status_inner, AppState, CommandError,
    ConnectorOAuthCallbackReq, ConnectorOAuthStartReq, GoogleDriveCreateReq,
    GoogleDriveCreateResult, GoogleSheetsAppendReq, GoogleSheetsAppendResult, NotionPageCreateReq,
    NotionPageCreateResult, NotionSearchReq, NotionSearchResult, SlackHistoryReq,
    SlackHistoryResult, SlackPostReq, SlackPostResult,
};
use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::Value;
use std::sync::Arc;

pub(crate) async fn api_connectors_oauth_start(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(provider): AxumPath<String>,
    Json(body): Json<ConnectorOAuthStartReq>,
) -> Result<Json<Value>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        connectors_oauth_start_inner(&state, provider, body).await?,
    ))
}

pub(crate) async fn api_connectors_oauth_callback(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(provider): AxumPath<String>,
    Json(body): Json<ConnectorOAuthCallbackReq>,
) -> Result<Json<clawork_core::ConnectorStatus>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        connectors_oauth_callback_inner(&state, provider, body).await?,
    ))
}

pub(crate) async fn api_connectors_status(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<clawork_core::ConnectorStatus>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(connectors_status_inner(&state).await?))
}

pub(crate) async fn api_connectors_google_sheets_append(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<GoogleSheetsAppendReq>,
) -> Result<Json<GoogleSheetsAppendResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        connectors_google_sheets_append_inner(&state, body).await?,
    ))
}

pub(crate) async fn api_connectors_google_drive_create(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<GoogleDriveCreateReq>,
) -> Result<Json<GoogleDriveCreateResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        connectors_google_drive_create_inner(&state, body).await?,
    ))
}

pub(crate) async fn api_connectors_notion_search(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NotionSearchReq>,
) -> Result<Json<NotionSearchResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(connectors_notion_search_inner(&state, body).await?))
}

pub(crate) async fn api_connectors_notion_page_create(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NotionPageCreateReq>,
) -> Result<Json<NotionPageCreateResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        connectors_notion_page_create_inner(&state, body).await?,
    ))
}

pub(crate) async fn api_connectors_slack_post(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SlackPostReq>,
) -> Result<Json<SlackPostResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(connectors_slack_post_inner(&state, body).await?))
}

pub(crate) async fn api_connectors_slack_history(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SlackHistoryReq>,
) -> Result<Json<SlackHistoryResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(connectors_slack_history_inner(&state, body).await?))
}
