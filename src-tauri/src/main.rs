mod commands;
mod database;
mod tree_commands;
mod brain;
mod mimir;
mod work_commands;
mod job_commands;
mod idea_commands;
mod resume_commands;
mod skill_commands;
mod export_commands;

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
            mimir::extract_pdf_text,
            mimir::mimir_chat,
            mimir::discover_links,
            mimir::fetch_playlist,
            mimir::rescrape_resource,
            mimir::rescrape_all,
            mimir::get_chunk_counts,
            work_commands::create_coop,
            work_commands::get_coops,
            work_commands::create_topic,
            work_commands::add_resource,
            work_commands::toggle_resource_completed,
            work_commands::extract_skills,
            work_commands::get_full_work_graph,
            job_commands::create_job,
            job_commands::get_jobs,
            job_commands::update_job,
            job_commands::delete_job,
            job_commands::save_job_description,
            job_commands::extract_job_skills,
            job_commands::get_job_skills,
            job_commands::get_skill_demand,
            job_commands::mark_followed_up,
            job_commands::reextract_all_skills,
            idea_commands::create_idea,
            idea_commands::get_ideas,
            idea_commands::update_idea,
            idea_commands::delete_idea,
            idea_commands::idea_to_project,
            resume_commands::parse_resume,
            resume_commands::get_resume,
            resume_commands::link_resume_project,
            resume_commands::unlink_resume_project,
            resume_commands::delete_resume,
            skill_commands::sync_skills_from_resume,
            skill_commands::sync_skills_from_trees,
            skill_commands::sync_skills_from_work,
            skill_commands::sync_all_skills,
            skill_commands::recalculate_skill_levels,
            skill_commands::get_universal_skills,
            skill_commands::get_skill_dependencies,
            skill_commands::get_skill_gaps,
            skill_commands::infer_skill_dependencies,
            export_commands::export_tree,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
