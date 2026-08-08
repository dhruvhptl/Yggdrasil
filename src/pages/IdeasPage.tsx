// src/pages/IdeasPage.tsx
// Scratchpad: capture ideas, tag them, pin important ones, promote to projects.

import React, { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { Pin, Trash2, GitBranch, Plus, X, Check } from 'lucide-react';

// ─── Types ────────────────────────────────────────────────────────────────────

interface Idea {
  id: string;
  content: string;
  tag: string;
  pinned: boolean;
  createdAt: string;
}

// ─── Constants ────────────────────────────────────────────────────────────────

const TAGS = ['project_idea', 'resource', 'learning', 'random'] as const;
type Tag = typeof TAGS[number];

const TAG_LABELS: Record<Tag, string> = {
  project_idea: 'Project Idea',
  resource: 'Resource',
  learning: 'Learning',
  random: 'Random',
};

const TAG_STYLES: Record<Tag, { pill: string; card: string }> = {
  project_idea: {
    pill: 'bg-emerald-900/50 text-emerald-300 border border-emerald-700/40',
    card: 'border-emerald-800/40 hover:border-emerald-600/60',
  },
  resource: {
    pill: 'bg-blue-900/50 text-blue-300 border border-blue-700/40',
    card: 'border-blue-800/40 hover:border-blue-600/60',
  },
  learning: {
    pill: 'bg-violet-900/50 text-violet-300 border border-violet-700/40',
    card: 'border-violet-800/40 hover:border-violet-600/60',
  },
  random: {
    pill: 'bg-slate-700/60 text-slate-300 border border-slate-600/40',
    card: 'border-slate-700/60 hover:border-slate-500/60',
  },
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

function timeAgo(iso: string): string {
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}d ago`;
  return new Date(iso).toLocaleDateString();
}

// ─── TagPicker ────────────────────────────────────────────────────────────────

function TagPicker({ value, onChange }: { value: Tag; onChange: (t: Tag) => void }) {
  return (
    <div className="flex gap-1.5 flex-wrap">
      {TAGS.map((t) => (
        <button
          key={t}
          onClick={() => onChange(t)}
          className={`text-xs px-2.5 py-1 rounded-full border transition-all ${
            value === t
              ? TAG_STYLES[t].pill + ' opacity-100'
              : 'bg-transparent text-slate-500 border-slate-700 hover:border-slate-500 hover:text-slate-300'
          }`}
        >
          {TAG_LABELS[t]}
        </button>
      ))}
    </div>
  );
}

// ─── ComposeArea ─────────────────────────────────────────────────────────────

interface ComposeProps {
  onCreated: (idea: Idea) => void;
}

function ComposeArea({ onCreated }: ComposeProps) {
  const [open, setOpen] = useState(false);
  const [content, setContent] = useState('');
  const [tag, setTag] = useState<Tag>('random');
  const [saving, setSaving] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  function handleOpen() {
    setOpen(true);
    setTimeout(() => textareaRef.current?.focus(), 50);
  }

  function handleDiscard() {
    setContent('');
    setTag('random');
    setOpen(false);
  }

  async function handleSave() {
    if (!content.trim()) return;
    setSaving(true);
    try {
      const idea = await invoke<Idea>('create_idea', { content: content.trim(), tag });
      onCreated(idea);
      setContent('');
      setTag('random');
      setOpen(false);
    } catch (err) {
      console.error('create_idea failed:', err);
    } finally {
      setSaving(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') handleSave();
    if (e.key === 'Escape') handleDiscard();
  }

  if (!open) {
    return (
      <button
        onClick={handleOpen}
        className="w-full flex items-center gap-3 px-4 py-3 bg-slate-900/60 border border-slate-700/60 hover:border-slate-500 rounded-xl text-slate-500 hover:text-slate-300 transition-all text-sm"
      >
        <Plus className="w-4 h-4 flex-shrink-0" />
        <span>New idea… <span className="text-slate-600 text-xs">(or Ctrl+Enter to save)</span></span>
      </button>
    );
  }

  return (
    <div className="bg-slate-900 border border-slate-600 rounded-xl p-4 shadow-xl">
      <textarea
        ref={textareaRef}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        onKeyDown={handleKeyDown}
        rows={4}
        placeholder="What's on your mind? First line becomes the project name if you promote it."
        className="w-full bg-transparent text-white text-sm placeholder-slate-600 resize-none focus:outline-none leading-relaxed"
      />
      <div className="flex items-center justify-between mt-3 pt-3 border-t border-slate-700/60">
        <TagPicker value={tag} onChange={setTag} />
        <div className="flex items-center gap-2 flex-shrink-0 ml-3">
          <button onClick={handleDiscard} className="text-slate-500 hover:text-slate-300 transition-colors">
            <X className="w-4 h-4" />
          </button>
          <button
            onClick={handleSave}
            disabled={saving || !content.trim()}
            className="flex items-center gap-1.5 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-40 text-white text-sm px-3 py-1.5 rounded-lg transition-colors"
          >
            <Check className="w-3.5 h-3.5" />
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── IdeaCard ─────────────────────────────────────────────────────────────────

interface IdeaCardProps {
  idea: Idea;
  onUpdate: (updated: Idea) => void;
  onDelete: (id: string) => void;
  onPromote: (id: string) => void;
  promoting: boolean;
}

function IdeaCard({ idea, onUpdate, onDelete, onPromote, promoting }: IdeaCardProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(idea.content);
  const [draftTag, setDraftTag] = useState<Tag>(idea.tag as Tag);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const tag = idea.tag as Tag;

  function startEdit() {
    setDraft(idea.content);
    setDraftTag(idea.tag as Tag);
    setEditing(true);
    setTimeout(() => textareaRef.current?.focus(), 50);
  }

  async function saveEdit() {
    if (!draft.trim()) { setEditing(false); return; }
    if (draft === idea.content && draftTag === idea.tag) { setEditing(false); return; }
    try {
      const updated = await invoke<Idea>('update_idea', {
        id: idea.id,
        content: draft.trim(),
        tag: draftTag,
        pinned: null,
      });
      onUpdate(updated);
    } catch (err) {
      console.error('update_idea failed:', err);
    }
    setEditing(false);
  }

  async function togglePin() {
    try {
      const updated = await invoke<Idea>('update_idea', {
        id: idea.id,
        content: null,
        tag: null,
        pinned: !idea.pinned,
      });
      onUpdate(updated);
    } catch (err) {
      console.error('toggle pin failed:', err);
    }
  }

  async function handleDelete() {
    if (!confirmDelete) { setConfirmDelete(true); return; }
    try {
      await invoke('delete_idea', { id: idea.id });
      onDelete(idea.id);
    } catch (err) {
      console.error('delete_idea failed:', err);
    }
  }

  const cardBorder = idea.pinned
    ? 'border-amber-600/50 hover:border-amber-500/70'
    : TAG_STYLES[tag]?.card ?? 'border-slate-700 hover:border-slate-500';

  return (
    <div
      className={`relative bg-slate-900/80 border rounded-xl p-4 transition-all break-inside-avoid mb-4 ${cardBorder} ${idea.pinned ? 'ring-1 ring-amber-600/20' : ''}`}
    >
      {/* Pin indicator */}
      {idea.pinned && (
        <div className="absolute top-3 right-3 text-amber-500/70">
          <Pin className="w-3 h-3 fill-current" />
        </div>
      )}

      {/* Content / edit area */}
      {editing ? (
        <div>
          <textarea
            ref={textareaRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') saveEdit();
              if (e.key === 'Escape') setEditing(false);
            }}
            rows={Math.max(3, draft.split('\n').length)}
            className="w-full bg-slate-800 border border-slate-600 rounded-lg px-3 py-2 text-white text-sm resize-none focus:outline-none focus:border-slate-400 leading-relaxed"
          />
          <div className="mt-2">
            <TagPicker value={draftTag} onChange={setDraftTag} />
          </div>
          <div className="flex justify-end gap-2 mt-2">
            <button onClick={() => setEditing(false)} className="text-xs text-slate-500 hover:text-slate-300 px-2 py-1">
              Cancel
            </button>
            <button onClick={saveEdit} className="text-xs bg-slate-700 hover:bg-slate-600 text-white px-3 py-1 rounded-lg transition-colors">
              Save
            </button>
          </div>
        </div>
      ) : (
        <p
          onClick={startEdit}
          className="text-slate-200 text-sm leading-relaxed cursor-text whitespace-pre-wrap pr-4"
        >
          {idea.content}
        </p>
      )}

      {/* Footer */}
      {!editing && (
        <div className="flex items-center justify-between mt-3 pt-3 border-t border-slate-700/40">
          <div className="flex items-center gap-2">
            <span className={`text-xs px-2 py-0.5 rounded-full ${TAG_STYLES[tag]?.pill ?? ''}`}>
              {TAG_LABELS[tag] ?? idea.tag}
            </span>
            <span className="text-slate-600 text-xs">{timeAgo(idea.createdAt)}</span>
          </div>

          <div className="flex items-center gap-1">
            {/* Promote to project */}
            <button
              onClick={() => onPromote(idea.id)}
              disabled={promoting}
              title="Turn into project"
              className="p-1.5 text-slate-500 hover:text-emerald-400 disabled:opacity-40 transition-colors rounded"
            >
              <GitBranch className="w-3.5 h-3.5" />
            </button>

            {/* Pin */}
            <button
              onClick={togglePin}
              title={idea.pinned ? 'Unpin' : 'Pin'}
              className={`p-1.5 rounded transition-colors ${idea.pinned ? 'text-amber-400 hover:text-amber-300' : 'text-slate-500 hover:text-amber-400'}`}
            >
              <Pin className={`w-3.5 h-3.5 ${idea.pinned ? 'fill-current' : ''}`} />
            </button>

            {/* Delete */}
            <button
              onClick={handleDelete}
              onBlur={() => setConfirmDelete(false)}
              title={confirmDelete ? 'Click again to confirm' : 'Delete'}
              className={`p-1.5 rounded transition-colors ${confirmDelete ? 'text-red-400 hover:text-red-300' : 'text-slate-600 hover:text-red-400'}`}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

type FilterTab = 'all' | Tag;

export default function IdeasPage() {
  const navigate = useNavigate();
  const [ideas, setIdeas] = useState<Idea[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<FilterTab>('all');
  const [promotingId, setPromotingId] = useState<string | null>(null);
  const [promoteToast, setPromoteToast] = useState<string | null>(null);

  useEffect(() => {
    invoke<Idea[]>('get_ideas')
      .then(setIdeas)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, []);

  function handleCreated(idea: Idea) {
    // Pinned ideas stay first; new ideas appear after pinned.
    setIdeas((prev) => {
      const pinned = prev.filter((i) => i.pinned);
      const unpinned = prev.filter((i) => !i.pinned);
      return [...pinned, idea, ...unpinned];
    });
  }

  function handleUpdate(updated: Idea) {
    setIdeas((prev) => {
      const list = prev.map((i) => i.id === updated.id ? updated : i);
      // Re-sort: pinned first, then by createdAt desc.
      return [
        ...list.filter((i) => i.pinned).sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
        ...list.filter((i) => !i.pinned).sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
      ];
    });
  }

  function handleDelete(id: string) {
    setIdeas((prev) => prev.filter((i) => i.id !== id));
  }

  async function handlePromote(id: string) {
    setPromotingId(id);
    try {
      const projectId = await invoke<string>('idea_to_project', { id });
      // Remove the promoted idea from local state (Rust already deleted it)
      setIdeas((prev) => prev.filter((i) => i.id !== id));
      // Show a brief toast then navigate
      setPromoteToast('Project created — opening tree…');
      setTimeout(() => {
        setPromoteToast(null);
        navigate(`/project/${projectId}`);
      }, 1200);
    } catch (err) {
      console.error('idea_to_project failed:', err);
      setPromotingId(null);
    }
  }

  const filtered = filter === 'all' ? ideas : ideas.filter((i) => i.tag === filter);

  const tabs: Array<{ id: FilterTab; label: string }> = [
    { id: 'all', label: `All (${ideas.length})` },
    ...TAGS.map((t) => ({
      id: t as FilterTab,
      label: `${TAG_LABELS[t]} (${ideas.filter((i) => i.tag === t).length})`,
    })),
  ];

  return (
    <div className="flex flex-col h-full bg-slate-950 text-white">
      {/* Top bar */}
      <div className="flex items-center gap-4 px-6 py-4 border-b border-slate-800 flex-shrink-0">
        <h1 className="text-lg font-semibold">Ideas</h1>
        <div className="flex bg-slate-800/50 rounded-lg p-0.5 overflow-x-auto">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setFilter(tab.id)}
              className="px-3 py-1.5 rounded-md text-xs whitespace-nowrap transition-colors flex-shrink-0"
              style={{
                backgroundColor: filter === tab.id ? 'rgb(51,65,85)' : 'transparent',
                color: filter === tab.id ? 'white' : '#94a3b8',
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-5">
        <div className="max-w-5xl mx-auto space-y-4">
          {/* Compose area */}
          <ComposeArea onCreated={handleCreated} />

          {/* Cards */}
          {loading ? (
            <div className="text-slate-500 text-sm animate-pulse py-8 text-center">Loading…</div>
          ) : filtered.length === 0 ? (
            <div className="text-slate-600 text-sm py-12 text-center">
              {filter === 'all' ? 'No ideas yet — add one above.' : `No ${TAG_LABELS[filter as Tag]} ideas yet.`}
            </div>
          ) : (
            // CSS columns for masonry layout — no extra library needed.
            <div className="[columns:1] sm:[columns:2] lg:[columns:3] gap-4">
              {filtered.map((idea) => (
                <IdeaCard
                  key={idea.id}
                  idea={idea}
                  onUpdate={handleUpdate}
                  onDelete={handleDelete}
                  onPromote={handlePromote}
                  promoting={promotingId === idea.id}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Promote success toast */}
      {promoteToast && (
        <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 flex items-center gap-2 bg-emerald-950/90 border border-emerald-700/60 rounded-lg px-4 py-2.5 shadow-xl">
          <GitBranch className="w-4 h-4 text-emerald-400" />
          <span className="text-sm font-medium text-emerald-300">{promoteToast}</span>
        </div>
      )}
    </div>
  );
}
