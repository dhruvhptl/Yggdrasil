// src/components/ProjectInput.tsx - Form for creating new projects

import React, { useState } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { Project } from '../types';

interface ProjectInputProps {
  onProjectCreated?: (project: Project) => void;
  prefillName?: string;
}

export default function ProjectInput({ onProjectCreated, prefillName }: ProjectInputProps) {
  const [formData, setFormData] = useState({ name: prefillName ?? '', description: '', discipline: '' });
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (!formData.name.trim() || !formData.description.trim()) {
      setMessage({ type: 'error', text: 'Name and description are required.' });
      return;
    }

    setIsSubmitting(true);
    setMessage(null);

    try {
      const project = await invoke<Project>('create_project', {
        name: formData.name.trim(),
        description: formData.description.trim(),
        discipline: formData.discipline.trim() || 'General',
      });

      setMessage({ type: 'success', text: `"${project.name}" created!` });
      setFormData({ name: '', description: '', discipline: '' });
      onProjectCreated?.(project);
    } catch (error) {
      console.error('Failed to create project:', error);
      setMessage({ type: 'error', text: 'Failed to create project. Please try again.' });
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleChange = (field: keyof typeof formData) =>
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setFormData(prev => ({ ...prev, [field]: e.target.value }));
    };

  return (
    <div className="bg-slate-900 border border-slate-800 rounded-lg p-4">
      <h3 className="text-xs font-medium text-slate-400 uppercase tracking-wider mb-3">New Project</h3>
      <form onSubmit={handleSubmit} className="flex flex-col gap-2.5">
        <input
          type="text"
          value={formData.name}
          onChange={handleChange('name')}
          placeholder="Project name *"
          disabled={isSubmitting}
          className="w-full bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 placeholder-slate-500 focus:outline-none focus:border-emerald-600 focus:ring-1 focus:ring-emerald-600/30 disabled:opacity-50 transition-colors"
        />
        <input
          type="text"
          value={formData.description}
          onChange={handleChange('description')}
          placeholder="What do you want to learn? *"
          disabled={isSubmitting}
          className="w-full bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 placeholder-slate-500 focus:outline-none focus:border-emerald-600 focus:ring-1 focus:ring-emerald-600/30 disabled:opacity-50 transition-colors"
        />
        <div className="flex gap-2">
          <input
            type="text"
            value={formData.discipline}
            onChange={handleChange('discipline')}
            placeholder="Discipline (e.g. Programming)"
            disabled={isSubmitting}
            className="flex-1 bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 placeholder-slate-500 focus:outline-none focus:border-emerald-600 focus:ring-1 focus:ring-emerald-600/30 disabled:opacity-50 transition-colors"
          />
          <button
            type="submit"
            disabled={isSubmitting}
            className="px-4 py-2 bg-emerald-700 hover:bg-emerald-600 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white text-sm font-medium rounded-md transition-colors flex-shrink-0"
          >
            {isSubmitting ? 'Creating…' : 'Create'}
          </button>
        </div>

        {message && (
          <p className={`text-xs ${message.type === 'success' ? 'text-emerald-400' : 'text-red-400'}`}>
            {message.text}
          </p>
        )}
      </form>
    </div>
  );
}
