use std::sync::{Arc, Mutex};

use tauri::Manager;

pub mod guid;
pub mod input;

/// Return the current device list, maintained by the input thread.
#[tauri::command]
fn list_joysticks(devices: tauri::State<input::DeviceList>) -> Vec<input::DeviceInfo> {
    devices.lock().map(|d| d.clone()).unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer triggers "Error 71 (Protocol error)" on many
    // Wayland/Nvidia setups. Disabling it is the canonical fix. Respect an
    // explicit user override; otherwise force it off on Linux.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let devices: input::DeviceList = Arc::new(Mutex::new(Vec::new()));
            app.manage(devices.clone());
            input::spawn(app.handle().clone(), devices);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_joysticks])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
