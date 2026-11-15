//! Test-only capture of this app's own WebView; no screen/other-window access.
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

static STARTED: AtomicBool = AtomicBool::new(false);
pub fn start_once() -> bool {
    !STARTED.swap(true, Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
pub fn finish(app: tauri::AppHandle, report: Value) -> Result<(), String> {
    use block2::RcBlock;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::WKWebView;
    let root = std::path::PathBuf::from(
        std::env::var_os("CLOUD_DECK_DATA_DIR").ok_or("Missing isolated smoke directory")?,
    );
    if !root.is_absolute() {
        return Err("Smoke directory must be absolute".into());
    }
    std::fs::create_dir_all(&root).map_err(|_| "Could not create smoke directory")?;
    let window = app
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    window
        .with_webview(move |platform| {
            let callback = RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
                let mut result = report.clone();
                // WebKit owns the image for this callback; the empty PNG properties dictionary is well typed.
                let png = unsafe { image.as_ref() }
                    .and_then(|image| image.TIFFRepresentation())
                    .and_then(|data| NSBitmapImageRep::imageRepWithData(&data))
                    .and_then(|bitmap| unsafe {
                        bitmap.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    });
                let captured = png.is_some_and(|data| {
                    std::fs::write(root.join("native-window.png"), data.to_vec()).is_ok()
                });
                result["nativeScreenshot"] = Value::Bool(captured);
                let passed = result["passed"].as_bool() == Some(true) && captured;
                let keep_open = passed
                    && crate::live_check_mode()
                    && std::env::var("CLOUD_DECK_KEEP_OPEN").as_deref() == Ok("1");
                result["nativeProcessRetained"] = Value::Bool(keep_open);
                let _ = std::fs::write(
                    root.join("native-check.json"),
                    serde_json::to_vec_pretty(&result).unwrap_or_default(),
                );
                if !keep_open {
                    app.exit(if passed { 0 } else { 1 });
                }
            });
            // Tauri invokes this closure on the UI thread and provides its own WKWebView pointer.
            unsafe {
                let webview = &*(platform.inner() as *mut WKWebView);
                webview.takeSnapshotWithConfiguration_completionHandler(None, &callback);
            }
        })
        .map_err(|_| "Could not capture native WebView".into())
}

#[cfg(not(target_os = "macos"))]
pub fn finish(_app: tauri::AppHandle, _report: Value) -> Result<(), String> {
    Err("Native screenshot check is currently implemented for macOS only".into())
}
