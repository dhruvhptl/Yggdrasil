// src/pages/ResumePage.tsx
import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useNavigate } from "react-router-dom";
import {
  FileText, Upload, Loader2, ExternalLink, TreePine, Link2, Unlink,
  GraduationCap, Briefcase, ChevronDown, ChevronRight, Trash2,
} from "lucide-react";

// ─── Types ──────────────────────────────────────────────────────────────────

interface Education {
  institution: string;
  degree: string;
  year: string;
}

interface WorkExperience {
  company: string;
  role: string;
  start: string;
  end: string;
  description: string;
}

interface ResumeProject {
  id: string;
  resumeId: string;
  name: string;
  description: string | null;
  techStack: string[];
  githubUrl: string | null;
  linkedProjectId: string | null;
  linkedProjectName: string | null;
  createdAt: string;
}

interface ResumeProfile {
  id: string;
  rawText: string;
  name: string | null;
  email: string | null;
  education: Education[];
  workExperience: WorkExperience[];
  skills: string[];
  projects: ResumeProject[];
  createdAt: string;
  updatedAt: string;
}

interface YggProject {
  id: string;
  name: string;
  description: string;
  disciplineIds: string[];
  skillIds: string[];
  status: string;
  createdAt: string;
  progress: number;
}

// ─── Main Page ──────────────────────────────────────────────────────────────

export default function ResumePage() {
  const [resume, setResume] = useState<ResumeProfile | null>(null);
  const [loading, setLoading] = useState(true);
  const [projects, setProjects] = useState<YggProject[]>([]);

  async function loadResume() {
    try {
      const r = await invoke<ResumeProfile | null>("get_resume");
      setResume(r);
    } catch (e) {
      console.error("Failed to load resume:", e);
    } finally {
      setLoading(false);
    }
  }

  async function loadProjects() {
    try {
      const p = await invoke<YggProject[]>("get_projects");
      setProjects(p);
    } catch (e) {
      console.error("Failed to load projects:", e);
    }
  }

  useEffect(() => {
    loadResume();
    loadProjects();
  }, []);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-6 h-6 text-emerald-400 animate-spin" />
      </div>
    );
  }

  if (!resume) {
    return <EmptyState onParsed={(r) => { setResume(r); loadProjects(); }} />;
  }

  return (
    <LoadedState
      resume={resume}
      projects={projects}
      onRefresh={() => { loadResume(); loadProjects(); }}
    />
  );
}

// ─── Empty State ────────────────────────────────────────────────────────────

