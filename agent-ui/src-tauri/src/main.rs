#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use tauri::{Manager, PhysicalSize, SystemTray};

fn main() {
    let system_tray = SystemTray::new();
    tauri::Builder::default()
        .system_tray(system_tray)
        .on_window_event(|event| match event.event() {
            tauri::WindowEvent::CloseRequested { .. } => {
                std::process::exit(0);
            },
            tauri::WindowEvent::Resized(PhysicalSize { width, height }) => {
                if *width == 0 && *height == 0 {
                    event.window().hide();
                }
            },
            event => {
                println!("Window event happen: {:?}", event);
            },
        })
        .on_system_tray_event(|app, event| match event {
            tauri::SystemTrayEvent::LeftClick { .. } => {
                let main_window = app.get_window("main").unwrap();
                if let Ok(true) = main_window.is_visible() {
                    main_window.hide();
                } else {
                    main_window.show();
                    main_window.set_focus();
                };
            },
            tauri::SystemTrayEvent::RightClick { .. } => {
                let main_window = app.get_window("main").unwrap();
                if let Ok(true) = main_window.is_visible() {
                    main_window.hide();
                } else {
                    main_window.show();
                    main_window.set_focus();
                };
            },
            tauri::SystemTrayEvent::DoubleClick { .. } => {
                let main_window = app.get_window("main").unwrap();
                if let Ok(true) = main_window.is_visible() {
                    main_window.hide();
                } else {
                    main_window.show();
                };
            },
            _ => todo!(),
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
