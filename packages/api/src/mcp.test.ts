import { describe, expect, test } from 'bun:test'
import { CONNECT_AGENT_GUIDE_URL, mcpClientCommands, mcpSetup, mcpUrl } from './mcp'

describe('mcpUrl', () => {
  test('appends /mcp to the origin, without a double slash', () => {
    expect(mcpUrl('https://orbit.example.com')).toBe('https://orbit.example.com/mcp')
    expect(mcpUrl('https://orbit.example.com/')).toBe('https://orbit.example.com/mcp')
  })
})

describe('mcpClientCommands', () => {
  test('puts the endpoint into every client command', () => {
    const commands = mcpClientCommands('http://127.0.0.1:18888/mcp')
    expect(commands.map((entry) => entry.client)).toEqual(['Claude', 'Codex', 'Grok'])
    expect(commands[0].command).toBe('claude mcp add --scope user --transport http orbit http://127.0.0.1:18888/mcp')
    expect(commands[1].command).toBe('codex mcp add orbit --url http://127.0.0.1:18888/mcp\ncodex mcp login orbit')
  })
})

describe('mcpSetup', () => {
  test('without a separate MCP address, shows the origin and every client, as before', () => {
    for (const mcpAddress of [undefined, null, '']) {
      expect(mcpSetup('https://orbit.example.com', mcpAddress)).toEqual({ url: 'https://orbit.example.com/mcp', commands: mcpClientCommands('https://orbit.example.com/mcp'), guide: null })
    }
  })

  test('with a separate MCP address, shows it in the Codex and Claude Code commands and links the guide', () => {
    const setup = mcpSetup('https://orbit.example.com', 'https://mcp.example/mcp')
    expect(setup.url).toBe('https://mcp.example/mcp')
    expect(setup.commands.map((entry) => entry.command)).toEqual([
      'codex mcp add orbit --url https://mcp.example/mcp\ncodex mcp login orbit',
      'claude mcp add --scope user --transport http orbit https://mcp.example/mcp',
    ])
    expect(setup.guide).toBe(CONNECT_AGENT_GUIDE_URL)
  })
})