function EmptyState({ onParsed }: { onParsed: (r: ResumeProfile) => void }) {
  const [mode, setMode] = useState<"text" | "pdf">("text");
  const [text, setText] = useState("");
  const [parsing, setParsing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pdfFile, setPdfFile] = useState<File | null>(null);
  const [extractingPdf, setExtractingPdf] = useState(false);
  const pdfInputRef = useRef<HTMLInputElement>(null);

  async function handleParse() {
    let resumeText = text;

    // If PDF mode, extract full text via Mimir sidecar (no ingestion, just text extraction)
    if (mode === "pdf" && pdfFile) {
      setExtractingPdf(true);
      try {
        const base64 = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => {
            const result = reader.result as string;
            resolve(result.split(",")[1]);
          };
          reader.onerror = reject;
          reader.readAsDataURL(pdfFile);
        });

        const pdfResult = await invoke<{
          text: string;
          pages: number;
          chars: number;
        }>("extract_pdf_text", { pdfBase64: base64 });

        resumeText = pdfResult.text;
      } catch (e) {
        setError(`PDF extraction failed: ${e}. Try pasting the text instead.`);
        setExtractingPdf(false);
        return;
      } finally {
        setExtractingPdf(false);
      }
    }

    if (!resumeText.trim()) {
      setError("Please paste your resume text or upload a PDF.");
      return;
    }

    setParsing(true);
    setError(null);
    try {
      const result = await invoke<ResumeProfile>("parse_resume", {
        text: resumeText,
      });
      onParsed(result);
      // Sync resume skills to universal skills (fire-and-forget)
      invoke('sync_skills_from_resume').catch(console.warn);
    } catch (e) {
      setError(String(e));
    } finally {
      setParsing(false);
    }
  }

  return (
    <div className="h-full flex items-center justify-center p-8">
      <div className="max-w-2xl w-full">
        <div className="text-center mb-8">
          <FileText className="w-12 h-12 text-emerald-400 mx-auto mb-3" />
          <h1 className="text-2xl font-bold text-slate-100 mb-2">
            Resume Parser
          </h1>
          <p className="text-slate-400 text-sm">
            Paste your resume or upload a PDF. AI will extract your projects,
            skills, and experience — then link them to your Yggdrasil skill
            trees.
          </p>
        </div>

        {/* Mode toggle */}
        <div className="flex mb-4 rounded-lg overflow-hidden border border-slate-700">
          {(
            [
              ["text", "📝 Paste Text"],
              ["pdf", "📄 Upload PDF"],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              onClick={() => setMode(key)}
              className={`flex-1 py-2 text-sm font-medium transition-colors ${
                mode === key
                  ? "bg-slate-800 text-slate-100"
                  : "bg-transparent text-slate-500 hover:text-slate-300"
              }`}
            >
              {label}
            </button>
          ))}
        </div>

        {mode === "text" ? (
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Paste your full resume text here..."
            className="w-full h-64 bg-slate-900 border border-slate-700 rounded-lg p-4 text-sm text-slate-200 placeholder-slate-600 resize-vertical focus:outline-none focus:border-emerald-500/50"
          />
        ) : (
          <div className="mb-4">
            <input
              ref={pdfInputRef}
              type="file"
              accept=".pdf"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) setPdfFile(file);
              }}
              style={{ display: "none" }}
            />
            {pdfFile ? (
              <div className="flex items-center gap-3 bg-slate-900 border border-slate-700 rounded-lg p-4">
                <FileText className="w-8 h-8 text-emerald-400 flex-shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="text-sm text-slate-200 truncate">
                    {pdfFile.name}
                  </div>
                  <div className="text-xs text-slate-500">
                    {(pdfFile.size / 1024).toFixed(1)} KB
                  </div>
                </div>
                <button
                  onClick={() => {
                    setPdfFile(null);
                    if (pdfInputRef.current) pdfInputRef.current.value = "";
                  }}
                  className="text-red-400 hover:text-red-300 text-sm"
                >
                  ✕
                </button>
              </div>
            ) : (
              <button
                onClick={() => pdfInputRef.current?.click()}
                className="w-full h-32 border-2 border-dashed border-slate-700 rounded-lg flex flex-col items-center justify-center gap-2 hover:border-emerald-500/50 transition-colors cursor-pointer"
              >
                <Upload className="w-6 h-6 text-slate-500" />
                <span className="text-sm text-slate-500">
                  Click to choose a PDF file
                </span>
              </button>
            )}
          </div>
        )}

        {error && (
          <div className="mt-3 text-sm text-red-400 bg-red-400/10 border border-red-400/20 rounded-lg px-4 py-2">
            {error}
          </div>
        )}

        <button
          onClick={handleParse}
          disabled={
            parsing ||
            extractingPdf ||
            (mode === "text" ? !text.trim() : !pdfFile)
          }
          className="mt-4 w-full py-3 bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 disabled:text-slate-500 text-white font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
        >
          {extractingPdf ? (
            <>
              <Loader2 className="w-4 h-4 animate-spin" /> Extracting PDF
              text…
            </>
          ) : parsing ? (
            <>
              <Loader2 className="w-4 h-4 animate-spin" /> Parsing with AI…
            </>
          ) : (
            <>
              <FileText className="w-4 h-4" /> Parse Resume
            </>
          )}
        </button>
      </div>
    </div>
  );
}

// ─── Loaded State ───────────────────────────────────────────────────────────

