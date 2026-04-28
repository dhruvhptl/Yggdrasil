// Prevents the console window from appearing on Windows in production builds.
// The `cfg_attr` ensures it only applies in release mode so you keep the console during dev.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod constants;
mod database;
mod tree_commands;
mod llm_client;
mod github;
mod prompt_builders;
mod tree_persistence;
mod brain;
mod mimir;
mod mimir_ingest;
mod mimir_retrieval;
mod mimir_tags;
mod mimir_manage;
mod work_commands;
mod job_commands;
mod idea_commands;
mod resume_commands;
mod skill_commands;
mod export_commands;
mod daily_commands;
mod read_models;
mod orchestrator;

use database::Database;
use tauri::Manager;

fn load_env_file() {
    // In production, env vars aren't inherited from a shell.
    // Read them from %APPDATA%/com.universal.skilltree/.env written by build.sh.
    #[cfg(not(debug_assertions))]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let env_path = std::path::Path::new(&appdata)
                .join("com.universal.skilltree")
                .join(".env");
            if let Ok(contents) = std::fs::read_to_string(&env_path) {
                for line in contents.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        let key = key.trim();
                        let value = value.trim();
                        if !key.is_empty() && !key.starts_with('#') {
                            std::env::set_var(key, value);
                        }
                    }
                }
            }
        }
    }
}

fn main() {
    load_env_file();
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let database = Database::new()
                    .await
                    .expect("Failed to initialize database");
                let pool = database.pool.clone();
                app_handle.manage(database);
                let http_client = reqwest::Client::new();
                let queue = orchestrator::start_worker(pool, app_handle.clone(), http_client.clone());
                app_handle.manage(http_client);
                app_handle.manage(queue);
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
            tree_commands::get_tree_node_data,
            mimir_manage::get_mimir_resources,
            mimir_manage::get_node_resources,
            mimir_ingest::ingest_mimir_url,
            mimir_ingest::ingest_mimir_text,
            mimir_ingest::ingest_mimir_pdf,
            mimir_manage::delete_mimir_resource,
            mimir_retrieval::match_node_to_resources,
            mimir_manage::link_resource_to_node,
            mimir_ingest::extract_pdf_text,
            mimir_retrieval::mimir_chat,
            mimir_manage::discover_links,
            mimir_manage::fetch_playlist,
            mimir_ingest::rescrape_resource,
            mimir_ingest::rescrape_all,
            mimir_manage::get_chunk_counts,
            mimir_tags::get_distinct_tags,
            mimir_tags::update_resource_tags,
            mimir_tags::auto_tag_existing_resources,
            mimir_retrieval::rematch_all_nodes,
            mimir_manage::toggle_resource_completion,
            mimir_manage::on_resource_completed,
            mimir_manage::get_linked_node_titles,
            mimir_ingest::reembed_pdfs,
            mimir_retrieval::get_chat_session,
            mimir_retrieval::clear_chat_session,
            mimir_retrieval::get_retrieval_stats,
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
            skill_commands::get_skill_aliases,
            skill_commands::merge_skills,
            skill_commands::mark_skill_reviewed,
            skill_commands::classify_skill_domains,
            skill_commands::reset_skill_domains,
            skill_commands::expand_skill_graph,
            read_models::get_active_tree_for_project,
            read_models::get_node_chat_context,
            read_models::get_project_tree_summary,
            read_models::get_skill_graph_snapshot,
            read_models::get_tree_resource_gaps,
            read_models::get_node_neighborhood,
            read_models::get_growth_recommendations,
            read_models::get_prereq_path,
            orchestrator::enqueue_rematch,
            orchestrator::enqueue_reembed,
            orchestrator::enqueue_autotag,
            orchestrator::enqueue_infer_deps,
            brain::get_prompt_stats,
            brain::get_tree_concept_graph,
            brain::regenerate_tree,
            brain::get_tree_regenerations,
            tree_persistence::backfill_node_embeddings,
            export_commands::export_tree,
            daily_commands::get_daily_log,
            daily_commands::upsert_daily_notes,
            daily_commands::add_quest_to_day,
            daily_commands::add_free_task_to_day,
            daily_commands::move_to_quadrant,
            daily_commands::remove_from_day,
            daily_commands::toggle_task_complete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
