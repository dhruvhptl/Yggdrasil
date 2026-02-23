mod commands;
mod database;
mod tree_commands;
mod brain;
mod mimir;

use database::Database;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let database = Database::new()
                    .await
                    .expect("Failed to initialize database");
                app_handle.manage(database);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            brain::generate_skill_tree,
            brain::analyze_repo,
            commands::create_project,
            commands::get_projects,
            commands::get_disciplines,
            commands::update_project,
            commands::update_project_progress,
            commands::delete_project,
            tree_commands::create_tree,
            tree_commands::get_trees,
            tree_commands::create_tree_node,
            tree_commands::update_tree_node,
            tree_commands::create_tree_edge,
            tree_commands::delete_tree_node,
            tree_commands::get_tree_with_contents,
            tree_commands::get_all_quests,
            tree_commands::recalculate_tree_progress,
            tree_commands::recalculate_unlocks,
            mimir::get_mimir_resources,
            mimir::get_node_resources,
            mimir::ingest_mimir_url,
            mimir::ingest_mimir_text,
            mimir::ingest_mimir_pdf,
            mimir::delete_mimir_resource,
            mimir::match_node_to_resources,
            mimir::link_resource_to_node,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
