use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::{api::AppError, AppState};

pub async fn start(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let mut rec = state.ptt_recorder.lock().await;
    if rec.is_recording() {
        return Ok((StatusCode::OK, Json(json!({"status": "already_recording"}))));
    }
    rec.start().map_err(AppError::Internal)?;
    tracing::info!("PTT started via HTTP");
    Ok((StatusCode::OK, Json(json!({"status": "recording"}))))
}

pub async fn stop(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    transcribe_and_resolve(state).await
}

/// Toggle: if idle → start recording; if recording → stop, transcribe, resolve.
pub async fn toggle(State(state): State<AppState>) -> Result<Response, AppError> {
    let is_recording = state.ptt_recorder.lock().await.is_recording();
    if is_recording {
        Ok(transcribe_and_resolve(state).await?.into_response())
    } else {
        let mut rec = state.ptt_recorder.lock().await;
        rec.start().map_err(AppError::Internal)?;
        tracing::info!("PTT started via toggle");
        Ok(Json(json!({"status": "recording"})).into_response())
    }
}

async fn transcribe_and_resolve(state: AppState) -> Result<impl IntoResponse, AppError> {
    let audio = {
        let mut rec = state.ptt_recorder.lock().await;
        if !rec.is_recording() {
            return Ok(Json(json!({"status": "not_recording", "transcript": null})));
        }
        rec.stop()
    };

    // 16 kHz mono: 1600 samples = 0.1 s minimum.
    if audio.len() < 1_600 {
        tracing::warn!("PTT stop: audio too short");
        return Ok(Json(json!({"status": "too_short", "transcript": null})));
    }

    let Some(transcriber) = state.transcriber.clone() else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Whisper model not loaded"
        )));
    };

    let initial_prompt = {
        let router = state.router.lock().await;
        let agents = state.agents.read().await;
        let pending_id = router.pending_agent_ids().next().map(str::to_string);
        drop(router);
        pending_id
            .map(|id| state.glossary.whisper_prompt(&agents.context_terms(&id)))
            .unwrap_or_default()
    };

    let transcript =
        tokio::task::spawn_blocking(move || transcriber.transcribe(&audio, &initial_prompt))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
            .map_err(AppError::Internal)?;

    tracing::info!(transcript = %transcript, "PTT transcribed via HTTP");

    let corrected = state.glossary.apply_corrections(&transcript);

    let mut router = state.router.lock().await;
    let agent_ids: Vec<String> = router.pending_agent_ids().map(str::to_string).collect();
    let resolved_agent = if let Some(id) = agent_ids.first() {
        if router.resolve(id, corrected.clone()) {
            tracing::info!(agent_id = %id, answer = %corrected, "ask resolved via PTT");
            Some(id.clone())
        } else {
            None
        }
    } else {
        tracing::info!("PTT: no pending ask");
        None
    };

    Ok(Json(json!({
        "status": "ok",
        "transcript": corrected,
        "resolved_agent": resolved_agent,
    })))
}
