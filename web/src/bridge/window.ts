import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

/**
 * The label of the window that runs this page: `main` for the main window, `project-<id>` for a project window.
 * The core sends the events of a view to the window with this label. Outside Tauri (Vitest, a plain browser) there is
 * no window, and the page is the main window.
 */
export function currentWindowLabel(): string {
  try {
    return getCurrentWebviewWindow().label
  } catch {
    return 'main'
  }
}
