/**
 * Agent Skills (`/api/v1/workspaces/{id}/agent-skills`): SKILL.md packages that owners and admins add and turn on per
 * agent. Members and guests get 403. Not in the OpenAPI contract, like the other agent routes; see
 * `.ai/agent-design.md`, "Skills".
 */
import type { Client } from './generated/client/index'
import { formDataBodySerializer } from './generated/client/index'

export type AgentSkillSource = 'manual' | 'zip' | 'github' | 'skills_sh'

export interface AgentSkill {
  id: string
  name: string
  description: string
  source: AgentSkillSource
  source_url: string | null
  /** `default` (the default agent) and the bot user ids of the custom agents that use it. */
  agents: string[]
  files: { path: string; size: number }[]
  /** Files below scripts/: they run only in the agent's sandbox. */
  has_scripts: boolean
  size_bytes: number
  created_at: number
  updated_at: number
  /** The SKILL.md body: only when one skill is read. */
  body?: string
}

export interface AgentSkills {
  skills: AgentSkill[]
  /** The agents a skill can be turned on for; `default` first. */
  agents: { key: string; name: string }[]
  /** The agent has its sandbox, where scripts run; `null`: the agent service is offline. */
  sandbox: boolean | null
  max_skills: number
}

export interface AgentSkillChange {
  name?: string
  description?: string
  body?: string
  agents?: string[]
}

const skillsUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/agent-skills`
const json = { 'Content-Type': 'application/json' }

export async function readAgentSkills(client: Client, workspaceId: string): Promise<AgentSkills> {
  const { data } = await client.get({ url: skillsUrl(workspaceId), throwOnError: true })
  return data as AgentSkills
}

export async function readAgentSkill(client: Client, workspaceId: string, id: string): Promise<AgentSkill> {
  const { data } = await client.get({ url: `${skillsUrl(workspaceId)}/${id}`, throwOnError: true })
  return data as AgentSkill
}

/** A skill written in Settings. 409 `skill_exists` when the name is taken. */
export async function createAgentSkill(client: Client, workspaceId: string, input: { name: string; description: string; body: string }): Promise<AgentSkill> {
  const { data } = await client.post({ url: skillsUrl(workspaceId), body: input, headers: json, throwOnError: true })
  return data as AgentSkill
}

/** From a GitHub folder, a skills.sh page or `owner/repo@skill`. 502 `skill_fetch_failed` says why it failed. */
export async function installAgentSkill(client: Client, workspaceId: string, source: string): Promise<AgentSkill> {
  const { data } = await client.post({ url: `${skillsUrl(workspaceId)}/install`, body: { source }, headers: json, throwOnError: true })
  return data as AgentSkill
}

/** A .zip with SKILL.md at its top or in one folder, at most 4 MiB. */
export async function uploadAgentSkill(client: Client, workspaceId: string, file: Blob): Promise<AgentSkill> {
  const { data } = await client.post({ ...formDataBodySerializer, url: `${skillsUrl(workspaceId)}/upload`, body: { file }, headers: { 'Content-Type': null }, throwOnError: true })
  return data as AgentSkill
}

export async function updateAgentSkill(client: Client, workspaceId: string, id: string, change: AgentSkillChange): Promise<AgentSkill> {
  const { data } = await client.patch({ url: `${skillsUrl(workspaceId)}/${id}`, body: change, headers: json, throwOnError: true })
  return data as AgentSkill
}

export async function deleteAgentSkill(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${skillsUrl(workspaceId)}/${id}`, throwOnError: true })
}