function LoadedState({
  resume,
  projects,
  onRefresh,
}: {
  resume: ResumeProfile;
  projects: YggProject[];
  onRefresh: () => void;
}) {
  const navigate = useNavigate();
  const [expandedSections, setExpandedSections] = useState<Set<string>>(
    new Set(["projects", "experience", "education"])
  );
  const [deleting, setDeleting] = useState(false);

  function toggleSection(s: string) {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      next.has(s) ? next.delete(s) : next.add(s);
      return next;
    });
  }

  async function handleDelete() {
    if (!confirm("Delete your parsed resume? This cannot be undone.")) return;
    setDeleting(true);
    try {
      await invoke("delete_resume");
      onRefresh();
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="h-full overflow-auto">
      <div className="max-w-4xl mx-auto p-8">
        {/* Profile Header */}
        <div className="flex items-start justify-between mb-8">
          <div>
            <h1 className="text-2xl font-bold text-slate-100 mb-1">
              {resume.name || "Your Resume"}
            </h1>
            {resume.email && (
              <p className="text-sm text-slate-400">{resume.email}</p>
            )}
            {resume.skills.length > 0 && (
              <div className="flex flex-wrap gap-2 mt-3">
                {resume.skills.map((skill) => (
                  <span
                    key={skill}
                    className="px-2 py-0.5 text-xs font-medium rounded-full bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                  >
                    {skill}
                  </span>
                ))}
              </div>
            )}
          </div>
          <button
            onClick={handleDelete}
            disabled={deleting}
            className="text-slate-500 hover:text-red-400 transition-colors p-2"
            title="Delete resume and re-parse"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>

        {/* Projects Section */}
        <Section
          title="Projects"
          icon={<TreePine className="w-4 h-4 text-emerald-400" />}
          count={resume.projects.length}
          expanded={expandedSections.has("projects")}
          onToggle={() => toggleSection("projects")}
        >
          {resume.projects.length === 0 ? (
            <p className="text-sm text-slate-500 py-4">
              No projects found in resume.
            </p>
          ) : (
            <div className="space-y-3">
              {resume.projects.map((rp) => (
                <ProjectCard
                  key={rp.id}
                  project={rp}
                  yggProjects={projects}
                  onRefresh={onRefresh}
                  onNavigate={(name) => navigate(`/?prefill=${encodeURIComponent(name)}`)}
                />
              ))}
            </div>
          )}
        </Section>

        {/* Work Experience */}
        <Section
          title="Work Experience"
          icon={<Briefcase className="w-4 h-4 text-blue-400" />}
          count={resume.workExperience.length}
          expanded={expandedSections.has("experience")}
          onToggle={() => toggleSection("experience")}
        >
          {resume.workExperience.length === 0 ? (
            <p className="text-sm text-slate-500 py-4">
              No work experience found.
            </p>
          ) : (
            <div className="relative ml-3 border-l border-slate-700 pl-6 space-y-6">
              {resume.workExperience.map((exp, i) => (
                <div key={i} className="relative">
                  <div className="absolute -left-[31px] top-1 w-3 h-3 rounded-full bg-blue-500 border-2 border-slate-950" />
                  <div className="text-sm font-semibold text-slate-100">
                    {exp.role}
                  </div>
                  <div className="text-xs text-slate-400 mb-1">
                    {exp.company} · {exp.start} – {exp.end}
                  </div>
                  <p className="text-xs text-slate-400 leading-relaxed">
                    {exp.description}
                  </p>
                </div>
              ))}
            </div>
          )}
        </Section>

        {/* Education */}
        <Section
          title="Education"
          icon={<GraduationCap className="w-4 h-4 text-amber-400" />}
          count={resume.education.length}
          expanded={expandedSections.has("education")}
          onToggle={() => toggleSection("education")}
        >
          {resume.education.length === 0 ? (
            <p className="text-sm text-slate-500 py-4">
              No education found.
            </p>
          ) : (
            <div className="relative ml-3 border-l border-slate-700 pl-6 space-y-5">
              {resume.education.map((edu, i) => (
                <div key={i} className="relative">
                  <div className="absolute -left-[31px] top-1 w-3 h-3 rounded-full bg-amber-500 border-2 border-slate-950" />
                  <div className="text-sm font-semibold text-slate-100">
                    {edu.degree}
                  </div>
                  <div className="text-xs text-slate-400">
                    {edu.institution} · {edu.year}
                  </div>
                </div>
              ))}
            </div>
          )}
        </Section>
      </div>
    </div>
  );
}

// ─── Section wrapper ────────────────────────────────────────────────────────

