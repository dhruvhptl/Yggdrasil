# Tailored Projects Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Tailor Resume" button to job cards that opens a side panel showing the user's top 3 resume projects matched to the job's required skills, with match scores and talking points to inform resume tailoring decisions.

**Architecture:** New `get_tailored_projects` read-model command in Rust loads job required skills and resume projects, computes match scores, and returns top 3. New `TailoredProjectsPanel` React component displays results. JobsPage manages panel state and invokes the command on button click.

**Tech Stack:** Rust (sqlx, serde), React 19, TypeScript, lucide-react icons

---

### Task 1: Add structs and command to read_models.rs

**Files:**
- Modify: `src-tauri/src/read_models.rs` — add structs at end of file + new command

- [ ] **Step 1: Read the end of read_models.rs to find insertion point**

Read the file to see where to add new structs and command (after existing structs, before or at end).

- [ ] **Step 2: Add TailoredProject and TailoredProjects structs**

Add these structs after the last existing struct in `read_models.rs`:

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProject {
    pub project_name: String,
    pub project_description: Option<String>,
    pub yggdrasil_project_id: Option<String>,
    pub matched_skills: Vec<String>,
    pub missing_required_skills: Vec<String>,
    pub match_score: f32,
    pub talking_points: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProjects {
    pub job_id: String,
    pub company: String,
    pub position: String,
    pub required_skills_count: i32,
    pub top_projects: Vec<TailoredProject>,
}
```

- [ ] **Step 3: Add get_tailored_projects command**

Add this command at the end of the file, before any closing braces:

```rust
#[tauri::command]
pub async fn get_tailored_projects(
    job_id: String,
    database: State<'_, Database>,
) -> Result<TailoredProjects, String> {
    use crate::job_commands::JobApplication;
    use serde_json::{json, Value};

    // Load job metadata
    let job_row = sqlx::query(
        "SELECT company, position FROM job_applications WHERE id = $1"
    )
    .bind(&job_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let (company, position) = match job_row {
        None => return Err(format!("Job {} not found", job_id)),
        Some(r) => (
            r.try_get::<String, _>("company").map_err(|e| e.to_string())?,
            r.try_get::<String, _>("position").map_err(|e| e.to_string())?,
        ),
    };

    // Load required skills for this job
    let skills_rows = sqlx::query(
        "SELECT skill_name FROM job_skills WHERE job_id = $1 AND is_required = true"
    )
    .bind(&job_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let required_skills: Vec<String> = skills_rows
        .iter()
        .map(|r| {
            r.try_get::<String, _>("skill_name")
                .map(|s| s.to_lowercase())
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let required_skills_count = required_skills.len() as i32;

    // Load resume profile (most recent)
    let resume_row = sqlx::query("SELECT parsed FROM resume_profile ORDER BY created_at DESC LIMIT 1")
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let resume_projects: Vec<(String, Option<String>, Vec<String>, Option<String>)> = match resume_row {
        None => vec![],
        Some(r) => {
            let parsed: Value = r.try_get("parsed").map_err(|e| e.to_string())?;
            let projects = parsed.get("projects").and_then(|p| p.as_array()).unwrap_or(&vec![]);
            projects
                .iter()
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?.to_string();
                    let desc = p.get("description").and_then(|d| d.as_str()).map(String::from);
                    let tech_stack: Vec<String> = p
                        .get("techStack")
                        .or_else(|| p.get("tech_stack"))
                        .and_then(|ts| ts.as_array())
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_lowercase()))
                        .collect();
                    let linked_id = p.get("linkedProjectId")
                        .or_else(|| p.get("linked_project_id"))
                        .and_then(|id| id.as_str())
                        .map(String::from);
                    Some((name, desc, tech_stack, linked_id))
                })
                .collect()
        }
    };

    // Match projects to job skills
    let mut tailored: Vec<TailoredProject> = resume_projects
        .iter()
        .map(|(name, desc, tech_stack, linked_id)| {
            let tech_lower: Vec<String> = tech_stack.iter().map(|s| s.to_lowercase()).collect();
            let matched_skills: Vec<String> = required_skills
                .iter()
                .filter(|rs| tech_lower.contains(rs))
                .cloned()
                .collect();
            let missing_required_skills: Vec<String> = required_skills
                .iter()
                .filter(|rs| !tech_lower.contains(rs))
                .cloned()
                .collect();
            let match_score = if required_skills.is_empty() {
                0.0
            } else {
                matched_skills.len() as f32 / required_skills.len() as f32
            };
            let talking_points: Vec<String> = matched_skills
                .iter()
                .map(|s| format!("Demonstrates {} through {}", s, name))
                .collect();

            TailoredProject {
                project_name: name.clone(),
                project_description: desc.clone(),
                yggdrasil_project_id: linked_id.clone(),
                matched_skills,
                missing_required_skills,
                match_score,
                talking_points,
            }
        })
        .collect();

    // Sort by match_score DESC, take top 3
    tailored.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap_or(std::cmp::Ordering::Equal));
    tailored.truncate(3);

    Ok(TailoredProjects {
        job_id,
        company,
        position,
        required_skills_count,
        top_projects: tailored,
    })
}
```

- [ ] **Step 4: Verify no compile errors**

Run: `cd "C:\Users\dhruv\projects\Yggdrasil" && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -50`

Expected: Should compile with no errors (warnings are OK)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/read_models.rs
git commit -m "feat: add get_tailored_projects command structs"
```

