// src/pages/JobsPage.tsx
// Job application tracker: Kanban board, analytics, follow-up tracker.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd';
import { ExternalLink, Plus, X, RefreshCw, ChevronDown, Calendar, Trash2 } from 'lucide-react';

// ─── Types ─────────────────────────────────────────────────────────────────────

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

interface JobApplication {
  id: string;
  company: string;
  position: string;
  location?: string;
  source?: string;
  status: string;
  dateApplied?: string;
  dateFollowUp?: string;
  jobDescription?: string;
  link?: string;
  notes?: string;
  ratingOverall?: number;
  ratingLocation?: number;
  ratingAlignment?: number;
  ratingSalary?: number;
  ratingRole?: number;
  season: string;
  createdAt: string;
  followUpDone: boolean;
}

interface JobSkill {
  id: string;
  jobId: string;
  skillName: string;
  isRequired: boolean;
}

interface SkillDemand {
  skillName: string;
  count: number;
  totalJobs: number;
  frequency: number;
  avgRating: number;
  demandScore: number;
}

interface WorkResourceSkill { id: string; skillName: string; }
interface WorkResource { id: string; skills: WorkResourceSkill[]; }
interface WorkTopic { id: string; resources: WorkResource[]; }
interface WorkCoop { id: string; topics: WorkTopic[]; }
interface WorkGraph { coops: WorkCoop[]; }

interface ReextractResult {
  processed: number;
  failed: number;
  errors: string[];
}

// ─── Constants ────────────────────────────────────────────────────────────────

const STATUSES = ['saved', 'applied', 'rejected', 'interviewing', 'offer'] as const;
type Status = typeof STATUSES[number];

const STATUS_LABELS: Record<Status, string> = {
  saved: 'Saved',
  applied: 'Applied',
  interviewing: 'Interviewing',
  offer: 'Offer',
  rejected: 'Rejected',
};

const STATUS_COLORS: Record<Status, string> = {
  saved: '#475569',
  applied: '#3b82f6',
  interviewing: '#f59e0b',
  offer: '#10b981',
  rejected: '#ef4444',
};

const DEFAULT_SEASONS = ['Winter 2026', 'Fall 2025', 'Summer 2025', 'Winter 2025'];

// ─── Helpers ─────────────────────────────────────────────────────────────────

function followUpBadge(job: JobApplication): string | null {
  if (!job.dateFollowUp) return null;
  const days = (new Date(job.dateFollowUp).getTime() - Date.now()) / 86400000;
  if (days < 0) return '🔴';
  if (days <= 7) return '🟡';
  return null;
}

function isoToDateInput(iso?: string): string {
  if (!iso) return '';
  return iso.slice(0, 10);
}

// Clicking an already-selected star clears the rating.
function StarRating({ value, onChange }: { value?: number | null; onChange: (v: number | null) => void }) {
  return (
    <div className="flex gap-0.5">
      {[1, 2, 3, 4, 5].map((n) => (
        <button
          key={n}
          onClick={() => onChange(value === n ? null : n)}
          className="text-base leading-none transition-opacity hover:opacity-80"
          style={{ opacity: (value != null) && n <= value ? 1 : 0.2, color: '#f59e0b' }}
        >
          ●
        </button>
      ))}
    </div>
  );
}

// Computes auto-overall from the four sub-ratings (average of whichever are set).
function computeOverall(
  loc?: number | null,
  align?: number | null,
  salary?: number | null,
  role?: number | null,
): number | null {
  const vals = [loc, align, salary, role].filter((v): v is number => v != null);
  if (vals.length === 0) return null;
  return Math.round((vals.reduce((a, b) => a + b, 0) / vals.length) * 10) / 10;
}

// Compact styled date input with calendar icon prefix.
// Uses defaultValue + onBlur so partial keystrokes don't fire updates mid-entry.
// key={value} resets the uncontrolled input when switching to a different job.
function DateField({ label, value, onChange }: { label: string; value?: string; onChange: (v: string | null) => void }) {
  return (
    <label className="block">
      <span className="text-slate-400 text-xs">{label}</span>
      <div className="mt-1 relative flex items-center bg-slate-800 border border-slate-700 rounded-lg overflow-hidden focus-within:border-emerald-600/50 focus-within:ring-1 focus-within:ring-emerald-600/20 transition-all">
        <Calendar className="absolute left-2.5 w-3 h-3 text-slate-500 pointer-events-none flex-shrink-0" />
        <input
          key={value ?? ''}
          type="date"
          defaultValue={isoToDateInput(value)}
          onBlur={(e) => onChange(e.target.value || null)}
          className="w-full bg-transparent pl-8 pr-2 py-1.5 text-white text-xs focus:outline-none [color-scheme:dark]"
        />
      </div>
    </label>
  );
}

// ─── AddJobForm ───────────────────────────────────────────────────────────────

interface AddJobFormProps {
  season: string;
  onCreated: (job: JobApplication) => void;
  onClose: () => void;
}

