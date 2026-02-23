// src/pages/ResourcesPage.tsx
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Trash2, Link, FileText, Globe, BookOpen } from 'lucide-react';
import { MimirResource } from '../types';

type AddMode = 'url' | 'text' | 'pdf';

const TYPE_ICON: Record<string, React.ReactElement> = {
  webpage: <Globe className="w-3.5 h-3.5" />,
  text: <FileText className="w-3.5 h-3.5" />,
  pdf: <BookOpen className="w-3.5 h-3.5" />,
};

const MODE_LABEL: Record<AddMode, string> = {
  url: 'URL',
  text: 'Paste Text',
  pdf: 'Upload PDF',
};

export default function ResourcesPage() {
  const [resources, setResources] = useState<MimirResource[]>([]);
  const [loading, setLoading] = useState(true);
  const [mimiDown, setMimiDown] = useState(false);

  // Add form state
  const [addMode, setAddMode] = useState<AddMode>('url');
  const [urlInput, setUrlInput] = useState('');
  const [titleInput, setTitleInput] = useState('');
  const [textInput, setTextInput] = useState('');
  const [pdfFile, setPdfFile] = useState<File | null>(null);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);

  async function loadResources() {
    try {
      const list = await invoke<MimirResource[]>('get_mimir_resources');
      setResources(list);
      setMimiDown(false);
    } catch (err) {
      const msg = String(err);
      if (msg.includes('not running') || msg.includes('unavailable') || msg.includes('Connection refused')) {
        setMimiDown(true);
      } else {
        console.error('Failed to load resources:', err);
      }
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { loadResources(); }, []);

  async function handleAdd() {
    if (adding) return;
    if (addMode === 'url' && !urlInput.trim()) return;
    if (addMode === 'text' && !textInput.trim()) return;
    if (addMode === 'pdf' && !pdfFile) return;

    setAdding(true);
    setAddError('');
    try {
      if (addMode === 'url') {
        await invoke('ingest_mimir_url', {
          url: urlInput.trim(),
          title: titleInput.trim() || undefined,
        });
        setUrlInput('');
        setTitleInput('');
      } else if (addMode === 'text') {
        if (!titleInput.trim()) {
          setAddError('Title is required for text ingestion');
          setAdding(false);
          return;
        }
        await invoke('ingest_mimir_text', {
          text: textInput.trim(),
          title: titleInput.trim(),
        });
        setTextInput('');
        setTitleInput('');
      } else {
        // PDF mode — read file as base64 and send to sidecar via Rust
        const file = pdfFile!;
        const arrayBuffer = await file.arrayBuffer();
        const bytes = new Uint8Array(arrayBuffer);
        let binary = '';
        bytes.forEach(b => { binary += String.fromCharCode(b); });
        const pdf_base64 = btoa(binary);
        await invoke('ingest_mimir_pdf', {
          filename: file.name,
          pdfBase64: pdf_base64,
        });
        setPdfFile(null);
        if (fileInputRef.current) fileInputRef.current.value = '';
      }
      await loadResources();
    } catch (err) {
      setAddError(String(err));
    } finally {
      setAdding(false);
    }
  }

  async function handleDelete(id: string) {
    try {
      await invoke('delete_mimir_resource', { resourceId: id });
      setResources(prev => prev.filter(r => r.id !== id));
    } catch (err) {
      alert(`Delete failed: ${err}`);
    }
  }

  const canAdd = addMode === 'url' ? !!urlInput.trim()
    : addMode === 'text' ? !!textInput.trim()
    : !!pdfFile;

  return (
    <div className="p-6 max-w-3xl mx-auto flex flex-col gap-6">
      {/* Header */}
      <div>
        <h1 className="text-xl font-semibold text-slate-100">Mimir Library</h1>
        <p className="text-sm text-slate-400 mt-1">
          Ingest URLs, text, or PDFs — Mimir embeds them and auto-links them to matching quest nodes.
        </p>
      </div>

      {/* Sidecar down warning */}
      {mimiDown && (
        <div className="bg-amber-950/40 border border-amber-800/60 rounded-lg p-4 text-sm text-amber-300">
          <strong>Mimir sidecar is not running.</strong> Start it with:
          <code className="ml-2 px-2 py-0.5 bg-amber-900/40 rounded text-amber-200 font-mono text-xs">
            npm --prefix sidecar start
          </code>
          <span className="block mt-1 text-amber-400/70 text-xs">
            Or run <code className="font-mono">npm run tauri dev</code> which starts it automatically via concurrently.
          </span>
        </div>
      )}

      {/* Add resource panel */}
      {!mimiDown && (
        <div className="bg-slate-900 border border-slate-800 rounded-lg p-4 flex flex-col gap-3">
          <p className="text-xs font-medium text-slate-500 uppercase tracking-wider">Add Resource</p>

          {/* Mode toggle */}
          <div className="flex gap-1 bg-slate-950 rounded-md p-1 w-fit">
            {(['url', 'text', 'pdf'] as AddMode[]).map(m => (
              <button
                key={m}
                onClick={() => { setAddMode(m); setAddError(''); }}
                className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
                  addMode === m
                    ? 'bg-emerald-900/60 text-emerald-300 border border-emerald-800/60'
                    : 'text-slate-500 hover:text-slate-300'
                }`}
              >
                {MODE_LABEL[m]}
              </button>
            ))}
          </div>

          {/* Title input (URL + text modes only) */}
          {addMode !== 'pdf' && (
            <input
              type="text"
              value={titleInput}
              onChange={e => setTitleInput(e.target.value)}
              placeholder={addMode === 'url' ? 'Title (optional — auto-detected from page)' : 'Title *'}
              className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-emerald-600"
            />
          )}

          {addMode === 'url' ? (
            <input
              type="url"
              value={urlInput}
              onChange={e => setUrlInput(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleAdd()}
              placeholder="https://example.com/article"
              className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-emerald-600"
            />
          ) : addMode === 'text' ? (
            <textarea
              value={textInput}
              onChange={e => setTextInput(e.target.value)}
              placeholder="Paste notes, documentation, or any text content…"
              rows={6}
              className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 font-mono resize-none focus:outline-none focus:border-emerald-600"
            />
          ) : (
            <div className="flex flex-col gap-2">
              <label
                className="flex items-center gap-3 bg-slate-950 border border-slate-700 border-dashed rounded-md px-3 py-4 cursor-pointer hover:border-emerald-700 transition-colors"
                onClick={() => fileInputRef.current?.click()}
              >
                <BookOpen className="w-5 h-5 text-slate-500 flex-shrink-0" />
                <span className="text-sm text-slate-400">
                  {pdfFile ? pdfFile.name : 'Click to select a PDF file…'}
                </span>
              </label>
              <input
                ref={fileInputRef}
                type="file"
                accept=".pdf,application/pdf"
                className="hidden"
                onChange={e => setPdfFile(e.target.files?.[0] ?? null)}
              />
            </div>
          )}

          {addError && (
            <p className="text-xs text-red-400">{addError}</p>
          )}

          <button
            onClick={handleAdd}
            disabled={adding || !canAdd}
            className="self-start px-4 py-2 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md text-sm font-medium transition-colors flex items-center gap-2"
          >
            {adding ? (
              <>
                <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none"/>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
                </svg>
                {addMode === 'url' ? 'Fetching & embedding…' : addMode === 'pdf' ? 'Parsing & embedding…' : 'Embedding…'}
              </>
            ) : (
              addMode === 'url' ? 'Ingest URL' : addMode === 'pdf' ? 'Ingest PDF' : 'Ingest Text'
            )}
          </button>
          {adding && (
            <p className="text-xs text-slate-500">
              First run downloads the ~90 MB embedding model — this can take 1–2 minutes.
            </p>
          )}
        </div>
      )}

      {/* Resource list */}
      {loading ? (
        <p className="text-sm text-slate-500">Loading…</p>
      ) : resources.length === 0 && !mimiDown ? (
        <p className="text-sm text-slate-500">
          No resources yet. Ingest a URL, paste some text, or upload a PDF to get started.
        </p>
      ) : (
        <>
          {resources.length > 0 && (
            <p className="text-xs font-medium text-slate-500 uppercase tracking-wider -mb-4">
              {resources.length} {resources.length === 1 ? 'resource' : 'resources'}
            </p>
          )}
          <div className="flex flex-col gap-2">
            {resources.map(resource => (
              <div
                key={resource.id}
                className="group bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-lg p-4 flex items-start gap-3 transition-colors"
              >
                {/* Type icon */}
                <div className="w-8 h-8 rounded-lg bg-slate-800 border border-slate-700/60 flex items-center justify-center flex-shrink-0 text-slate-400 mt-0.5">
                  {TYPE_ICON[resource.resourceType] ?? <Link className="w-3.5 h-3.5" />}
                </div>

                {/* Content */}
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-slate-100 truncate">{resource.title}</p>
                  {resource.url && (
                    <a
                      href={resource.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-blue-400 hover:text-blue-300 truncate block mt-0.5"
                    >
                      {resource.url}
                    </a>
                  )}
                  <div className="flex items-center gap-2 mt-1.5">
                    <span className="text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-500 border border-slate-700/50">
                      {resource.resourceType}
                    </span>
                    <span className="text-xs text-slate-600">
                      {resource.createdAt && !isNaN(Date.parse(resource.createdAt))
                        ? new Date(resource.createdAt).toLocaleDateString()
                        : ''}
                    </span>
                  </div>
                </div>

                {/* Delete */}
                <button
                  onClick={() => handleDelete(resource.id)}
                  title="Delete resource"
                  className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-red-400 hover:bg-red-950/40 transition-all flex-shrink-0"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