function Section({
  title,
  icon,
  count,
  expanded,
  onToggle,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-6">
      <button
        onClick={onToggle}
        className="w-full flex items-center gap-2 py-3 border-b border-slate-800 hover:border-slate-700 transition-colors"
      >
        {expanded ? (
          <ChevronDown className="w-4 h-4 text-slate-500" />
        ) : (
          <ChevronRight className="w-4 h-4 text-slate-500" />
        )}
        {icon}
        <span className="text-sm font-semibold text-slate-200">{title}</span>
        <span className="text-xs text-slate-500 ml-1">({count})</span>
      </button>
      {expanded && <div className="pt-4">{children}</div>}
    </div>
  );
}

// ─── Project Card ───────────────────────────────────────────────────────────

function ProjectCard({
  project,
  yggProjects,
  onRefresh,
  onNavigate,
}: {
  project: ResumeProject;
  yggProjects: YggProject[];
  onRefresh: () => void;
  onNavigate: (name: string) => void;
}) {
  const [selectedProjectId, setSelectedProjectId] = useState("");
  const [linking, setLinking] = useState(false);

  async function handleLink() {
    if (!selectedProjectId) return;
    setLinking(true);
    try {
      await invoke("link_resume_project", {
        resumeProjectId: project.id,
        projectId: selectedProjectId,
      });
      onRefresh();
    } finally {
      setLinking(false);
    }
  }

  async function handleUnlink() {
    setLinking(true);
    try {
      await invoke("unlink_resume_project", {
        resumeProjectId: project.id,
      });
      onRefresh();
    } finally {
      setLinking(false);
    }
  }

  return (
    <div className="bg-slate-900/60 border border-slate-800 rounded-lg p-4">
      <div className="flex items-start justify-between mb-2">
        <div>
          <h3 className="text-sm font-semibold text-slate-100">
            {project.name}
          </h3>
          {project.description && (
            <p className="text-xs text-slate-400 mt-1 leading-relaxed">
              {project.description}
            </p>
          )}
        </div>
        {project.githubUrl && (
          <a
            href={project.githubUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-slate-500 hover:text-emerald-400 transition-colors flex-shrink-0 ml-3"
          >
            <ExternalLink className="w-3.5 h-3.5" />
          </a>
        )}
      </div>

      {/* Tech stack badges */}
      {project.techStack.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mb-3">
          {project.techStack.map((tech) => (
            <span
              key={tech}
              className="px-2 py-0.5 text-[10px] font-medium rounded bg-slate-800 text-slate-400 border border-slate-700"
            >
              {tech}
            </span>
          ))}
        </div>
      )}

      {/* Link status */}
      {project.linkedProjectId ? (
        <div className="flex items-center gap-2 mt-2">
          <div className="flex items-center gap-1.5 text-xs text-emerald-400">
            <TreePine className="w-3 h-3" />
            <span>
              Linked to{" "}
              <span className="font-semibold">
                {project.linkedProjectName || "project"}
              </span>
            </span>
          </div>
          <button
            onClick={handleUnlink}
            disabled={linking}
            className="text-slate-500 hover:text-red-400 transition-colors ml-auto"
            title="Unlink project"
          >
            <Unlink className="w-3 h-3" />
          </button>
        </div>
      ) : (
        <div className="flex items-center gap-2 mt-2">
          <select
            value={selectedProjectId}
            onChange={(e) => setSelectedProjectId(e.target.value)}
            className="flex-1 bg-slate-800 border border-slate-700 rounded text-xs text-slate-300 py-1.5 px-2 focus:outline-none focus:border-emerald-500/50"
          >
            <option value="">Select a project…</option>
            {yggProjects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <button
            onClick={handleLink}
            disabled={!selectedProjectId || linking}
            className="px-3 py-1.5 text-xs font-medium bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 disabled:text-slate-500 text-white rounded transition-colors flex items-center gap-1"
          >
            <Link2 className="w-3 h-3" />
            {linking ? "…" : "Link"}
          </button>
          <button
            onClick={() => onNavigate(project.name)}
            className="px-3 py-1.5 text-xs font-medium bg-slate-700 hover:bg-slate-600 text-slate-300 rounded transition-colors flex items-center gap-1"
            title="Create a new Yggdrasil project for this"
          >
            <TreePine className="w-3 h-3" />
            Generate Tree
          </button>
        </div>
      )}
    </div>
  );
}
