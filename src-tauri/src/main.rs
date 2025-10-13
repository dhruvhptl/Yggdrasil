// src-tauri/src/main.rs - Main entry point for Rust backend

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::AppState;

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::get_projects,
            commands::get_disciplines
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
