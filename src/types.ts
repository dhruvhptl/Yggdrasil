export interface Project {
  id: string;
  name: string;
  description: string;
  disciplineIds: string[];
  skillIds: string[];
  status: string;
  createdAt: string;
  progress: number;
}

// Checkpoint tracking types

export interface CheckpointData {
  mastery_criteria: string;
  exercises: string[];
  notes: string;
  completed: boolean;
}

export interface QuestNode {
  id: string;
  treeId: string;
  projectId: string;
  projectName: string;
  treeName: string;
  title: string;
  description: string;
  progress: number;
  tasks: CheckpointData;
  orderIndex: number;
}

// Add to src/types.ts at the bottom

// Brain-generated tree structure (Phases → Skills → Quests)
export interface GeneratedTree {
  project_id: string;
  phases: Phase[];
}

export interface Phase {
  id: string;
  name: string;
  description: string;
  order: number;
  skills: BrainSkill[];
}

export interface BrainSkill {
  id: string;
  name: string;
  description: string;
  order: number;
  quests: BrainQuest[];
}

export interface BrainQuest {
  id: string;
  title: string;
  mastery_criteria: string;
  exercises: string[];
  order: number;
}

// Mimir library resource (from Rust mimir.rs / sidecar)
export interface MimirResource {
  id: string;
  title: string;
  url: string | null;
  resourceType: string;  // Rust renames `type` → `resource_type` → camelCase `resourceType`
  status: string;
  userNotes: string | null;
  createdAt: string;
  relevanceScore?: number | null;
  parentId?: string | null;
  tags: string[];
  nodeCount: number;
  isCompleted: boolean;
  matchedSectionTitle?: string | null;
  matchedPageStart?: number | null;
  matchedPageEnd?: number | null;
}

// Universal Skill Tree types

export interface SkillEvidence {
  type: 'resume' | 'tree_quest' | 'work_resource' | 'job_demand';
  detail?: string;
  projectName?: string;
  treeName?: string;
  nodeTitle?: string;
  nodeId?: string;
  progress?: number;
  company?: string;
  resourceTitle?: string;
  resourceId?: string;
  count?: number;
  frequency?: number;
  demandScore?: number;
}

export interface UniversalSkill {
  id: string;
  name: string;
  domain: string | null;
  level: number;
  evidence: SkillEvidence[];
  lastUpdated: string;
}

export interface SkillGap {
  skillName: string;
  demandCount: number;
  frequency: number;
  demandScore: number;
  currentLevel: number;
}

export interface SkillDependency {
  id: string;
  sourceSkillId: string;
  targetSkillId: string;
  relationship: string;
}

// Daily Eisenhower Matrix types

export interface DailyQuestLink {
  id: string;
  date: string;
  nodeId: string | null;
  freeText: string | null;
  quadrant: 'do' | 'schedule' | 'delegate' | 'eliminate';
  sortOrder: number;
  addedAt: string;
  nodeTitle: string | null;
  nodeProgress: number | null;
  nodeIsLocked: boolean | null;
  completed: boolean;
}

export interface DailyLog {
  date: string;
  notes: string | null;
  links: DailyQuestLink[];
}
