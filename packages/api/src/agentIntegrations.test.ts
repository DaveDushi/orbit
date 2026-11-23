import { expect, test } from 'bun:test'

import { type AgentIntegrationTool, toolMatches, withToolsSet } from './agentIntegrations'

const tool = (name: string, enabled: boolean, description = '', title: string | null = null): AgentIntegrationTool => ({ name, title, description, read_only: false, enabled })

test('the filter matches every word in the name, title or description, in any case', () => {
  const zones = tool('list_zones', false, 'Lists the DNS zones of the account', 'Zones')
  expect(toolMatches(zones, '')).toBe(true)
  expect(toolMatches(zones, '  ')).toBe(true)
  expect(toolMatches(zones, 'ZONES')).toBe(true)
  expect(toolMatches(zones, 'dns account')).toBe(true)
  expect(toolMatches(zones, 'dns worker')).toBe(false)
  expect(toolMatches(tool('purge_cache', false), 'zones')).toBe(false)
})

test('all on or off changes only the shown tools and keeps the hidden ones as they were', () => {
  const tools = [tool('a', true), tool('b', false), tool('c', true), tool('d', false)]
  expect(withToolsSet(tools, ['a', 'b', 'c', 'd'], true)).toEqual(['a', 'b', 'c', 'd'])
  expect(withToolsSet(tools, ['a', 'b', 'c', 'd'], false)).toEqual([])
  expect(withToolsSet(tools, ['a', 'b'], false)).toEqual(['c'])
  expect(withToolsSet(tools, ['b'], true)).toEqual(['a', 'b', 'c'])
  // a name the server no longer lists is not sent back
  expect(withToolsSet(tools, ['gone'], true)).toEqual(['a', 'c'])
})
