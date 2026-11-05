use std::path::Path;
use std::process::Command;

fn main() {
    // A clean checkout has no generated frontend. Build it before Tauri embeds it.
    if std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some()
        && !Path::new("../../web/dist/index.html").is_file()
    {
        let status = Command::new("pnpm")
            .args(["--dir", "../../web", "build"])
            .status()
            .expect("pnpm is required to build the frontend");
        assert!(status.success(), "frontend build failed");
    }
    tauri_build::build()
}
