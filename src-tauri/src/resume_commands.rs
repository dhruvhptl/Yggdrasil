// src-tauri/src/resume_commands.rs

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
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

async fn call_groq(api_key: &str, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();

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
        .post("https://api.groq.com/openai/v1/chat/completions")
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
    database: State<'_, Database>,
) -> Result<ResumeProfile, String> {
    println!("\n=== Parse Resume ===");
    println!("Text length: {} chars\n", text.len());

    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenv::from_path(&env_path).ok();
    dotenv::dotenv().ok();
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not found in .env file".to_string())?;

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
    let profile_json = call_groq(&api_key, system_prompt, &profile_prompt).await?;
    println!("Got profile response: {} chars", profile_json.len());

    let profile: ParsedProfileAI = serde_json::from_str(&profile_json)
        .map_err(|e| format!("Failed to parse profile AI response: {}. Raw: {}", e, &profile_json[..200.min(profile_json.len())]))?;

    // ── Call 2: Extract projects ────────────────────────────────────────────────
    println!("Calling Groq API — Call 2: project extraction…");
    let projects_prompt = format!(
        "Extract ALL projects from this resume. Include personal projects, course projects, \
        hackathon projects, open source contributions, and any other project work mentioned.\n\
        Return ONLY a JSON object with this exact shape:\n\
        {{\n  \"projects\": [\n    {{\n      \"name\": \"Project Name\",\n      \
        \"description\": \"What the project does and key accomplishments\",\n      \
        \"tech_stack\": [\"React\", \"Python\", \"etc\"],\n      \
        \"github_url\": \"https://github.com/... or null\"\n    }}\n  ]\n}}\n\
        Extract EVERY project mentioned anywhere in the resume. Be thorough — check \
        the projects section, work experience bullet points, and education section.\n\
        Return ONLY the JSON, nothing else.\n\nResume:\n{}",
        text
    );
    let projects_json = call_groq(&api_key, system_prompt, &projects_prompt).await?;
    println!("Got projects response: {} chars", projects_json.len());

    let projects_parsed: ParsedProjectsAI = serde_json::from_str(&projects_json)
        .map_err(|e| format!("Failed to parse projects AI response: {}. Raw: {}", e, &projects_json[..200.min(projects_json.len())]))?;

    // ── Save to database ────────────────────────────────────────────────────────

    // Delete any existing resume (single-profile model)
    sqlx::query("DELETE FROM resume_profile")
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let profile_id = Uuid::new_v4().to_string();
    let education_json = serde_json::to_value(&profile.education.unwrap_or_default()).unwrap();
    let work_exp_json = serde_json::to_value(&profile.work_experience.unwrap_or_default()).unwrap();
    let skills_json = serde_json::to_value(&profile.skills.unwrap_or_default()).unwrap();

    sqlx::query(
        "INSERT INTO resume_profile (id, raw_text, name, email, education, work_experience, skills) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(&profile_id)
    .bind(&text)
    .bind(&profile.name)
    .bind(&profile.email)
    .bind(&education_json)
    .bind(&work_exp_json)
    .bind(&skills_json)
    .execute(&database.pool)
    .await
    .map_err(|e| format!("Failed to save resume profile: {}", e))?;

    println!("✅ Saved resume profile: {}", profile_id);

    // Save projects
    let projects = projects_parsed.projects.unwrap_or_default();
    let mut saved_projects: Vec<ResumeProject> = Vec::new();

    for p in &projects {
        let project_id = Uuid::new_v4().to_string();
        let tech_stack_json = serde_json::to_value(&p.tech_stack).unwrap();
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

    Ok(ResumeProfile {
        id: profile_id,
        raw_text: text,
        name: profile.name,
        email: profile.email,
        education,
        work_experience,
        skills,
        projects: saved_projects,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn get_resume(
    database: State<'_, Database>,
) -> Result<Option<ResumeProfile>, String> {
    let profile_row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, serde_json::Value, serde_json::Value, serde_json::Value, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, raw_text, name, email, education, work_experience, skills, created_at, updated_at \
         FROM resume_profile ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = match profile_row {
        Some(r) => r,
        None => return Ok(None),
    };

    let profile_id = row.0.clone();

    // Fetch projects with linked project names
    let project_rows = sqlx::query_as::<_, (String, String, String, Option<String>, serde_json::Value, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)>(
        "SELECT rp.id, rp.resume_id, rp.name, rp.description, rp.tech_stack, rp.github_url, \
                rp.linked_project_id, rp.created_at \
         FROM resume_projects rp WHERE rp.resume_id = $1 ORDER BY rp.created_at"
    )
    .bind(&profile_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut projects: Vec<ResumeProject> = Vec::new();
    for pr in project_rows {
        let linked_name = if let Some(ref pid) = pr.6 {
            let name_row = sqlx::query_as::<_, (String,)>(
                "SELECT name FROM projects WHERE id = $1"
            )
            .bind(pid)
            .fetch_optional(&database.pool)
            .await
            .map_err(|e| e.to_string())?;
            name_row.map(|n| n.0)
        } else {
            None
        };

        projects.push(ResumeProject {
            id: pr.0,
            resume_id: pr.1,
            name: pr.2,
            description: pr.3,
            tech_stack: serde_json::from_value(pr.4).unwrap_or_default(),
            github_url: pr.5,
            linked_project_id: pr.6,
            linked_project_name: linked_name,
            created_at: pr.7.to_rfc3339(),
        });
    }

    let education: Vec<Education> = serde_json::from_value(row.4).unwrap_or_default();
    let work_experience: Vec<WorkExperience> = serde_json::from_value(row.5).unwrap_or_default();
    let skills: Vec<String> = serde_json::from_value(row.6).unwrap_or_default();

    Ok(Some(ResumeProfile {
        id: row.0,
        raw_text: row.1,
        name: row.2,
        email: row.3,
        education,
        work_experience,
        skills,
        projects,
        created_at: row.7.to_rfc3339(),
        updated_at: row.8.to_rfc3339(),
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
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM resume_profile")
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    println!("🗑️ Deleted resume profile");
    Ok(())
}
