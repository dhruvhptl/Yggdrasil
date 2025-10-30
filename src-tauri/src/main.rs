// src-tauri/src/main.rs - Main entry point for Rust backend with database initialization

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod database;

use database::Database;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle();
            
            // Initialize database in async context
            tauri::async_runtime::spawn(async move {
                let database = Database::new(&app_handle).await
                    .expect("Failed to initialize database");
                app_handle.manage(database);
            });
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::get_projects,
            commands::get_disciplines,
            commands::update_project_progress,
            commands::delete_project
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");
    
    // Wait for database initialization before starting the app
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    app.run(|_app_handle, event| match event {
        tauri::RunEvent::Updater(updater_event) => {
            match updater_event {
                tauri::UpdaterEvent::UpdateAvailable { body, date, version } => {
                    println!("update available {} {:?} {}", body, date, version);
                }
                tauri::UpdaterEvent::Updated => {
                    println!("app has been updated");
                }
                _ => {}
            }
        }
        _ => {}
    });
}