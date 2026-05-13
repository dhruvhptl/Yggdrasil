# Tailored Projects Feature — Design Spec

**Goal:** Quick-reference side panel showing top 3 resume projects matched to a job's required skills, enabling resume tailoring decisions.

**Use Case:** User reviews a job application, sees which of their resume projects best match the job's core requirements, and uses this to decide which projects to highlight/reorder when tailoring their resume for that position.

**Architecture:** New Tauri read-model command in `read_models.rs` + new React side panel component. No new database tables or migrations.

---

## 1. Backend: `get_tailored_projects` Command

**File:** `src-tauri/src/read_models.rs`

### Structs

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProject {
    pub project_name: String,
    pub project_description: Option<String>,
    pub yggdrasil_project_id: Option<String>,  // linked_project_id from resume_projects
    pub matched_skills: Vec<String>,           // intersection of project skills & required job skills
    pub missing_required_skills: Vec<String>,  // job required skills NOT in project
    pub match_score: f32,                      // matched_skills.len() / required_skills.len()
    pub talking_points: Vec<String>,           // "Demonstrates {skill} through {project_name}"
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProjects {
    pub job_id: String,
    pub company: String,
    pub position: String,
    pub required_skills_count: i32,            // total required skills for this job
    pub top_projects: Vec<TailoredProject>,    // top 3 by match_score DESC
}
```

### Command Signature

```rust
#[tauri::command]
pub async fn get_tailored_projects(
    job_id: String,
    database: State<'_, Database>,
) -> Result<TailoredProjects, String>
```

### Logic

1. **Load job metadata** — Query `job_applications` for company + position
2. **Load required skills** — Query `job_skills WHERE job_id = $1 AND is_required = true`, collect skill names (case-insensitive normalized for comparison)
3. **Load resume projects** — Query `resume_profile` table (most recent by created_at), parse JSONB `projects` array; each project has `{ name, description?, tech_stack: [...], linked_project_id? }`
4. **Match projects to job skills:**
   - For each resume project:
     - `matched_skills = intersection(project.tech_stack, required_skills)` using case-insensitive comparison
     - `missing_skills = required_skills - matched_skills`
     - `match_score = matched_skills.len() as f32 / required_skills.len() as f32`
     - `talking_points = matched_skills.map(|s| format!("Demonstrates {} through {}", s, project.name))`
5. **Sort & return** — Sort by match_score DESC, take top 3; return `TailoredProjects` struct

### Edge Cases

- No resume profile exists → return empty `top_projects` with `required_skills_count` set
- No required skills for job → return `required_skills_count = 0`, `top_projects` empty (all projects have score 1.0, not meaningful)
- Fewer than 3 projects in resume → return all of them sorted

---

## 2. Frontend: TailoredProjectsPanel Component

**File:** Create `src/components/TailoredProjectsPanel.tsx`

### Props

```typescript
interface TailoredProjectsPanelProps {
  job: JobApplication;
  tailoredData: TailoredProjects;
  loading?: boolean;
  onClose: () => void;
}
```

### Layout

**Header:**
- Job title + company in a bold header row
- Close button (X icon, top-right)

**Project Cards (stacked vertically):**
For each project in `top_projects`:
- **Project name** — text + optional Yggdrasil link (lucide `ExternalLink` icon) if `yggdrasil_project_id` exists
- **Match score bar** — visual progress bar showing `match_score * 100` as percentage, color gradient (red→yellow→green based on 0-100%)
- **Match score text** — e.g., "75% match (3/4 skills)"
- **Matched skills** — green chips, one per skill
- **Missing required skills** — amber chips, one per skill (only if > 0)
- **Talking points** — unordered list of bullet points (1-3 points per project)

**Bottom buttons:**
- "Copy to clipboard" — formats the 3 projects as a markdown bullet list: `• Project Name: description (matched skills); talking points` → copies to clipboard, toast notification "Copied!"
- "Open resume" — navigates to `/resume` page (use `window.location.href`)

### Styling

- Dark theme matching rest of app (slate-800/900 backgrounds, slate-400 text)
- Panel width: 420px fixed, scrollable if content exceeds viewport height
- Card spacing: 12px between project cards
- Chip padding: 4px 8px, rounded-full, green (#10b981) for matched, amber (#f59e0b) for missing

---

## 3. Frontend: JobCard Integration

**File:** Modify `src/pages/JobsPage.tsx` — `JobCard` component

Add a "Tailor Resume" button below the existing rating display:
- Button text: "Tailor Resume"
- Icon: `Sparkles` (lucide-react) or `Zap`
- Styling: inline with rating, text-xs, bg-slate-700 hover:bg-slate-600
- onClick handler: calls parent-provided callback to open TailoredProjectsPanel

### JobCard Props

Add to existing `JobCardProps`:
```typescript
onTailorClick?: () => void;  // callback when "Tailor Resume" is clicked
```

---

## 4. Frontend: JobsPage State Management

**File:** Modify `src/pages/JobsPage.tsx` — main component

Add state:
```typescript
const [tailorPanelOpen, setTailorPanelOpen] = useState(false);
const [tailorJob, setTailorJob] = useState<JobApplication | null>(null);
const [tailoredProjects, setTailoredProjects] = useState<TailoredProjects | null>(null);
const [tailorLoading, setTailorLoading] = useState(false);
const [tailorError, setTailorError] = useState('');
```

Handler when "Tailor Resume" is clicked on a job:
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
  } finally {
    setTailorLoading(false);
  }
}
```

Render TailoredProjectsPanel when `tailorPanelOpen && tailorJob && tailoredProjects`:
```typescript
{tailorPanelOpen && tailorJob && tailoredProjects && (
  <TailoredProjectsPanel
    job={tailorJob}
    tailoredData={tailoredProjects}
    loading={tailorLoading}
    onClose={() => setTailorPanelOpen(false)}
  />
)}
```

Pass `onTailorClick` to each `<JobCard>` in the render loop.

---

## 5. Registration

**File:** Modify `src-tauri/src/main.rs` — add `get_tailored_projects` to invoke_handler

```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    read_models::get_tailored_projects,  // ADD THIS LINE
    // ... more commands ...
])
```

---

## 6. What Stays Unchanged

- `JobApplication`, `JobSkill`, `JobDetailPanel` components — no changes
- Job CRUD, skill extraction, analytics endpoints — no changes
- Database schema — no new tables or columns
- Resume page — no changes (panel just navigates to it)

---

## 7. Error Handling

- If `get_tailored_projects` fails (network, DB error): display error message in panel, offer retry button
- If no resume exists: show "No resume profile found" message in panel
- If no required skills in job: show "No required skills extracted for this job" message
- Toast notifications for "Copied!" on copy-to-clipboard action

---

## 8. Testing Checklist

- [ ] Command loads correct job + required skills only
- [ ] Case-insensitive skill matching works
- [ ] Match score calculation is accurate
- [ ] Top 3 sorting by match_score DESC works
- [ ] Panel opens/closes correctly
- [ ] Yggdrasil links only show when `linked_project_id` is set
- [ ] Copy-to-clipboard formats and copies correctly
- [ ] "Open resume" navigation works
- [ ] Error states display appropriate messages
