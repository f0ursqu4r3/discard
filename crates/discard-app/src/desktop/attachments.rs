use super::settings::SettingsContext;
use discard_core::attachments::{self, Attachment};
use tauri::State;

#[tauri::command]
pub async fn choose_attachments(
    app: tauri::AppHandle,
    context: State<'_, SettingsContext>,
) -> Result<Vec<Attachment>, String> {
    use tauri_plugin_dialog::DialogExt;
    let home = context.home.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(files) = app
            .dialog()
            .file()
            .set_title("Attach files · up to 8 files, 5 MB each")
            .blocking_pick_files()
        else {
            return Ok(Vec::new());
        };
        let paths = files
            .into_iter()
            .map(|file| file.into_path().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        attachments::import_files(&home, &paths)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn import_attachments(
    context: State<'_, SettingsContext>,
    files: Vec<attachments::AttachmentUpload>,
) -> Result<Vec<Attachment>, String> {
    let home = context.home.clone();
    tauri::async_runtime::spawn_blocking(move || attachments::import_uploads(&home, files))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn attachment_preview(
    context: State<'_, SettingsContext>,
    id: String,
) -> Result<String, String> {
    let home = context.home.clone();
    tauri::async_runtime::spawn_blocking(move || attachments::load(&home, &id)?.data_url())
        .await
        .map_err(|e| e.to_string())?
}
