import { z } from 'zod';
import type { ZodSchema } from 'zod';
import { warn } from './logger';

// ── Core helper ──────────────────────────────────────────────────────────────

/**
 * Validates data against a Zod schema. On failure, logs a console.warn with
 * flattened errors and returns the raw data cast as T — never throws, never
 * breaks the UI.
 */
export function validateOrLog<T>(schema: ZodSchema<T>, data: unknown, label: string): T {
  const result = schema.safeParse(data);
  if (result.success) return result.data;
  warn(`[validation] ${label}:`, result.error.flatten());
  return data as T;
}

// ── Schemas ──────────────────────────────────────────────────────────────────

export const SourceSchema = z.object({
  title: z.string(),
  url: z.string().nullable(),
  chunk: z.string(),
  score: z.number(),
  sectionTitle: z.string().nullable(),
  pageStart: z.number().nullable(),
  pageEnd: z.number().nullable(),
});

export const SuggestionSchema = z.object({
  action: z.string(),
  label: z.string(),
  payload: z.unknown().optional(),
});

export const MimirChatResponseSchema = z.object({
  answer: z.string(),
  sources: z.array(SourceSchema),
  suggestions: z.array(SuggestionSchema).default([]),
});

export const StoredChatMessageSchema = z.object({
  id: z.string(),
  role: z.string(),
  content: z.string(),
  sources: z.array(SourceSchema).nullable(),
  createdAt: z.string(),
});

export const MimirResourceSchema = z.object({
  id: z.string(),
  title: z.string(),
  url: z.string().nullable(),
  resourceType: z.string(),
  status: z.string(),
  userNotes: z.string().nullable(),
  createdAt: z.string(),
  relevanceScore: z.number().nullable().optional(),
  parentId: z.string().nullable().optional(),
  tags: z.array(z.string()),
  nodeCount: z.number(),
  isCompleted: z.boolean(),
  matchedSectionTitle: z.string().nullable().optional(),
  matchedPageStart: z.number().nullable().optional(),
  matchedPageEnd: z.number().nullable().optional(),
});

export const TreeNodeSchema = z.object({
  id: z.string(),
  tree_id: z.string(),
  parent_id: z.string().nullable(),
  type: z.enum(['trunk', 'branch', 'leaf']),
  title: z.string(),
  description: z.string(),
  progress: z.number(),
  tasks: z.unknown(),
  resources: z.array(z.unknown()).nullable(),
  x: z.number().nullable(),
  y: z.number().nullable(),
  order_index: z.number(),
  is_locked: z.boolean(),
});

const SkillEvidenceSchema = z.object({
  type: z.enum(['resume', 'tree_quest', 'work_resource', 'job_demand', 'mimir_resource', 'manual', 'external']),
  detail: z.string().optional(),
  projectName: z.string().optional(),
  treeName: z.string().optional(),
  nodeTitle: z.string().optional(),
  nodeId: z.string().optional(),
  progress: z.number().optional(),
  company: z.string().optional(),
  resourceTitle: z.string().optional(),
  resourceId: z.string().optional(),
  count: z.number().optional(),
  frequency: z.number().optional(),
  demandScore: z.number().optional(),
});

export const SkillSchema = z.object({
  id: z.string(),
  name: z.string(),
  domain: z.string().nullable(),
  level: z.number(),
  evidence: z.array(SkillEvidenceSchema),
  lastUpdated: z.string(),
  reviewNeeded: z.boolean().optional().default(false),
  status: z.string().optional().default('active'),
  origin: z.string().optional().default('tree_quest'),
  state: z.string().optional().default('adjacent'),
});
