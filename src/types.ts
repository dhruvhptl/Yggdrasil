// src/types.ts - Data models for our skill tree app

export interface Discipline {
  id: string;
  name: string;
  description: string;
  color?: string;
}

export interface Skill {
  id: string;
  name: string;
  description: string;
  disciplineId: string;
  proficiencyLevel: 'beginner' | 'intermediate' | 'advanced';
  progress: number; // 0-100
  isUnlocked: boolean;
  prerequisites: string[]; // Array of skill IDs that must be completed first
  projectIds: string[]; // Array of project IDs this skill belongs to
}

export interface Project {
  id: string;
  name: string;
  description: string;
  disciplineIds: string[]; // Can span multiple disciplines
  skillIds: string[]; // Required skills for this project
  status: 'active' | 'completed' | 'paused';
  createdAt: Date;
  progress: number; // 0-100, calculated from skill progress
}

export interface Quest {
  id: string;
  title: string;
  description: string;
  skillId: string;
  projectId: string;
  isCompleted: boolean;
  createdAt: Date;
  dueDate?: Date;
  priority: 'low' | 'medium' | 'high';
}

export interface Resource {
  id: string;
  title: string;
  description: string;
  url: string;
  type: 'article' | 'video' | 'course' | 'book' | 'other';
  skillIds: string[];
  projectIds: string[];
  source?: 'notion' | 'manual';
  notionId?: string;
}

// For Beautiful Skill Tree component
export interface SkillTreeNode {
  id: string;
  title: string;
  tooltip: {
    content: string;
  };
  children: SkillTreeNode[];
}
