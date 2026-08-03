// src-tauri/src/resume_commands.rs

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
use crate::constants::GROQ_API_URL;
use crate::database::Database;

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Education {
    #[serde(default)]
    pub institution: String,
    #[serde(default)]
    pub degree: String,
    #[serde(default)]
    pub year: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkExperience {
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParsedProject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, alias = "techStack")]
    pub tech_stack: Vec<String>,
    #[serde(default, alias = "githubUrl")]
    pub github_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResumeProfile {
    pub id: String,
    pub raw_text: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub education: Vec<Education>,
    pub work_experience: Vec<WorkExperience>,
    pub skills: Vec<String>,
    pub projects: Vec<ResumeProject>,
    pub label: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight row for the resume-version switcher (no heavy fields).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResumeHeader {
    pub id: String,
    pub label: String,
    pub name: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResumeProject {
    pub id: String,
    pub resume_id: String,
    pub name: String,
    pub description: Option<String>,
    pub tech_stack: Vec<String>,
    pub github_url: Option<String>,
    pub linked_project_id: Option<String>,
    pub linked_project_name: Option<String>,
    pub created_at: String,
}

// ─── Groq types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GroqResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqMessage,
}

#[derive(Debug, Deserialize)]
struct GroqMessage {
    content: String,
}

// Call 1 response: profile info (name, email, skills, education, work experience)
#[derive(Debug, Deserialize)]
struct ParsedProfileAI {
    name: Option<String>,
    email: Option<String>,
    skills: Option<Vec<String>>,
    education: Option<Vec<Education>>,
    work_experience: Option<Vec<WorkExperience>>,
}

// Call 2 response: projects
#[derive(Debug, Deserialize)]
struct ParsedProjectsAI {
    projects: Option<Vec<ParsedProject>>,
}

// ─── Groq helper ────────────────────────────────────────────────────────────

async fn call_groq(client: &reqwest::Client, api_key: &str, system_prompt: &str, user_prompt: &str) -> Result<String, String> {

    let request_body = json!({
        "model": "llama-3.3-70b-versatile",
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": 0.3,
        "max_tokens": 4096,
        "response_format": { "type": "json_object" }
    });

    let response = client
        .post(GROQ_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Groq API returned {}: {}", status, response_text));
    }

    let groq_response: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse Groq response: {}", e))?;

    Ok(groq_response.choices[0].message.content.clone())
}

// ─── Commands ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn parse_resume(
    text: String,
    label: String,
    app: tauri::AppHandle,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<ResumeProfile, String> {
    println!("\n=== Parse Resume ===");
    println!("Text length: {} chars\n", text.len());

    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    let system_prompt = "You extract structured data from resumes. Return ONLY valid JSON, nothing else.";

    // ── Call 1: Extract profile (name, email, skills, education, work experience) ──
    println!("Calling Groq API — Call 1: profile extraction…");
    let profile_prompt = format!(
        "Extract the person's profile from this resume. Return ONLY a JSON object with this exact shape:\n\
        {{\n  \"name\": \"string\",\n  \"email\": \"string or null\",\n  \
        \"skills\": [\"skill1\", \"skill2\"],\n  \
        \"education\": [{{\"institution\": \"\", \"degree\": \"\", \"year\": \"\"}}],\n  \
        \"work_experience\": [{{\"company\": \"\", \"role\": \"\", \"start\": \"\", \"end\": \"\", \"description\": \"\"}}]\n}}\n\
        Extract ALL education entries and ALL work experience entries. \
        For skills, list every technical skill, tool, framework, and language mentioned.\n\
        Return ONLY the JSON, nothing else.\n\nResume:\n{}",
        text
    );
    let profile_json = call_groq(&*client, &api_key, system_prompt, &profile_prompt).await?;
    println!("Got profile response: {} chars", profile_json.len());

    let profile: ParsedProfileAI = serde_json::from_str(&profile_json)
        .map_err(|e| format!("Failed to parse profile AI response: {}. Raw: {}", e, crate::text_util::truncate_chars(&profile_json, 200)))?;

    // ── Call 2: Extract projects ────────────────────────────────────────────────
    println!("Calling Groq API — Call 2: project extraction…");
    let projects_prompt = format!(
        "Extract projects from this resume, but ONLY the ones explicitly listed under a \
        dedicated projects section — a heading such as \"Projects\", \"Project Experience\", \
        \"Personal Projects\", \"Academic Projects\", or the like.\n\
        Return ONLY a JSON object with this exact shape:\n\
        {{\n  \"projects\": [\n    {{\n      \"name\": \"Project Name\",\n      \
        \"description\": \"What the project does and key accomplishments\",\n      \
        \"tech_stack\": [\"React\", \"Python\", \"etc\"],\n      \
        \"github_url\": \"https://github.com/... or null\"\n    }}\n  ]\n}}\n\
        STRICT RULES:\n\
        - Include a project ONLY if it appears as its own entry under a projects-style heading.\n\
        - Do NOT extract anything from Work Experience, Employment, or Internship sections. \
        Bullet points inside a job description are job responsibilities/sub-tasks, NOT standalone \
        projects — ignore them even when they describe building, shipping, or launching something.\n\
        - Do NOT invent projects from the summary, skills, or education sections.\n\
        - If the resume has no dedicated projects section, return an empty array: {{\"projects\": []}}.\n\
        Return ONLY the JSON, nothing else.\n\nResume:\n{}",
        text
    );
    let projects_json = call_groq(&*client, &api_key, system_prompt, &projects_prompt).await?;
    println!("Got projects response: {} chars", projects_json.len());

    let projects_parsed: ParsedProjectsAI = serde_json::from_str(&projects_json)
        .map_err(|e| format!("Failed to parse projects AI response: {}. Raw: {}", e, crate::text_util::truncate_chars(&projects_json, 200)))?;

    // ── Save to database ────────────────────────────────────────────────────────

    let profile_id = Uuid::new_v4().to_string();
    let education_json = serde_json::to_value(&profile.education.unwrap_or_default())
        .map_err(|e| format!("Failed to serialize education: {}", e))?;
    let work_exp_json = serde_json::to_value(&profile.work_experience.unwrap_or_default())
        .map_err(|e| format!("Failed to serialize work experience: {}", e))?;
    let skills_json = serde_json::to_value(&profile.skills.unwrap_or_default())
        .map_err(|e| format!("Failed to serialize skills: {}", e))?;

    // Flip the current active off and insert the new profile as active, in one
    // transaction so a crash mid-flip can't leave zero active resumes. Every new
    // parse becomes the active resume; older versions are kept as history.
    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE resume_profile SET is_active = FALSE WHERE is_active")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO resume_profile (id, raw_text, name, email, education, work_experience, skills, label, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE)"
    )
    .bind(&profile_id)
    .bind(&text)
    .bind(&profile.name)
    .bind(&profile.email)
    .bind(&education_json)
    .bind(&work_exp_json)
    .bind(&skills_json)
    .bind(&label)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Failed to save resume profile: {}", e))?;
    tx.commit().await.map_err(|e| e.to_string())?;

    println!("✅ Saved resume profile: {}", profile_id);

    // Save projects
    let projects = projects_parsed.projects.unwrap_or_default();
    let mut saved_projects: Vec<ResumeProject> = Vec::new();

    for p in &projects {
        let project_id = Uuid::new_v4().to_string();
        let tech_stack_json = serde_json::to_value(&p.tech_stack)
            .map_err(|e| format!("Failed to serialize tech stack: {}", e))?;
        let github_url = p.github_url.as_deref().filter(|u| !u.is_empty());

        sqlx::query(
            "INSERT INTO resume_projects (id, resume_id, name, description, tech_stack, github_url) \
             VALUES ($1, $2, $3, $4, $5, $6)"
        )
        .bind(&project_id)
        .bind(&profile_id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(&tech_stack_json)
        .bind(&github_url)
        .execute(&database.pool)
        .await
        .map_err(|e| format!("Failed to save resume project: {}", e))?;

        println!("  📁 Project: {}", p.name);

        saved_projects.push(ResumeProject {
            id: project_id,
            resume_id: profile_id.clone(),
            name: p.name.clone(),
            description: Some(p.description.clone()),
            tech_stack: p.tech_stack.clone(),
            github_url: github_url.map(|s| s.to_string()),
            linked_project_id: None,
            linked_project_name: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    let education: Vec<Education> = serde_json::from_value(education_json).unwrap_or_default();
    let work_experience: Vec<WorkExperience> = serde_json::from_value(work_exp_json).unwrap_or_default();
    let skills: Vec<String> = serde_json::from_value(skills_json).unwrap_or_default();

    crate::orchestrator::on_resume_parsed(&database.pool, &app).await;

    Ok(ResumeProfile {
        id: profile_id,
        raw_text: text,
        name: profile.name,
        email: profile.email,
        education,
        work_experience,
        skills,
        projects: saved_projects,
        label,
        is_active: true,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn get_resume(
    database: State<'_, Database>,
) -> Result<Option<ResumeProfile>, String> {
    use sqlx::Row;

    // Active-first, newest as fallback — robust even if no row is flagged active.
    let profile_row = sqlx::query(
        "SELECT id, raw_text, name, email, education, work_experience, skills, label, is_active, created_at, updated_at \
         FROM resume_profile ORDER BY is_active DESC, created_at DESC LIMIT 1"
    )
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = match profile_row {
        Some(r) => r,
        None => return Ok(None),
    };

    let profile_id: String = row.try_get("id").map_err(|e| e.to_string())?;

    // Fetch projects with linked project name in a single LEFT JOIN — no N+1
    let project_rows = sqlx::query(
        "SELECT rp.id, rp.resume_id, rp.name, rp.description, rp.tech_stack, rp.github_url, \
                rp.linked_project_id, rp.created_at, p.name AS linked_project_name \
         FROM resume_projects rp \
         LEFT JOIN projects p ON p.id = rp.linked_project_id \
         WHERE rp.resume_id = $1 \
         ORDER BY rp.created_at"
    )
    .bind(&profile_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut projects: Vec<ResumeProject> = Vec::new();
    for pr in &project_rows {
        projects.push(ResumeProject {
            id: pr.try_get("id").map_err(|e| e.to_string())?,
            resume_id: pr.try_get("resume_id").map_err(|e| e.to_string())?,
            name: pr.try_get("name").map_err(|e| e.to_string())?,
            description: pr.try_get("description").map_err(|e| e.to_string())?,
            tech_stack: serde_json::from_value(
                pr.try_get::<serde_json::Value, _>("tech_stack").map_err(|e| e.to_string())?
            ).unwrap_or_default(),
            github_url: pr.try_get("github_url").map_err(|e| e.to_string())?,
            linked_project_id: pr.try_get("linked_project_id").map_err(|e| e.to_string())?,
            linked_project_name: pr.try_get("linked_project_name").map_err(|e| e.to_string())?,
            created_at: pr.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map(|d| d.to_rfc3339())
                .map_err(|e| e.to_string())?,
        });
    }

    let education: Vec<Education> = serde_json::from_value(
        row.try_get::<serde_json::Value, _>("education").map_err(|e| e.to_string())?
    ).unwrap_or_default();
    let work_experience: Vec<WorkExperience> = serde_json::from_value(
        row.try_get::<serde_json::Value, _>("work_experience").map_err(|e| e.to_string())?
    ).unwrap_or_default();
    let skills: Vec<String> = serde_json::from_value(
        row.try_get::<serde_json::Value, _>("skills").map_err(|e| e.to_string())?
    ).unwrap_or_default();

    Ok(Some(ResumeProfile {
        id: profile_id,
        raw_text: row.try_get("raw_text").map_err(|e| e.to_string())?,
        name: row.try_get("name").map_err(|e| e.to_string())?,
        email: row.try_get("email").map_err(|e| e.to_string())?,
        education,
        work_experience,
        skills,
        projects,
        label: row.try_get("label").map_err(|e| e.to_string())?,
        is_active: row.try_get("is_active").map_err(|e| e.to_string())?,
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|d| d.to_rfc3339())
            .map_err(|e| e.to_string())?,
        updated_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
            .map(|d| d.to_rfc3339())
            .map_err(|e| e.to_string())?,
    }))
}

#[tauri::command]
pub async fn link_resume_project(
    resume_project_id: String,
    project_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE resume_projects SET linked_project_id = $1 WHERE id = $2"
    )
    .bind(&project_id)
    .bind(&resume_project_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    println!("🔗 Linked resume project {} → project {}", resume_project_id, project_id);
    Ok(())
}

#[tauri::command]
pub async fn unlink_resume_project(
    resume_project_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE resume_projects SET linked_project_id = NULL WHERE id = $1"
    )
    .bind(&resume_project_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    println!("🔓 Unlinked resume project {}", resume_project_id);
    Ok(())
}

#[tauri::command]
pub async fn delete_resume(
    id: String,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<(), String> {
    // Delete the target profile (resume_projects cascade via FK). If it was the
    // active one, promote the newest remaining resume so there's always an active.
    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM resume_profile WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE resume_profile SET is_active = TRUE \
         WHERE id = (SELECT id FROM resume_profile ORDER BY created_at DESC LIMIT 1) \
           AND NOT EXISTS (SELECT 1 FROM resume_profile WHERE is_active)"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;

    // Seed skills follow the (possibly newly promoted) active resume.
    crate::orchestrator::on_resume_parsed(&database.pool, &app).await;

    println!("🗑️ Deleted resume profile {}", id);
    Ok(())
}

#[tauri::command]
pub async fn list_resumes(
    database: State<'_, Database>,
) -> Result<Vec<ResumeHeader>, String> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, label, name, is_active, created_at, updated_at \
         FROM resume_profile ORDER BY created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(ResumeHeader {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            label: r.try_get("label").map_err(|e| e.to_string())?,
            name: r.try_get("name").map_err(|e| e.to_string())?,
            is_active: r.try_get("is_active").map_err(|e| e.to_string())?,
            created_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map(|d| d.to_rfc3339()).map_err(|e| e.to_string())?,
            updated_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
                .map(|d| d.to_rfc3339()).map_err(|e| e.to_string())?,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn set_active_resume_cmd(
    id: String,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<(), String> {
    // Flip active in one transaction: on failure the rollback restores the old
    // active, so we never strand the app with zero active resumes.
    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE resume_profile SET is_active = FALSE WHERE is_active")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let res = sqlx::query("UPDATE resume_profile SET is_active = TRUE WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err(format!("No resume with id {}", id));
    }
    tx.commit().await.map_err(|e| e.to_string())?;

    // Re-seed skills from the newly active resume + emit ygg-skills-updated.
    crate::orchestrator::on_resume_parsed(&database.pool, &app).await;

    println!("⭐ Active resume → {}", id);
    Ok(())
}

#[tauri::command]
pub async fn update_resume_label_cmd(
    id: String,
    label: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("UPDATE resume_profile SET label = $1 WHERE id = $2")
        .bind(&label)
        .bind(&id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    println!("🏷️ Renamed resume {} → {:?}", id, label);
    Ok(())
}
