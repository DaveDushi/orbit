import { describe, expect, test } from 'bun:test'
import type { AgentProfile, AgentProfiles } from '@orbit/api/agent'
import type { Routine, Script } from '@orbit/api/scheduled'
import type { ConversationRecord } from '@orbit/api/generated/types.gen'

import {
  agentChannelChoices,
  agentFormOf,
  agenticTabs,
  customAgentInput,
  defaultAgentInput,
  INTEGRATION_TOOL,
  NO_SCRIPT,
  pickedTab,
  routineActions,
  routineChange,
  routineFormOf,
  routineTargets,
  servicesLabel,
  timeZoneChoices,
  webSetupLinks,
} from './agentic'

describe('agenticTabs', () => {
  test('the web tabs with the team agent, and no Admin tab', () => {
    expect(agenticTabs(true, false).map((tab) => tab.value)).toEqual(['you', 'workspace', 'agents', 'routines', 'bots'])
  })

  test('without the team agent only You and Bots', () => {
    expect(agenticTabs(false, false).map((tab) => tab.value)).toEqual(['you', 'bots'])
  })

  test('the account settings show only You', () => {
    expect(agenticTabs(true, true).map((tab) => tab.value)).toEqual(['you'])
  })

  test('a link to a tab this person does not have opens You', () => {
    expect(pickedTab(agenticTabs(false, false), 'routines')).toBe('you')
    expect(pickedTab(agenticTabs(true, false), 'routines')).toBe('routines')
    expect(pickedTab(agenticTabs(true, false), 'admin')).toBe('you')
  })
})

describe('webSetupLinks', () => {
  test('a member sees no admin setup pages', () => {
    expect(webSetupLinks([], true)).toEqual([])
  })

  test('each page needs its own permission, agent tools also the agent', () => {
    expect(webSetupLinks(['integrations.manage', 'api_tokens.manage'], true).map((link) => link.label)).toEqual(['Agent tools', 'API tokens'])
    expect(webSetupLinks(['integrations.manage', 'api_tokens.manage'], false).map((link) => link.label)).toEqual(['API tokens'])
    expect(webSetupLinks(['api_tokens.manage'], true).map((link) => link.label)).toEqual(['API tokens'])
  })
})

const agent = (over: Partial<AgentProfile> = {}): AgentProfile => ({
  id: 'a1',
  user_id: 'u-a1',
  name: 'Writer',
  avatar_url: null,
  instructions: 'Draft posts',
  tools: ['read_doc', INTEGRATION_TOOL],
  model: null,
  thinking: 'high',
  answer_level: 'normal',
  all_channels: false,
  channels: [{ id: 'c1', name: 'blog', kind: 'public' }],
  integration_ids: ['i1'],
  created_at: 0,
  updated_at: 0,
  ...over,
})

describe('agent form → body', () => {
  test('a custom agent keeps the services tool out of the form and adds it back with a service', () => {
    const form = agentFormOf(agent())
    expect(form.tools).toEqual(['read_doc'])
    expect(form.model).toBe('default')
    expect(customAgentInput(form)).toEqual({
      instructions: 'Draft posts',
      model: null,
      thinking: 'high',
      answer_level: 'normal',
      integration_ids: ['i1'],
      name: 'Writer',
      tools: ['read_doc', INTEGRATION_TOOL],
      all_channels: false,
      channel_ids: ['c1'],
    })
  })

  test('no service: no services tool', () => {
    const form = { ...agentFormOf(agent()), services: [] }
    expect(customAgentInput(form).tools).toEqual(['read_doc'])
  })

  test('the default agent sends its name only when it changed', () => {
    const main = { user_id: 'bot', name: 'Donna', avatar_url: null, instructions: '', model: 'gpt-x', thinking: null, answer_level: 'short', integration_ids: [], can_rename: false } as AgentProfiles['default_agent']
    const form = agentFormOf(null, main)
    expect(defaultAgentInput(form, 'Donna')).toEqual({ instructions: '', model: 'gpt-x', thinking: null, answer_level: 'short', integration_ids: [] })
    expect(defaultAgentInput({ ...form, name: ' Dana ' }, 'Donna').name).toBe('Dana')
  })

  test('services label counts only for managers', () => {
    const integrations = [{ id: 'i1', name: 'A', kind: 'mcp' as const }, { id: 'i2', name: 'B', kind: 'http' as const }]
    expect(servicesLabel({ can_manage: false, integrations }, ['i1'])).toBe('No connected services')
    expect(servicesLabel({ can_manage: true, integrations }, ['i1'])).toBe('1 of 2 connected services')
    expect(servicesLabel({ can_manage: true, integrations }, ['i1', 'i2'])).toBe('All connected services')
  })
})