---

### Task 2: Register command in main.rs

**Files:**
- Modify: `src-tauri/src/main.rs` — add command to invoke_handler

- [ ] **Step 1: Find the invoke_handler section**

Locate the `.invoke_handler(tauri::generate_handler![` section in main.rs and note the last command before the closing `])`.

- [ ] **Step 2: Add get_tailored_projects to invoke_handler**

In the `tauri::generate_handler![ ... ]` list, add this line (insert before the closing `]`):

```rust
read_models::get_tailored_projects,
```

The full invoke_handler should have this command registered alongside other read_models commands like `get_active_tree_for_project`.

- [ ] **Step 3: Verify syntax**

Run: `cd "C:\Users\dhruv\projects\Yggdrasil" && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -50`

Expected: Should compile with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat: register get_tailored_projects command"
```

---

### Task 3: Create TailoredProjectsPanel component

**Files:**
- Create: `src/components/TailoredProjectsPanel.tsx`

- [ ] **Step 1: Create the component file with header and types**

Create `src/components/TailoredProjectsPanel.tsx` with:

```typescript
import React, { useState } from 'react';
import { X, ExternalLink } from 'lucide-react';

interface TailoredProject {
  projectName: string;
  projectDescription?: string;
  yggdrasilProjectId?: string;
  matchedSkills: string[];
  missingRequiredSkills: string[];
  matchScore: number;
  talkingPoints: string[];
}

interface TailoredProjects {
  jobId: string;
  company: string;
  position: string;
  requiredSkillsCount: number;
  topProjects: TailoredProject[];
}

interface TailoredProjectsPanelProps {
  job: { id: string; company: string; position: string };
  tailoredData: TailoredProjects;
  loading?: boolean;
  onClose: () => void;
}