function AddJobForm({ season, onCreated, onClose }: AddJobFormProps) {
  const [company, setCompany] = useState('');
  const [position, setPosition] = useState('');
  const [location, setLocation] = useState('');
  const [source, setSource] = useState('');
  const [link, setLink] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!company.trim() || !position.trim()) {
      setError('Company and position are required.');
      return;
    }
    setSaving(true);
    try {
      const job = await invoke<JobApplication>('create_job', {
        company: company.trim(),
        position: position.trim(),
        location: location.trim() || null,
        source: source.trim() || null,
        link: link.trim() || null,
        season,
      });
      onCreated(job);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <form
        className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-md shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-white font-semibold text-lg">Add Job</h2>
          <button type="button" onClick={onClose} className="text-slate-400 hover:text-white"><X className="w-4 h-4" /></button>
        </div>
        {error && <p className="text-red-400 text-sm mb-3">{error}</p>}
        <div className="space-y-3">
          {[
            { label: 'Company *', value: company, set: setCompany, placeholder: 'Acme Corp' },
            { label: 'Position *', value: position, set: setPosition, placeholder: 'Software Engineer Intern' },
            { label: 'Location', value: location, set: setLocation, placeholder: 'San Francisco, CA / Remote' },
            { label: 'Source', value: source, set: setSource, placeholder: 'LinkedIn, referral, company site…' },
            { label: 'Link', value: link, set: setLink, placeholder: 'https://…' },
          ].map(({ label, value, set, placeholder }) => (
            <label key={label} className="block">
              <span className="text-slate-400 text-xs">{label}</span>
              <input
                type="text"
                value={value}
                onChange={(e) => set(e.target.value)}
                placeholder={placeholder}
                className="mt-1 w-full bg-slate-800 border border-slate-700 rounded-lg px-3 py-2 text-white text-sm placeholder-slate-500 focus:outline-none focus:border-slate-500"
              />
            </label>
          ))}
        </div>
        <div className="flex gap-2 mt-5">
          <button
            type="submit"
            disabled={saving}
            className="flex-1 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg py-2 text-sm font-medium transition-colors"
          >
            {saving ? 'Adding…' : 'Add Job'}
          </button>
          <button type="button" onClick={onClose} className="px-4 py-2 text-slate-400 hover:text-white text-sm">
            Cancel
          </button>
        </div>
      </form>
    </div>
  );
}

// ─── JobCard ──────────────────────────────────────────────────────────────────

function JobCard({
  job,
  onClick,
}: {
  job: JobApplication;
  onClick: () => void;
}) {
  const badge = followUpBadge(job);
  const overall = computeOverall(job.ratingLocation, job.ratingAlignment, job.ratingSalary, job.ratingRole);
  const rating = overall ?? 0;

  return (
    <div
      onClick={onClick}
      className="bg-slate-800 border border-slate-700 hover:border-slate-500 rounded-lg p-3 cursor-pointer transition-colors"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="text-white font-medium text-sm truncate">{job.company}</p>
          <p className="text-slate-400 text-xs truncate mt-0.5">{job.position}</p>
        </div>
        {badge && <span className="text-base leading-none flex-shrink-0">{badge}</span>}
      </div>
      <div className="flex items-center justify-between mt-2">
        <div className="flex gap-0.5">
          {[1, 2, 3, 4, 5].map((n) => (
            <span key={n} style={{ color: n <= rating ? '#f59e0b' : '#334155', fontSize: '10px' }}>●</span>
          ))}
        </div>
        {job.source && (
          <span className="text-xs bg-slate-700 text-slate-300 px-1.5 py-0.5 rounded">
            {job.source}
          </span>
        )}
      </div>
    </div>
  );
}

// ─── JobDetailPanel ────────────────────────────────────────────────────────────

interface DetailPanelProps {
  job: JobApplication;
  skills: JobSkill[];
  onClose: () => void;
  onUpdate: (updated: JobApplication) => void;
  onSkillsRefresh: (skills: JobSkill[]) => void;
  onDelete: (id: string) => void;
}

