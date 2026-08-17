//! Memory injection: per-project toggle + the recent-injections feed the GUI
//! shows so nothing is ever fed to the agent silently.

use crate::error::AppResult;
use crate::services::corpus_feedback;
use crate::services::inject_service::{self, InjectionRecord};

#[tauri::command]
pub fn memory_injection_enabled(project_root: String) -> bool {
    inject_service::is_enabled(&project_root)
}

#[tauri::command]
pub fn memory_injection_set_enabled(project_root: String, enabled: bool) -> AppResult<()> {
    inject_service::set_enabled(&project_root, enabled)
}

#[tauri::command]
pub fn memory_injection_recent() -> Vec<InjectionRecord> {
    inject_service::recent()
}

/// One corpus doc's outcome stats, shaped for the GUI.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocFeedback {
    pub doc_id: String,
    pub injected_count: u32,
    pub helpful: u32,
    pub unhelpful: u32,
    pub last_injected_ms: u64,
    pub excluded: bool,
    pub pinned: bool,
}

fn to_doc_feedback(doc_id: String, s: corpus_feedback::DocStats) -> DocFeedback {
    DocFeedback {
        excluded: s.excluded(),
        pinned: s.pinned,
        doc_id,
        injected_count: s.injected_count,
        helpful: s.helpful,
        unhelpful: s.unhelpful,
        last_injected_ms: s.last_injected_ms,
    }
}

/// Flywheel metrics (E4). `day_starts` = ascending LOCAL day boundaries
/// computed by the frontend (N+1 boundaries → N-day curve).
#[tauri::command]
pub fn flywheel_metrics(
    project_root: String,
    day_starts: Vec<i64>,
) -> crate::services::flywheel::FlywheelMetrics {
    crate::services::flywheel::metrics(&project_root, &day_starts)
}

#[tauri::command]
pub fn work_profile_get() -> String {
    crate::services::work_profile::get()
}

#[tauri::command]
pub fn work_profile_set(content: String) -> AppResult<()> {
    crate::services::work_profile::set(&content)
}

#[tauri::command]
pub fn memory_feedback_stats(project_root: String) -> Vec<DocFeedback> {
    let mut out: Vec<DocFeedback> = corpus_feedback::stats(&project_root)
        .into_iter()
        .map(|(id, s)| to_doc_feedback(id, s))
        .collect();
    // Most-used first — the ranking the Coach view will lean on.
    out.sort_by_key(|d| std::cmp::Reverse(d.injected_count));
    out
}

#[tauri::command]
pub fn memory_feedback_set(
    project_root: String,
    doc_id: String,
    helpful: bool,
) -> AppResult<DocFeedback> {
    let s = corpus_feedback::set_verdict(&project_root, &doc_id, helpful)?;
    Ok(to_doc_feedback(doc_id, s))
}

#[tauri::command]
pub fn memory_feedback_pin(
    project_root: String,
    doc_id: String,
    pinned: bool,
) -> AppResult<DocFeedback> {
    let s = corpus_feedback::set_pinned(&project_root, &doc_id, pinned)?;
    Ok(to_doc_feedback(doc_id, s))
}

#[tauri::command]
pub fn memory_feedback_reset(project_root: String, doc_id: String) -> AppResult<DocFeedback> {
    let s = corpus_feedback::reset_verdicts(&project_root, &doc_id)?;
    Ok(to_doc_feedback(doc_id, s))
}