export function TailoredProjectsPanel({
  job,
  tailoredData,
  loading = false,
  onClose,
}: TailoredProjectsPanelProps) {
  const [copyNotification, setCopyNotification] = useState(false);

  function handleCopyToClipboard() {
    const text = tailoredData.topProjects
      .map((p) => `• ${p.projectName}: ${p.projectDescription || 'N/A'} (${p.matchedSkills.join(', ')}); ${p.talkingPoints.join('; ')}`)
      .join('\n');
    navigator.clipboard.writeText(text);
    setCopyNotification(true);
    setTimeout(() => setCopyNotification(false), 2000);
  }

  function handleOpenResume() {
    window.location.href = '/resume';
  }

  return (
    <div className="fixed right-0 top-0 h-full w-96 bg-slate-900 border-l border-slate-700 shadow-2xl z-40 flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b border-slate-700">
        <div>
          <p className="text-white font-semibold text-sm">{job.company}</p>
          <p className="text-slate-400 text-xs">{job.position}</p>
        </div>
        <button onClick={onClose} className="text-slate-400 hover:text-white">
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4">
        {loading ? (
          <p className="text-slate-400 text-sm">Loading...</p>
        ) : tailoredData.topProjects.length === 0 ? (
          <p className="text-slate-400 text-sm">No resume projects matched.</p>
        ) : (
          <div className="space-y-3">
            {tailoredData.topProjects.map((project, idx) => (
              <div key={idx} className="bg-slate-800 border border-slate-700 rounded-lg p-3">
                {/* Project Name + Yggdrasil Link */}
                <div className="flex items-center gap-2 mb-2">
                  <p className="text-white font-medium text-sm">{project.projectName}</p>
                  {project.yggdrasilProjectId && (
                    <a
                      href={`/projects/${project.yggdrasilProjectId}`}
                      className="text-blue-400 hover:text-blue-300"
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  )}
                </div>

                {/* Match Score Bar */}
                <div className="mb-2">
                  <div className="flex items-center justify-between mb-1">
                    <span className="text-xs text-slate-400">
                      {Math.round(project.matchScore * 100)}% match ({project.matchedSkills.length}/{tailoredData.requiredSkillsCount} skills)
                    </span>
                  </div>
                  <div className="w-full bg-slate-700 rounded-full h-2">
                    <div
                      className="h-2 rounded-full transition-all"
                      style={{
                        width: `${project.matchScore * 100}%`,
                        backgroundColor: project.matchScore < 0.33 ? '#ef4444' : project.matchScore < 0.66 ? '#f59e0b' : '#10b981',
                      }}
                    />
                  </div>
                </div>

                {/* Matched Skills */}
                {project.matchedSkills.length > 0 && (
                  <div className="mb-2">
                    <p className="text-xs text-slate-400 mb-1">Matched skills</p>
                    <div className="flex flex-wrap gap-1">
                      {project.matchedSkills.map((skill, i) => (
                        <span key={i} className="bg-emerald-900 text-emerald-100 text-xs px-2 py-0.5 rounded-full">
                          {skill}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Missing Skills */}
                {project.missingRequiredSkills.length > 0 && (
                  <div className="mb-2">
                    <p className="text-xs text-slate-400 mb-1">Missing skills</p>
                    <div className="flex flex-wrap gap-1">
                      {project.missingRequiredSkills.map((skill, i) => (
                        <span key={i} className="bg-amber-900 text-amber-100 text-xs px-2 py-0.5 rounded-full">
                          {skill}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Talking Points */}
                {project.talkingPoints.length > 0 && (
                  <div>
                    <p className="text-xs text-slate-400 mb-1">Talking points</p>
                    <ul className="text-xs text-slate-300 space-y-0.5 pl-4 list-disc">
                      {project.talkingPoints.map((point, i) => (
                        <li key={i}>{point}</li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Bottom Buttons */}
      <div className="border-t border-slate-700 p-4 space-y-2">
        {copyNotification && (
          <p className="text-xs text-emerald-400 mb-2">Copied to clipboard!</p>
        )}
        <button
          onClick={handleCopyToClipboard}
          className="w-full bg-blue-600 hover:bg-blue-500 text-white rounded-lg py-2 text-xs font-medium transition-colors"
        >
          Copy to clipboard
        </button>
        <button
          onClick={handleOpenResume}
          className="w-full bg-slate-700 hover:bg-slate-600 text-white rounded-lg py-2 text-xs font-medium transition-colors"
        >
          Open resume
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify no TypeScript errors**

Run: `cd "C:\Users\dhruv\projects\Yggdrasil" && npx tsc --noEmit 2>&1 | head -30`

Expected: No errors related to TailoredProjectsPanel.

- [ ] **Step 3: Commit**

```bash
git add src/components/TailoredProjectsPanel.tsx
git commit -m "feat: create TailoredProjectsPanel component"
```

---

### Task 4: Add button and state to JobsPage

**Files:**
- Modify: `src/pages/JobsPage.tsx` — add state + invoke command + panel rendering

- [ ] **Step 1: Add import for TailoredProjectsPanel and types**

At the top of JobsPage.tsx (after existing imports), add:

```typescript
import { TailoredProjectsPanel } from '../components/TailoredProjectsPanel';

interface TailoredProject {
  projectName: string;
  projectDescription?: string;
  yggdrasilProjectId?: string;
  matchedSkills: string[];
  missingRequiredSkills: string[];
  matchScore: number;
  talkingPoints: string[];
}

interface TailoredProjects {
  jobId: string;
  company: string;
  position: string;
  requiredSkillsCount: number;
  topProjects: TailoredProject[];
}
```

- [ ] **Step 2: Add state variables to main JobsPage component**

Find the main `export default function JobsPage()` component and add these state variables near the top (after existing state declarations):

```typescript
const [tailorPanelOpen, setTailorPanelOpen] = useState(false);
const [tailorJob, setTailorJob] = useState<JobApplication | null>(null);
const [tailoredProjects, setTailoredProjects] = useState<TailoredProjects | null>(null);
const [tailorLoading, setTailorLoading] = useState(false);
const [tailorError, setTailorError] = useState('');
```

- [ ] **Step 3: Add handler function for "Tailor Resume" click**

Add this function near other event handlers in JobsPage:

```typescript
async function handleTailorClick(job: JobApplication) {
  setTailorJob(job);
  setTailorLoading(true);
  setTailorError('');
  try {
    const data = await invoke<TailoredProjects>('get_tailored_projects', {
      jobId: job.id,
    });
    setTailoredProjects(data);
    setTailorPanelOpen(true);
  } catch (err) {
    setTailorError(String(err));
    console.error('get_tailored_projects failed:', err);
  } finally {
    setTailorLoading(false);
  }
}
```

- [ ] **Step 4: Update JobCard render calls to pass onTailorClick**

Find where JobCard components are rendered in the main component (inside the Kanban board columns). Update each `<JobCard>` to include the handler:

```typescript
<JobCard 
  job={job} 
  onClick={() => setSelectedJob(job)}
  onTailorClick={() => handleTailorClick(job)}
/>
```

- [ ] **Step 5: Update JobCard component to accept and use onTailorClick prop**

Find the `function JobCard()` component definition and update its signature:

```typescript
function JobCard({ 
  job, 
  onClick,
  onTailorClick,
}: { 
  job: JobApplication; 
  onClick: () => void;
  onTailorClick?: () => void;
}) {
```

Then add this button after the existing rating stars (in the JobCard JSX, after the star rating div):

```typescript
{onTailorClick && (
  <button
    onClick={(e) => {
      e.stopPropagation();
      onTailorClick();
    }}
    className="mt-2 w-full bg-slate-700 hover:bg-slate-600 text-white rounded text-xs py-1 transition-colors"
  >
    Tailor Resume
  </button>
)}
```

- [ ] **Step 6: Add TailoredProjectsPanel rendering to main JobsPage component**

Add this conditional render at the end of the JobsPage return JSX (after the main Kanban section, before the closing tags):

```typescript
{tailorPanelOpen && tailorJob && tailoredProjects && (
  <TailoredProjectsPanel
    job={{ id: tailorJob.id, company: tailorJob.company, position: tailorJob.position }}
    tailoredData={tailoredProjects}
    loading={tailorLoading}
    onClose={() => setTailorPanelOpen(false)}
  />
)}
```

- [ ] **Step 7: Verify no TypeScript errors**

Run: `cd "C:\Users\dhruv\projects\Yggdrasil" && npx tsc --noEmit 2>&1 | head -30`

Expected: No errors related to JobsPage or TailoredProjectsPanel.

- [ ] **Step 8: Commit**

```bash
git add src/pages/JobsPage.tsx
git commit -m "feat: add tailor resume button and panel state to JobsPage"
```

---

### Task 5: Manual testing in dev environment

**Files:**
- None (manual testing only)

- [ ] **Step 1: Start dev environment**

Run: `bash dev.sh`

Wait for the app to build and launch. Both the React dev server and Tauri backend should be running.

- [ ] **Step 2: Navigate to Jobs page**

In the running app, click on "Jobs" in the sidebar to open the Jobs page.

- [ ] **Step 3: Create a test job if needed**

If no jobs exist, click "Add Job" and create one with:
- Company: "Test Corp"
- Position: "Software Engineer"
- Click "Add Job"

- [ ] **Step 4: Add required skills to the job**

Click on the newly created job card to open the detail panel. Scroll down to "Job Skills" section and click "Add Skill" a few times to add skills like: "React", "TypeScript", "Node.js"

- [ ] **Step 5: Check that you have a resume profile**

Navigate to the Resume page. If no resume exists, paste sample resume text with projects that mention some skills (e.g., "Built a React dashboard using TypeScript").

- [ ] **Step 6: Test "Tailor Resume" button**

Go back to Jobs page. On your test job card, you should now see a "Tailor Resume" button. Click it.

Expected: TailoredProjectsPanel should open on the right side of the screen showing matched projects with:
- Project name
- Match score percentage
- Green chips for matched skills
- Amber chips for missing skills
- Bullet points for talking points

- [ ] **Step 7: Test copy to clipboard**

In the TailoredProjectsPanel, click "Copy to clipboard". A toast notification should appear saying "Copied to clipboard!"

Paste the clipboard content into a text editor to verify it's formatted correctly as a bullet list.

- [ ] **Step 8: Test "Open resume" button**

In the TailoredProjectsPanel, click "Open resume". The page should navigate to `/resume`.

- [ ] **Step 9: Test close button**

Navigate back to Jobs page. Click the TailoredProjectsPanel close button (X icon). The panel should disappear.

- [ ] **Step 10: Commit**

If all tests pass:

```bash
git add -A
git commit -m "test: manual testing complete, all features working"
```

---

## Self-Review Checklist

✅ **Spec Coverage:**
- Task 1: `get_tailored_projects` command with TailoredProject + TailoredProjects structs ✓
- Task 2: Command registered in main.rs ✓
- Task 3: TailoredProjectsPanel component with all UI elements (header, project cards, match bar, skills chips, talking points, buttons) ✓
- Task 4: JobCard "Tailor Resume" button + JobsPage state management + panel rendering ✓
- Task 5: Manual testing all features ✓

✅ **Placeholder Scan:** No TBD, TODO, or incomplete sections. All code is complete and ready to execute.

✅ **Type Consistency:** 
- TailoredProject, TailoredProjects defined in Rust match frontend TypeScript interfaces ✓
- All field names use camelCase via `#[serde(rename_all = "camelCase")]` ✓
- Match score is f32 in Rust, number in TypeScript ✓

✅ **Scope Check:** Feature is focused, self-contained, and independent. No unrelated refactoring included.
