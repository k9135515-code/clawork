use crate::local_auth::require_auth;
use crate::{
    operator_create_project_inner, operator_create_session_inner, operator_create_task_inner,
    operator_get_session_inner, operator_list_artifacts_inner, operator_list_projects_inner,
    operator_list_sessions_inner, operator_list_tasks_inner, operator_pending_approvals_inner,
    operator_plan_session_inner, operator_run_session_inner, operator_run_task_now_inner,
    operator_set_action_state_inner, operator_timeline_inner, AppState, CommandError,
    LimitQueryDefault, OperatorActionProposal, OperatorActionStateReq, OperatorPlanReq,
    OperatorProjectCreateReq, OperatorTaskCreateReq,
};
use axum::extract::{Path as AxumPath, Query, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use clawork_core::{
    OperatorActionState, OperatorSession, OperatorStep, OperatorTaskTemplate, OperatorTimelineItem,
    ProjectArtifact, ProjectInfo,
};
use std::sync::Arc;

pub(crate) async fn api_operator_create_session(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<crate::OperatorCreateSessionReq>,
) -> Result<Json<OperatorSession>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_create_session_inner(&state, body.title, body.goal).await?,
    ))
}

pub(crate) async fn api_operator_list_sessions(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<OperatorSession>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_list_sessions_inner(&state, query.limit).await?,
    ))
}

pub(crate) async fn api_operator_get_session(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<OperatorSession>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(operator_get_session_inner(&state, session_id).await?))
}

pub(crate) async fn api_operator_plan_session(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<OperatorPlanReq>,
) -> Result<Json<Vec<OperatorStep>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_plan_session_inner(&state, session_id, body.steps).await?,
    ))
}

pub(crate) async fn api_operator_run_session(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<OperatorSession>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(operator_run_session_inner(&state, session_id).await?))
}

pub(crate) async fn api_operator_timeline(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<OperatorTimelineItem>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_timeline_inner(&state, session_id, query.limit).await?,
    ))
}

pub(crate) async fn api_operator_pending_approvals(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<OperatorActionProposal>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_pending_approvals_inner(&state, query.limit).await?,
    ))
}

pub(crate) async fn api_operator_action_approve(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(action_id): AxumPath<String>,
    Json(body): Json<OperatorActionStateReq>,
) -> Result<Json<OperatorActionProposal>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_set_action_state_inner(
            &state,
            action_id,
            OperatorActionState::Approved,
            body.actor,
        )
        .await?,
    ))
}

pub(crate) async fn api_operator_action_reject(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(action_id): AxumPath<String>,
    Json(body): Json<OperatorActionStateReq>,
) -> Result<Json<OperatorActionProposal>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_set_action_state_inner(
            &state,
            action_id,
            OperatorActionState::Rejected,
            body.actor,
        )
        .await?,
    ))
}

pub(crate) async fn api_operator_create_task(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<OperatorTaskCreateReq>,
) -> Result<Json<OperatorTaskTemplate>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(operator_create_task_inner(&state, body).await?))
}

pub(crate) async fn api_operator_list_tasks(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<OperatorTaskTemplate>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(operator_list_tasks_inner(&state, query.limit).await?))
}

pub(crate) async fn api_operator_run_task_now(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<String>,
) -> Result<Json<OperatorTaskTemplate>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(operator_run_task_now_inner(&state, task_id).await?))
}

pub(crate) async fn api_operator_list_projects(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<ProjectInfo>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_list_projects_inner(&state, query.limit).await?,
    ))
}

pub(crate) async fn api_operator_create_project(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<OperatorProjectCreateReq>,
) -> Result<Json<ProjectInfo>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_create_project_inner(&state, body.name, body.description).await?,
    ))
}

pub(crate) async fn api_operator_list_artifacts(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(project_id): AxumPath<String>,
    Query(query): Query<LimitQueryDefault>,
) -> Result<Json<Vec<ProjectArtifact>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        operator_list_artifacts_inner(&state, project_id, query.limit).await?,
    ))
}
