//! Applies the app icon the user picked in Settings.
use discard_core::settings::AppIcon;
use tauri::AppHandle;

pub(super) fn png(icon: AppIcon) -> &'static [u8] {
    match icon {
        AppIcon::Classic => include_bytes!("../../icons/classic.png"),
        AppIcon::Landscape => include_bytes!("../../icons/landscape.png"),
        AppIcon::Graffiti => include_bytes!("../../icons/graffiti.png"),
        AppIcon::Manga => include_bytes!("../../icons/manga.png"),
        AppIcon::Cyberpunk => include_bytes!("../../icons/cyberpunk.png"),
        AppIcon::Matrix => include_bytes!("../../icons/matrix.png"),
        AppIcon::Synthwave => include_bytes!("../../icons/synthwave.png"),
    }
}

/// Show `icon` as the Dock icon on macOS, or as the icon of every window and of the tray icon elsewhere.
pub fn apply(app: &AppHandle, icon: AppIcon) {
    let bytes = png(icon);
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || {
            use objc2::AnyThread;
            let Some(main_thread) = objc2::MainThreadMarker::new() else { return };
            let data = objc2_foundation::NSData::with_bytes(bytes);
            let Some(image) = objc2_app_kit::NSImage::initWithData(objc2_app_kit::NSImage::alloc(), &data) else {
                tracing::warn!(icon = icon.as_str(), "cannot decode app icon");
                return;
            };
            let application = objc2_app_kit::NSApplication::sharedApplication(main_thread);
            // SAFETY: The image is a valid NSImage, and this runs on the main thread.
            unsafe { application.setApplicationIconImage(Some(&image)) };
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::Manager;
        match tauri::image::Image::from_bytes(bytes) {
            Ok(image) => {
                for window in app.webview_windows().values() {
                    let _ = window.set_icon(image.clone());
                }
                // The tray icon follows the setting (spec T5). Windows and Linux only: macOS has none.
                super::tray::set_icon(app, image);
            }
            Err(error) => tracing::warn!(%error, icon = icon.as_str(), "cannot decode app icon"),
        }
    }
}
