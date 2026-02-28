use crate::local_auth::require_auth;
use crate::{
    get_daily_briefing_inner, list_proactive_suggestions_inner, media_generate_inner,
    research_create_job_inner, research_get_job_inner, research_get_report_inner, AppState,
    CommandError, LimitQueryUsize, MediaGenerateReq, MediaGenerateResult, ResearchJobCreateReq,
};
use axum::extract::{Path as AxumPath, Query, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use clawork_core::{DailyBriefing, ProactiveSuggestion};
use clawork_operator::ResearchJobRecord;
use serde_json::Value;
use std::sync::Arc;

pub(crate) async fn api_briefing(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<DailyBriefing>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(get_daily_briefing_inner(&state).await?))
}

pub(crate) async fn api_suggestions(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQueryUsize>,
) -> Result<Json<Vec<ProactiveSuggestion>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        list_proactive_suggestions_inner(&state, query.limit).await?,
    ))
}

pub(crate) async fn api_research_create_job(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ResearchJobCreateReq>,
) -> Result<Json<ResearchJobRecord>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(research_create_job_inner(&state, body).await?))
}

pub(crate) async fn api_research_get_job(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<ResearchJobRecord>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(research_get_job_inner(&state, id).await?))
}

pub(crate) async fn api_research_get_report(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(research_get_report_inner(&state, id).await?))
}

pub(crate) async fn api_media_image(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MediaGenerateReq>,
) -> Result<Json<MediaGenerateResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(media_generate_inner(&state, "image", body).await?))
}

pub(crate) async fn api_media_video(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MediaGenerateReq>,
) -> Result<Json<MediaGenerateResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(media_generate_inner(&state, "video", body).await?))
}
