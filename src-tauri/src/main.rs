mod commands;
mod database;
mod tree_commands;

use database::Database;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle();
            tauri::async_runtime::spawn(async move {
                let database = Database::new(&app_handle).await.expect("Failed to initialize database");
                app_handle.manage(database);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_project,
            commands::get_projects,
            commands::get_disciplines,
            commands::update_project_progress,
            commands::delete_project,
            tree_commands::create_tree,
            tree_commands::get_trees,
            tree_commands::create_tree_node,
            tree_commands::update_tree_node,
            tree_commands::create_tree_edge,
            tree_commands::get_tree_with_contents
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    app.run(|_, _| {});
}