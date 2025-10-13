// src/components/ProjectInput.tsx - Form for creating new projects

import React, { useState } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { Project } from '../types';

interface ProjectInputProps {
  onProjectCreated?: (project: Project) => void;
}

export default function ProjectInput({ onProjectCreated }: ProjectInputProps) {
  const [formData, setFormData] = useState({
    name: '',
    description: '',
    discipline: ''
  });
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error', text: string } | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    
    // Validate form
    if (!formData.name.trim() || !formData.description.trim()) {
      setMessage({ type: 'error', text: 'Please fill in all required fields' });
      return;
    }

    setIsSubmitting(true);
    setMessage(null);

    try {
      // Call Rust backend to create project
      const project = await invoke<Project>('create_project', {
        name: formData.name.trim(),
        description: formData.description.trim(),
        discipline: formData.discipline.trim() || 'General'
      });

      setMessage({ type: 'success', text: `Project "${project.name}" created successfully!` });
      
      // Reset form
      setFormData({ name: '', description: '', discipline: '' });
      
      // Notify parent component
      onProjectCreated?.(project);
      
    } catch (error) {
      console.error('Failed to create project:', error);
      setMessage({ type: 'error', text: 'Failed to create project. Please try again.' });
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleInputChange = (field: keyof typeof formData) => (
    e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>
  ) => {
    setFormData(prev => ({ ...prev, [field]: e.target.value }));
  };

  return (
    <div className="max-w-md mx-auto p-6 bg-white rounded-lg shadow-md">
      <h2 className="text-2xl font-bold mb-4 text-gray-800">Create New Project</h2>
      
      <form onSubmit={handleSubmit} className="space-y-4">
        {/* Project Name */}
        <div>
          <label htmlFor="name" className="block text-sm font-medium text-gray-700 mb-1">
            Project Name *
          </label>
          <input
            type="text"
            id="name"
            value={formData.name}
            onChange={handleInputChange('name')}
            placeholder="e.g., Learn Python Programming"
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            disabled={isSubmitting}
          />
        </div>

        {/* Description */}
        <div>
          <label htmlFor="description" className="block text-sm font-medium text-gray-700 mb-1">
            Description *
          </label>
          <textarea
            id="description"
            value={formData.description}
            onChange={handleInputChange('description')}
            placeholder="Describe what you want to achieve with this project..."
            rows={3}
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            disabled={isSubmitting}
          />
        </div>

        {/* Discipline */}
        <div>
          <label htmlFor="discipline" className="block text-sm font-medium text-gray-700 mb-1">
            Discipline
          </label>
          <input
            type="text"
            id="discipline"
            value={formData.discipline}
            onChange={handleInputChange('discipline')}
            placeholder="e.g., Programming, Design, Business"
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            disabled={isSubmitting}
          />
        </div>

        {/* Submit Button */}
        <button
          type="submit"
          disabled={isSubmitting}
          className="w-full bg-blue-600 text-white py-2 px-4 rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {isSubmitting ? 'Creating Project...' : 'Create Project'}
        </button>
      </form>

      {/* Status Message */}
      {message && (
        <div className={`mt-4 p-3 rounded-md ${
          message.type === 'success' 
            ? 'bg-green-100 text-green-800 border border-green-200'
            : 'bg-red-100 text-red-800 border border-red-200'
        }`}>
          {message.text}
        </div>
      )}
    </div>
  );
}
