use super::{
    core::AppCore,
    dto::{MessageDto, SnapshotDto},
};
use std::sync::Arc;
use tauri::State;
#[tauri::command]
pub fn get_snapshot(core: State<'_, Arc<AppCore>>) -> SnapshotDto {
    core.snapshot()
}
#[tauri::command]
pub fn get_history(core: State<'_, Arc<AppCore>>) -> Vec<MessageDto> {
    core.history()
}
#[tauri::command]
pub fn send_message(core: State<'_, Arc<AppCore>>, text: String) {
    core.send(text);
}
#[tauri::command]
pub fn decide_approval(core: State<'_, Arc<AppCore>>, approval_id: String, decision: String) {
    core.decide(approval_id, decision);
}
#[tauri::command]
pub fn cancel_turn(core: State<'_, Arc<AppCore>>) {
    core.cancel();
}