const conversation = (over: Partial<ConversationRecord>): ConversationRecord =>
  ({ id: 'c', kind: 'public', name: 'c', archived: false, member_ids: [], ...over }) as ConversationRecord

describe('conversation choices', () => {
  const list = [
    conversation({ id: 'open', kind: 'public' }),
    conversation({ id: 'secret', kind: 'private', member_ids: ['other'] }),
    conversation({ id: 'mine', kind: 'private', member_ids: ['me'] }),
    conversation({ id: 'old', kind: 'public', archived: true, member_ids: ['me'] }),
    conversation({ id: 'dm-agent', kind: 'dm', member_ids: ['me', 'bot'] }),
    conversation({ id: 'dm-other', kind: 'dm', member_ids: ['me', 'other'] }),
  ]

  test('an agent can get open channels and private ones the person is in', () => {
    expect(agentChannelChoices(list, 'me').map((item) => item.id)).toEqual(['open', 'mine'])
  })

  test('a routine posts where the person is, and in a DM only with its agent', () => {
    expect(routineTargets(list, 'me', 'bot').map((item) => item.id)).toEqual(['mine', 'dm-agent'])
  })
})

const routine = (over: Partial<Routine> = {}): Routine =>
  ({ id: 'r1', agent_user_id: 'bot', conversation_id: 'c1', name: 'Daily', prompt: 'Sum up', schedule: '0 9 * * 1-5', time_zone: 'Europe/Berlin', status: 'active', script_id: null, can_edit: true, can_stop: true, ...over }) as Routine

describe('routineChange', () => {
  const scripts = [{ id: 's1', code_hash: 'hash1' }] as Script[]

  test('unchanged target, agent and script are left out', () => {
    expect(routineChange(routine(), routineFormOf(routine()), scripts)).toEqual({ name: 'Daily', prompt: 'Sum up', schedule: '0 9 * * 1-5', time_zone: 'Europe/Berlin' })
  })

  test('a newly picked script is approved at its code hash', () => {
    const form = { ...routineFormOf(routine()), scriptId: 's1', agentId: 'other-bot', target: 'c2' }
    expect(routineChange(routine(), form, scripts)).toMatchObject({ script_id: 's1', approve_script_hash: 'hash1', agent_user_id: 'other-bot', conversation_id: 'c2' })
  })

  test('removing the script sends an empty id and no approval', () => {
    const current = routine({ script_id: 's1' })
    const change = routineChange(current, { ...routineFormOf(current), scriptId: NO_SCRIPT }, scripts)
    expect(change.script_id).toBe('')
    expect(change.approve_script_hash).toBeUndefined()
  })
})

describe('routineActions', () => {
  test('a manager who is not the creator may pause and delete, not edit or resume', () => {
    expect(routineActions(routine({ can_edit: false, can_stop: true }))).toEqual({ pause: true, resume: false, edit: false, test: false, remove: true })
    expect(routineActions(routine({ status: 'paused', can_edit: false, can_stop: true })).resume).toBe(false)
  })

  test('the creator resumes a paused routine', () => {
    expect(routineActions(routine({ status: 'paused' }))).toMatchObject({ pause: false, resume: true })
  })
})

test('timeZoneChoices keeps a saved zone the engine does not list', () => {
  expect(timeZoneChoices('Mars/Base')[0]).toBe('Mars/Base')
})
