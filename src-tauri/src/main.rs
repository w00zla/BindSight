// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Force X11 backend on Linux to prevent massive CSD titlebars in KDE Wayland
    #[cfg(target_os = "linux")]
    std::env::set_var("GDK_BACKEND", "x11");
    
    bindsight_lib::run()
}