function JobDetailPanel({ job, skills, onClose, onUpdate, onSkillsRefresh, onDelete }: DetailPanelProps) {
  const [deleting, setDeleting] = useState(false);
  const [tailoredData, setTailoredData] = useState<TailoredProjects | null>(null);
  const [tailorLoading, setTailorLoading] = useState(false);
  const [tailorOpen, setTailorOpen] = useState(false);

  async function handleDelete() {
    if (!window.confirm(`Delete ${job.company} — ${job.position}?`)) return;
    setDeleting(true);
    try {
      await invoke('delete_job', { id: job.id });
      onDelete(job.id);
    } catch (err) {
      console.error('delete_job failed:', err);
    } finally {
      setDeleting(false);
    }
  }

  function handleCopyTailored() {
    if (!tailoredData) return;
    const text = tailoredData.topProjects
      .map(p => `• ${p.projectName}: ${p.projectDescription || 'N/A'} (${p.matchedSkills.join(', ')}); ${p.talkingPoints.join('; ')}`)
      .join('\n');
    navigator.clipboard.writeText(text).catch(console.error);
  }

  const [notes, setNotes] = useState(job.notes ?? '');
  const [jd, setJd] = useState(job.jobDescription ?? '');
  const [extracting, setExtracting] = useState(false);
  const notesTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Refs keep debounce callbacks honest — they always read the latest value
  // even when the React closure they were created in is stale.
  const jobRef = useRef(job);
  jobRef.current = job;
  const notesRef = useRef(job.notes ?? '');

  // Local sub-ratings for instant overall feedback before the debounce fires.
  const [localRatings, setLocalRatings] = useState({
    ratingLocation: job.ratingLocation ?? null,
    ratingAlignment: job.ratingAlignment ?? null,
    ratingSalary: job.ratingSalary ?? null,
    ratingRole: job.ratingRole ?? null,
  });
  const localRatingsRef = useRef(localRatings);

  const displayOverall = computeOverall(
    localRatings.ratingLocation, localRatings.ratingAlignment,
    localRatings.ratingSalary, localRatings.ratingRole,
  );

  // Sync when switching to a different job.
  useEffect(() => {
    const n = job.notes ?? '';
    setNotes(n);
    notesRef.current = n;
    setJd(job.jobDescription ?? '');
    const r = {
      ratingLocation: job.ratingLocation ?? null,
      ratingAlignment: job.ratingAlignment ?? null,
      ratingSalary: job.ratingSalary ?? null,
      ratingRole: job.ratingRole ?? null,
    };
    setLocalRatings(r);
    localRatingsRef.current = r;
  }, [job.id]);

  // Fetch tailored projects
  useEffect(() => {
    setTailorLoading(true);
    invoke<TailoredProjects>('get_tailored_projects', { jobId: job.id })
      .then(setTailoredData)
      .catch(console.error)
      .finally(() => setTailorLoading(false));
  }, [job.id]);

  // update() always merges patch on top of the full current state so that
  // a notes debounce never zeros out ratings and vice-versa.
  // IMPORTANT: ratingOverall must be rounded to integer — the DB column is
  // INTEGER and Rust expects Option<i32>. A float like 3.5 causes serde
  // deserialization failure, silently aborting the entire invoke.
  async function update(patch: Record<string, unknown>) {
    const j = jobRef.current;
    const r = localRatingsRef.current;
    const rawOverall = computeOverall(
      r.ratingLocation, r.ratingAlignment, r.ratingSalary, r.ratingRole,
    );
    try {
      const updated = await invoke<JobApplication>('update_job', {
        id: j.id,
        status: j.status,
        notes: notesRef.current || null,
        dateApplied: isoToDateInput(j.dateApplied) || null,
        dateFollowUp: isoToDateInput(j.dateFollowUp) || null,
        ratingOverall: rawOverall != null ? Math.round(rawOverall) : null,
        ratingLocation: r.ratingLocation,
        ratingAlignment: r.ratingAlignment,
        ratingSalary: r.ratingSalary,
        ratingRole: r.ratingRole,
        ...patch,
      });
      onUpdate(updated);
    } catch (err) {
      console.error('update_job failed:', err);
    }
  }

  function handleNotesChange(val: string) {
    setNotes(val);
    notesRef.current = val;
    clearTimeout(notesTimer.current);
    notesTimer.current = setTimeout(() => update({ notes: val || null }), 800);
  }

  function handleRating(field: string, val: number | null) {
    const newRatings = { ...localRatingsRef.current, [field]: val };
    setLocalRatings(newRatings);
    localRatingsRef.current = newRatings;
  }

  const isDirty =
    (localRatings.ratingLocation ?? null) !== (job.ratingLocation ?? null) ||
    (localRatings.ratingAlignment ?? null) !== (job.ratingAlignment ?? null) ||
    (localRatings.ratingSalary ?? null) !== (job.ratingSalary ?? null) ||
    (localRatings.ratingRole ?? null) !== (job.ratingRole ?? null);

  const [isSavingRatings, setIsSavingRatings] = useState(false);

  async function handleSaveRatings() {
    setIsSavingRatings(true);
    try {
      await update({});
    } finally {
      setIsSavingRatings(false);
    }
  }

  async function handleJdBlur() {
    if (jd === (job.jobDescription ?? '')) return;
    setExtracting(true);
    try {
      const newSkills = await invoke<JobSkill[]>('save_job_description', {
        id: job.id,
        jobDescription: jd,
      });
      onSkillsRefresh(newSkills);
      onUpdate({ ...job, jobDescription: jd });
    } catch (err) {
      console.error('save_job_description failed:', err);
    } finally {
      setExtracting(false);
    }
  }

  async function handleReExtract() {
    setExtracting(true);
    try {
      const newSkills = await invoke<JobSkill[]>('extract_job_skills', { id: job.id });
      onSkillsRefresh(newSkills);
    } catch (err) {
      console.error('extract_job_skills failed:', err);
    } finally {
      setExtracting(false);
    }
  }

  const required = skills.filter((s) => s.isRequired);
  const niceToHave = skills.filter((s) => !s.isRequired);

  return (
    <div className="fixed top-0 right-0 h-full w-96 bg-slate-900 border-l border-slate-700 flex flex-col z-40 overflow-hidden shadow-2xl">
      {/* Header */}
      <div className="flex items-start justify-between p-4 border-b border-slate-700 flex-shrink-0">
        <div className="min-w-0 flex-1 pr-2">
          <h2 className="text-white font-semibold truncate">{job.company}</h2>
          <p className="text-slate-400 text-sm truncate">{job.position}</p>
          <div className="flex items-center gap-2 mt-1.5 flex-wrap">
            <span
              className="text-xs px-2 py-0.5 rounded-full font-medium"
              style={{ backgroundColor: STATUS_COLORS[job.status as Status] + '33', color: STATUS_COLORS[job.status as Status] }}
            >
              {STATUS_LABELS[job.status as Status] ?? job.status}
            </span>
            <span className="text-xs bg-slate-800 text-slate-400 px-2 py-0.5 rounded-full">{job.season}</span>
            {displayOverall !== null && (
              <div className="flex items-center gap-1">
                <div className="flex gap-px">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <span key={n} style={{ fontSize: '8px', color: n <= Math.round(displayOverall) ? '#f59e0b' : '#334155' }}>●</span>
                  ))}
                </div>
                <span className="text-amber-400/80 text-xs font-medium tabular-nums">{displayOverall.toFixed(1)}</span>
              </div>
            )}
          </div>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          {job.link && (
            <a href={job.link} target="_blank" rel="noreferrer" className="text-slate-400 hover:text-blue-400 transition-colors">
              <ExternalLink className="w-4 h-4" />
            </a>
          )}
          <button onClick={handleDelete} disabled={deleting} title="Delete job" className="text-slate-400 hover:text-red-400 disabled:opacity-40 transition-colors">
            <Trash2 className="w-4 h-4" />
          </button>
          <button onClick={onClose} className="text-slate-400 hover:text-white transition-colors">
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Scrollable body */}
      <div className="flex-1 overflow-y-auto p-4 space-y-5">
        {/* Status */}
        <label className="block">
          <span className="text-slate-400 text-xs uppercase tracking-wide">Status</span>
          <select
            value={job.status}
            onChange={(e) => update({ status: e.target.value })}
            className="mt-1 w-full bg-slate-800 border border-slate-700 rounded-lg px-3 py-2 text-white text-sm focus:outline-none focus:border-slate-500"
          >
            {STATUSES.map((s) => (
              <option key={s} value={s}>{STATUS_LABELS[s]}</option>
            ))}
          </select>
        </label>

        {/* Ratings */}
        <div>
          <span className="text-slate-400 text-xs uppercase tracking-wide">Ratings</span>
          <div className="mt-2 space-y-2">
            {(
              [
                { label: 'Location',  field: 'ratingLocation',  value: localRatings.ratingLocation },
                { label: 'Alignment', field: 'ratingAlignment', value: localRatings.ratingAlignment },
                { label: 'Salary',    field: 'ratingSalary',    value: localRatings.ratingSalary },
                { label: 'Role',      field: 'ratingRole',      value: localRatings.ratingRole },
              ] as Array<{ label: string; field: string; value: number | null }>
            ).map(({ label, field, value }) => (
              <div key={field} className="flex items-center justify-between">
                <span className="text-slate-400 text-xs w-20">{label}</span>
                <StarRating value={value} onChange={(v) => handleRating(field, v)} />
              </div>
            ))}
          </div>
          {isDirty && (
            <button
              onClick={handleSaveRatings}
              disabled={isSavingRatings}
              className="mt-2 w-full py-1.5 rounded-lg text-xs font-medium bg-amber-500/20 text-amber-400 hover:bg-amber-500/30 disabled:opacity-50 transition-colors"
            >
              {isSavingRatings ? 'Saving…' : 'Save Ratings'}
            </button>
          )}
        </div>

        {/* Dates */}
        <div className="grid grid-cols-2 gap-3">
          <DateField
            label="Date Applied"
            value={job.dateApplied}
            onChange={(v) => update({ dateApplied: v })}
          />
          <DateField
            label="Follow-up"
            value={job.dateFollowUp}
            onChange={(v) => update({ dateFollowUp: v })}
          />
        </div>

        {/* Notes */}
        <label className="block">
          <span className="text-slate-400 text-xs uppercase tracking-wide">Notes</span>
          <textarea
            value={notes}
            onChange={(e) => handleNotesChange(e.target.value)}
            rows={3}
            placeholder="Any notes…"
            className="mt-1 w-full bg-slate-800 border border-slate-700 rounded-lg px-3 py-2 text-white text-sm placeholder-slate-500 focus:outline-none focus:border-slate-500 resize-none"
          />
        </label>

        {/* Job Description */}
        <div>
          <div className="flex items-center justify-between">
            <span className="text-slate-400 text-xs uppercase tracking-wide">Job Description</span>
            <button
              onClick={handleReExtract}
              disabled={extracting || !job.jobDescription}
              className="flex items-center gap-1 text-xs text-slate-400 hover:text-white disabled:opacity-40 transition-colors"
            >
              <RefreshCw className={`w-3 h-3 ${extracting ? 'animate-spin' : ''}`} />
              Re-extract
            </button>
          </div>
          <textarea
            value={jd}
            onChange={(e) => setJd(e.target.value)}
            onBlur={handleJdBlur}
            rows={6}
            placeholder="Paste job description here — skills will extract automatically on blur…"
            className="mt-1 w-full bg-slate-800 border border-slate-700 rounded-lg px-3 py-2 text-white text-xs placeholder-slate-500 focus:outline-none focus:border-slate-500 resize-none"
          />
          {extracting && (
            <p className="text-xs text-blue-400 mt-1">Extracting skills…</p>
          )}
        </div>

        {/* Skills */}
        {(required.length > 0 || niceToHave.length > 0) && (
          <div>
            <span className="text-slate-400 text-xs uppercase tracking-wide">Skills</span>
            {required.length > 0 && (
              <div className="flex flex-wrap gap-1.5 mt-2">
                {required.map((s) => (
                  <span key={s.id} className="text-xs bg-emerald-900/50 text-emerald-300 border border-emerald-700/50 px-2 py-0.5 rounded-full">
                    {s.skillName}
                  </span>
                ))}
              </div>
            )}
            {niceToHave.length > 0 && (
              <div className="flex flex-wrap gap-1.5 mt-1.5">
                {niceToHave.map((s) => (
                  <span key={s.id} className="text-xs bg-slate-700/50 text-slate-300 border border-slate-600/50 px-2 py-0.5 rounded-full">
                    {s.skillName}
                  </span>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Recommended Projects */}
        <div>
          <button
            onClick={() => setTailorOpen(v => !v)}
            className="flex items-center justify-between w-full text-xs text-slate-400 hover:text-white transition-colors"
          >
            <span className="font-medium uppercase tracking-wide text-[10px]">Recommended Projects</span>
            <ChevronDown className={`w-3 h-3 transition-transform ${tailorOpen ? 'rotate-180' : ''}`} />
          </button>

          {tailorOpen && (
            <div className="mt-2 space-y-2">
              {tailorLoading && <p className="text-slate-500 text-xs">Loading…</p>}
              {!tailorLoading && tailoredData && tailoredData.topProjects.length === 0 && (
                <p className="text-slate-500 text-xs">No matching projects found.</p>
              )}
              {!tailorLoading && tailoredData && tailoredData.topProjects.map((p, idx) => (
                <div key={idx} className="bg-slate-800/60 border border-slate-700 rounded-lg p-2.5">
                  <p className="text-white text-xs font-medium mb-1">{p.projectName}</p>
                  <div className="flex items-center gap-2 mb-1.5">
                    <div className="flex-1 bg-slate-700 rounded-full h-1.5">
                      <div
                        className="h-1.5 rounded-full"
                        style={{
                          width: `${Math.round(p.matchScore * 100)}%`,
                          backgroundColor: p.matchScore >= 0.66 ? '#10b981' : p.matchScore >= 0.33 ? '#f59e0b' : '#ef4444',
                        }}
                      />
                    </div>
                    <span className="text-[10px] text-slate-400 whitespace-nowrap">
                      {Math.round(p.matchScore * 100)}%
                    </span>
                  </div>
                  {p.matchedSkills.length > 0 && (
                    <div className="flex flex-wrap gap-1 mb-1">
                      {p.matchedSkills.map((s, si) => (
                        <span key={si} className="text-[10px] bg-emerald-900/60 text-emerald-300 px-1.5 py-0.5 rounded-full">{s}</span>
                      ))}
                    </div>
                  )}
                  {p.missingRequiredSkills.length > 0 && (
                    <div className="flex flex-wrap gap-1">
                      {p.missingRequiredSkills.slice(0, 3).map((s, si) => (
                        <span key={si} className="text-[10px] bg-amber-900/60 text-amber-300 px-1.5 py-0.5 rounded-full">{s}</span>
                      ))}
                    </div>
                  )}
                </div>
              ))}

              {tailoredData && tailoredData.topProjects.length > 0 && (
                <div className="flex gap-2 pt-1">
                  <button onClick={handleCopyTailored} className="flex-1 text-[10px] bg-slate-700 hover:bg-slate-600 text-white rounded py-1 transition-colors">
                    Copy
                  </button>
                  <button onClick={() => { window.location.href = '/resume'; }} className="flex-1 text-[10px] bg-slate-700 hover:bg-slate-600 text-white rounded py-1 transition-colors">
                    Resume
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── Board View ───────────────────────────────────────────────────────────────

interface BoardViewProps {
  jobs: JobApplication[];
  season: string;
  onJobClick: (job: JobApplication) => void;
  onJobsChange: (jobs: JobApplication[]) => void;
}

function BoardView({ jobs, season, onJobClick, onJobsChange }: BoardViewProps) {
  const [showAddForm, setShowAddForm] = useState(false);

  function handleDragEnd(result: DropResult) {
    const { destination, source, draggableId } = result;
    if (!destination) return;
    const newStatus = destination.droppableId as Status;
    if (newStatus === source.droppableId) return;

    const job = jobs.find((j) => j.id === draggableId);
    if (!job) return;

    // Optimistic update
    const updated = jobs.map((j) => j.id === draggableId ? { ...j, status: newStatus } : j);
    onJobsChange(updated);

    const dragOverall = computeOverall(job.ratingLocation, job.ratingAlignment, job.ratingSalary, job.ratingRole);
    invoke<JobApplication>('update_job', {
      id: draggableId,
      status: newStatus,
      notes: job.notes ?? null,
      dateApplied: isoToDateInput(job.dateApplied) || null,
      dateFollowUp: isoToDateInput(job.dateFollowUp) || null,
      ratingOverall: dragOverall != null ? Math.round(dragOverall) : null,
      ratingLocation: job.ratingLocation ?? null,
      ratingAlignment: job.ratingAlignment ?? null,
      ratingSalary: job.ratingSalary ?? null,
      ratingRole: job.ratingRole ?? null,
    }).then((j) => {
      onJobsChange(jobs.map((x) => x.id === j.id ? j : x));
    }).catch(console.error);
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-6 py-3 flex-shrink-0">
        <p className="text-slate-400 text-sm">{jobs.length} application{jobs.length !== 1 ? 's' : ''}</p>
        <button
          onClick={() => setShowAddForm(true)}
          className="flex items-center gap-1.5 bg-blue-600 hover:bg-blue-500 text-white text-sm px-3 py-1.5 rounded-lg transition-colors"
        >
          <Plus className="w-4 h-4" /> Add Job
        </button>
      </div>

      <div className="flex-1 overflow-x-auto px-6 pb-6">
        <DragDropContext onDragEnd={handleDragEnd}>
          <div className="flex gap-4 h-full min-w-max">
            {STATUSES.map((status) => {
              const col = jobs.filter((j) => j.status === status);
              return (
                <div key={status} className="flex flex-col w-64 flex-shrink-0">
                  <div
                    className="flex items-center gap-2 px-2 py-2 rounded-t-lg mb-1"
                    style={{ backgroundColor: STATUS_COLORS[status] + '22' }}
                  >
                    <div className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: STATUS_COLORS[status] }} />
                    <span className="text-sm font-medium" style={{ color: STATUS_COLORS[status] }}>{STATUS_LABELS[status]}</span>
                    <span className="text-xs text-slate-500 ml-auto">{col.length}</span>
                  </div>
                  <Droppable droppableId={status}>
                    {(provided, snapshot) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.droppableProps}
                        className="flex-1 space-y-2 p-2 rounded-b-lg rounded-tr-lg min-h-32 transition-colors"
                        style={{ backgroundColor: snapshot.isDraggingOver ? STATUS_COLORS[status] + '11' : 'transparent' }}
                      >
                        {col.map((job, index) => (
                          <Draggable key={job.id} draggableId={job.id} index={index}>
                            {(dragProvided, dragSnapshot) => (
                              <div
                                ref={dragProvided.innerRef}
                                {...dragProvided.draggableProps}
                                {...dragProvided.dragHandleProps}
                                style={{
                                  ...dragProvided.draggableProps.style,
                                  opacity: dragSnapshot.isDragging ? 0.85 : 1,
                                }}
                              >
                                <JobCard
                                  job={job}
                                  onClick={() => onJobClick(job)}
                                />
                              </div>
                            )}
                          </Draggable>
                        ))}
                        {provided.placeholder}
                      </div>
                    )}
                  </Droppable>
                </div>
              );
            })}
          </div>
        </DragDropContext>
      </div>

      {showAddForm && (
        <AddJobForm
          season={season}
          onCreated={(job) => {
            onJobsChange([job, ...jobs]);
            setShowAddForm(false);
          }}
          onClose={() => setShowAddForm(false)}
        />
      )}
    </div>
  );
}

// ─── Analytics View ────────────────────────────────────────────────────────────

interface AnalyticsViewProps {
  jobs: JobApplication[];
  skillDemand: SkillDemand[];
  workSkillNames: Set<string>;
  onSkillsRefreshed: () => void;
}

function AnalyticsView({ jobs, skillDemand, workSkillNames, onSkillsRefreshed }: AnalyticsViewProps) {
  const [reextracting, setReextracting] = useState(false);
  const [reextractResult, setReextractResult] = useState<ReextractResult | null>(null);

  const jobsWithJd = jobs.filter((j) => j.jobDescription?.trim()).length;

  async function handleReextractAll() {
    setReextracting(true);
    setReextractResult(null);
    try {
      const result = await invoke<ReextractResult>('reextract_all_skills');
      setReextractResult(result);
    } catch (err) {
      setReextractResult({ processed: 0, failed: 1, errors: [String(err)] });
    } finally {
      setReextracting(false);
    }
  }

  function handleCloseModal() {
    setReextractResult(null);
    onSkillsRefreshed();
  }

  const maxDemand = Math.max(...skillDemand.map((s) => s.demandScore), 0.01);

  const gapSkills = skillDemand.filter(
    (s) => s.frequency >= 0.3 && s.avgRating >= 3 && !workSkillNames.has(s.skillName.toLowerCase())
  );

  // Source breakdown
  const sourceCounts: Record<string, number> = {};
  for (const job of jobs) {
    const key = job.source?.trim() || 'Unknown';
    sourceCounts[key] = (sourceCounts[key] ?? 0) + 1;
  }
  const sources = Object.entries(sourceCounts).sort((a, b) => b[1] - a[1]);
  const maxSource = Math.max(...sources.map(([, n]) => n), 1);

  // Status breakdown
  const statusCounts: Record<string, number> = {};
  for (const job of jobs) statusCounts[job.status] = (statusCounts[job.status] ?? 0) + 1;

  return (
    <div className="p-6 space-y-8 overflow-y-auto h-full">
      {/* Re-extract progress modal */}
      {(reextracting || reextractResult) && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-sm shadow-2xl">
            {reextracting ? (
              <>
                <div className="flex items-center gap-3 mb-2">
                  <RefreshCw className="w-5 h-5 text-blue-400 animate-spin" />
                  <h3 className="text-white font-semibold">Re-extracting Skills</h3>
                </div>
                <p className="text-slate-400 text-sm">
                  Processing {jobsWithJd} job{jobsWithJd !== 1 ? 's' : ''} with job descriptions…
                </p>
                <p className="text-slate-500 text-xs mt-2">
                  500ms delay between jobs to avoid rate limits.
                </p>
              </>
            ) : reextractResult && (
              <>
                <div className="flex items-center gap-3 mb-3">
                  <div className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold ${reextractResult.failed === 0 ? 'bg-emerald-900/50 text-emerald-400' : 'bg-amber-900/50 text-amber-400'}`}>
                    ✓
                  </div>
                  <p className="text-white font-semibold text-sm">
                    {reextractResult.processed} processed{reextractResult.failed > 0 ? `, ${reextractResult.failed} failed` : ''}
                  </p>
                </div>
                {reextractResult.errors.length > 0 && (
                  <div className="bg-red-950/30 border border-red-800/40 rounded-lg p-3 mb-4 space-y-1 max-h-40 overflow-y-auto">
                    {reextractResult.errors.map((e, i) => (
                      <p key={i} className="text-red-400 text-xs">{e}</p>
                    ))}
                  </div>
                )}
                <button
                  onClick={handleCloseModal}
                  className="w-full py-2 bg-slate-700 hover:bg-slate-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Close & Refresh
                </button>
              </>
            )}
          </div>
        </div>
      )}

      {/* Status summary */}
      <div>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-slate-300 font-medium">Status Breakdown</h3>
          <button
            onClick={handleReextractAll}
            disabled={reextracting || jobsWithJd === 0}
            className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            title={jobsWithJd === 0 ? 'No jobs with job descriptions' : `Re-extract skills for ${jobsWithJd} jobs`}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${reextracting ? 'animate-spin' : ''}`} />
            Re-extract All Skills
          </button>
        </div>
        <div className="flex gap-3 flex-wrap">
          {STATUSES.map((s) => (
            <div
              key={s}
              className="px-3 py-2 rounded-lg border text-center min-w-16"
              style={{ borderColor: STATUS_COLORS[s] + '55', backgroundColor: STATUS_COLORS[s] + '18' }}
            >
              <div className="text-xl font-bold" style={{ color: STATUS_COLORS[s] }}>
                {statusCounts[s] ?? 0}
              </div>
              <div className="text-xs text-slate-400">{STATUS_LABELS[s]}</div>
            </div>
          ))}
        </div>
      </div>

      {/* Skill Demand */}
      {skillDemand.length > 0 && (
        <div>
          <h3 className="text-slate-300 font-medium mb-3">Skill Demand (top 20)</h3>
          <div className="space-y-2">
            {skillDemand.map((s) => (
              <div key={s.skillName} className="flex items-center gap-3">
                <span className="text-slate-300 text-sm w-36 truncate flex-shrink-0">{s.skillName}</span>
                <div className="flex-1 h-2 bg-slate-800 rounded-full overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all"
                    style={{
                      width: `${(s.demandScore / maxDemand) * 100}%`,
                      backgroundColor: '#3b82f6',
                    }}
                  />
                </div>
                <span className="text-slate-500 text-xs w-12 text-right flex-shrink-0">
                  {Math.round((s.demandScore / maxDemand) * 100)}%
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Skills Gap */}
      {gapSkills.length > 0 && (
        <div>
          <h3 className="text-slate-300 font-medium mb-2">Skills Gap</h3>
          <p className="text-slate-500 text-xs mb-3">High-demand skills (≥30% frequency, avg rating ≥3) not covered in your work graph.</p>
          <div className="flex flex-wrap gap-2">
            {gapSkills.map((s) => (
              <span key={s.skillName} className="text-sm bg-amber-900/30 text-amber-300 border border-amber-700/40 px-2.5 py-1 rounded-full">
                {s.skillName}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Source breakdown */}
      {sources.length > 0 && (
        <div>
          <h3 className="text-slate-300 font-medium mb-3">Source Breakdown</h3>
          <div className="space-y-2">
            {sources.map(([source, count]) => (
              <div key={source} className="flex items-center gap-3">
                <span className="text-slate-300 text-sm w-32 truncate flex-shrink-0">{source}</span>
                <div className="flex-1 h-2 bg-slate-800 rounded-full overflow-hidden">
                  <div
                    className="h-full rounded-full"
                    style={{ width: `${(count / maxSource) * 100}%`, backgroundColor: '#8b5cf6' }}
                  />
                </div>
                <span className="text-slate-500 text-xs w-6 text-right flex-shrink-0">{count}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {jobs.length === 0 && (
        <p className="text-slate-500 text-sm">No jobs tracked yet — add some from the Board view.</p>
      )}
    </div>
  );
}

// ─── Follow-ups View ──────────────────────────────────────────────────────────

interface FollowUpsViewProps {
  jobs: JobApplication[];
  onJobClick: (job: JobApplication) => void;
  onMarked: (id: string) => void;
}

function FollowUpsView({ jobs, onJobClick, onMarked }: FollowUpsViewProps) {
  const [marking, setMarking] = useState<string | null>(null);

  const relevant = jobs
    .filter((j) => !j.followUpDone && (j.dateFollowUp || j.status === 'applied' || j.status === 'interviewing'))
    .sort((a, b) => {
      const ta = a.dateFollowUp ? new Date(a.dateFollowUp).getTime() : Infinity;
      const tb = b.dateFollowUp ? new Date(b.dateFollowUp).getTime() : Infinity;
      return ta - tb;
    });

  async function handleMark(id: string) {
    setMarking(id);
    try {
      await invoke('mark_followed_up', { id });
      onMarked(id);
    } catch (err) {
      console.error(err);
    } finally {
      setMarking(null);
    }
  }

  if (relevant.length === 0) {
    return (
      <div className="p-6">
        <p className="text-slate-500 text-sm">No follow-ups needed right now.</p>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-2 overflow-y-auto h-full">
      {relevant.map((job) => {
        const badge = followUpBadge(job);
        const isOverdue = badge === '🔴';
        const isThisWeek = badge === '🟡';

        return (
          <div
            key={job.id}
            className="flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors"
            style={{
              backgroundColor: isOverdue ? 'rgba(127,29,29,0.25)' : isThisWeek ? 'rgba(120,53,15,0.25)' : 'rgba(30,41,59,0.5)',
              borderColor: isOverdue ? '#7f1d1d' : isThisWeek ? '#78350f' : '#334155',
            }}
            onClick={() => onJobClick(job)}
          >
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                {badge && <span>{badge}</span>}
                <p className="text-white text-sm font-medium truncate">{job.company}</p>
                <span
                  className="text-xs px-1.5 py-0.5 rounded flex-shrink-0"
                  style={{ backgroundColor: STATUS_COLORS[job.status as Status] + '33', color: STATUS_COLORS[job.status as Status] }}
                >
                  {STATUS_LABELS[job.status as Status] ?? job.status}
                </span>
              </div>
              <p className="text-slate-400 text-xs truncate mt-0.5">{job.position}</p>
              {job.dateFollowUp && (
                <p className={`text-xs mt-1 ${isOverdue ? 'text-red-400' : isThisWeek ? 'text-amber-400' : 'text-slate-500'}`}>
                  Follow-up: {new Date(job.dateFollowUp).toLocaleDateString()}
                </p>
              )}
            </div>
            <button
              onClick={(e) => { e.stopPropagation(); handleMark(job.id); }}
              disabled={marking === job.id}
              className="flex-shrink-0 text-xs bg-slate-700 hover:bg-slate-600 disabled:opacity-50 text-slate-300 px-2.5 py-1 rounded-lg transition-colors"
            >
              {marking === job.id ? '…' : 'Mark Done'}
            </button>
          </div>
        );
      })}
    </div>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

type View = 'board' | 'analytics' | 'followups';

export default function JobsPage() {
  const [view, setView] = useState<View>('board');
  const [selectedSeason, setSelectedSeason] = useState<string>(DEFAULT_SEASONS[0]);
  const [allSeasons, setAllSeasons] = useState<string[]>(DEFAULT_SEASONS);
  const [jobs, setJobs] = useState<JobApplication[]>([]);
  const [skillDemand, setSkillDemand] = useState<SkillDemand[]>([]);
  const [workSkillNames, setWorkSkillNames] = useState<Set<string>>(new Set());
  const [selectedJob, setSelectedJob] = useState<JobApplication | null>(null);
  const [selectedJobSkills, setSelectedJobSkills] = useState<JobSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [showSeasonMenu, setShowSeasonMenu] = useState(false);

  const loadAll = useCallback(async () => {
    setLoading(true);
    try {
      const [j, sd, wg] = await Promise.all([
        invoke<JobApplication[]>('get_jobs', { season: selectedSeason }),
        invoke<SkillDemand[]>('get_skill_demand', { season: selectedSeason }),
        invoke<WorkGraph>('get_full_work_graph'),
      ]);
      setJobs(j);
      setSkillDemand(sd);
      // Merge seasons from all loaded jobs with defaults, preserving order
      const allJobSeasons = invoke<JobApplication[]>('get_jobs', { season: null })
        .catch(() => [] as JobApplication[])
        .then((all: JobApplication[]) => {
          const seen = new Set<string>();
          const merged: string[] = [];
          for (const s of DEFAULT_SEASONS) { if (!seen.has(s)) { seen.add(s); merged.push(s); } }
          for (const job of all) { if (!seen.has(job.season)) { seen.add(job.season); merged.push(job.season); } }
          setAllSeasons(merged);
        });
      void allJobSeasons;

      // Extract all skill names from work graph
      const names = new Set<string>();
      for (const coop of wg.coops) {
        for (const topic of coop.topics) {
          for (const resource of topic.resources) {
            for (const skill of resource.skills) {
              names.add(skill.skillName.toLowerCase());
            }
          }
        }
      }
      setWorkSkillNames(names);
    } catch (err) {
      console.error('loadAll error:', err);
    } finally {
      setLoading(false);
    }
  }, [selectedSeason]);

  useEffect(() => { loadAll(); }, [loadAll]);

  // Load skills when a job is selected
  useEffect(() => {
    if (!selectedJob) { setSelectedJobSkills([]); return; }
    invoke<JobSkill[]>('get_job_skills', { jobId: selectedJob.id })
      .then(setSelectedJobSkills)
      .catch(console.error);
  }, [selectedJob?.id]);

  function handleJobUpdate(updated: JobApplication) {
    setJobs((prev) => prev.map((j) => j.id === updated.id ? updated : j));
    setSelectedJob(updated);
  }

  function handleJobDelete(id: string) {
    setJobs((prev) => prev.filter((j) => j.id !== id));
    setSelectedJob(null);
  }

  function handleMarkedFollowUp(_id: string) {
    // Reload to get fresh date_follow_up = NOW()
    invoke<JobApplication[]>('get_jobs', { season: selectedSeason })
      .then(setJobs)
      .catch(console.error);
  }

  const tabs: Array<{ id: View; label: string }> = [
    { id: 'board', label: 'Board' },
    { id: 'analytics', label: 'Analytics' },
    { id: 'followups', label: 'Follow-ups' },
  ];

  return (
    <div className="flex flex-col h-full bg-slate-950 text-white">
      {/* Top bar */}
      <div className="flex items-center gap-4 px-6 py-4 border-b border-slate-800 flex-shrink-0">
        <h1 className="text-lg font-semibold">Jobs</h1>

        {/* Tab bar */}
        <div className="flex bg-slate-800/50 rounded-lg p-0.5">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setView(tab.id)}
              className="px-4 py-1.5 rounded-md text-sm transition-colors"
              style={{
                backgroundColor: view === tab.id ? 'rgb(51,65,85)' : 'transparent',
                color: view === tab.id ? 'white' : '#94a3b8',
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>

        <div className="ml-auto relative">
          <button
            onClick={() => setShowSeasonMenu((v) => !v)}
            className="flex items-center gap-1.5 bg-slate-800 border border-slate-700 hover:border-slate-500 text-slate-300 text-sm px-3 py-1.5 rounded-lg transition-colors"
          >
            {selectedSeason}
            <ChevronDown className="w-3.5 h-3.5" />
          </button>
          {showSeasonMenu && (
            <div className="absolute right-0 top-full mt-1 bg-slate-800 border border-slate-700 rounded-lg shadow-xl z-10 min-w-40 py-1">
              {allSeasons.map((s) => (
                <button
                  key={s}
                  onClick={() => { setSelectedSeason(s); setShowSeasonMenu(false); }}
                  className="w-full text-left px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-700 transition-colors"
                  style={{ fontWeight: s === selectedSeason ? 600 : 400 }}
                >
                  {s}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Content */}
      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <div className="text-slate-400 text-sm animate-pulse">Loading…</div>
        </div>
      ) : (
        <div className="flex-1 min-h-0">
          {view === 'board' && (
            <BoardView
              jobs={jobs}
              season={selectedSeason}
              onJobClick={(job) => setSelectedJob(job)}
              onJobsChange={setJobs}
            />
          )}
          {view === 'analytics' && (
            <AnalyticsView
              jobs={jobs}
              skillDemand={skillDemand}
              workSkillNames={workSkillNames}
              onSkillsRefreshed={loadAll}
            />
          )}
          {view === 'followups' && (
            <FollowUpsView
              jobs={jobs}
              onJobClick={(job) => setSelectedJob(job)}
              onMarked={handleMarkedFollowUp}
            />
          )}
        </div>
      )}

      {/* Detail panel */}
      {selectedJob && (
        <JobDetailPanel
          job={selectedJob}
          skills={selectedJobSkills}
          onClose={() => setSelectedJob(null)}
          onUpdate={handleJobUpdate}
          onSkillsRefresh={setSelectedJobSkills}
          onDelete={handleJobDelete}
        />
      )}
    </div>
  );
}
