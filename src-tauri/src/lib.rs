// src-tauri/src/lib.rs - Library configuration for Tauri app with database support

pub mod commands;
pub mod database;

// Re-export main types for easier access
pub use commands::{Project, Discipline, Skill};
pub use database::Database;

// Tauri configuration and setup
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            
            // Initialize database asynchronously
            tauri::async_runtime::spawn(async move {
                match Database::new(&app_handle).await {
                    Ok(database) => {
                        app_handle.manage(database);
                        println!("Database initialized successfully");
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize database: {}", e);
                        std::process::exit(1);
                    }
                }
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}