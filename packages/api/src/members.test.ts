import { describe, expect, test } from 'bun:test'
import { emailLine, memberHandle, nameHandle, visibleEmail } from './members'

describe('member handles and emails', () => {
  test('an agent shows its handle, never its placeholder address', () => {
    expect(memberHandle('research-bot@agents.orbit.invalid', 'Research Bot')).toBe('research-bot')
    expect(visibleEmail('research-bot@agents.orbit.invalid')).toBe('')
    expect(emailLine('research-bot@agents.orbit.invalid', 'Research Bot')).toBe('@research-bot')
  })

  test('an agent from before handles gets the handle of its name', () => {
    const email = 'agent-01a1f3d7-c292-7028-b226-1cc4d2f705d3@orbit.invalid'
    expect(memberHandle(email, 'Donna (v2)!')).toBe('donna-v2')
    expect(emailLine(email, 'Donna')).toBe('@donna')
  })

  test('people keep their email and its handle; a guest view without email uses the name', () => {
    expect(memberHandle('ann.lee@example.com', 'Ann Lee')).toBe('ann.lee')
    expect(emailLine('ann.lee@example.com', 'Ann Lee')).toBe('ann.lee@example.com')
    expect(memberHandle('agent-team@example.com', 'Agent Team')).toBe('agent-team')
    expect(memberHandle('', 'Ann Lee')).toBe('Ann Lee')
  })

  test('the name handle matches the server slug', () => {
    expect(nameHandle("Zoë's helper")).toBe('zo-s-helper')
    expect(nameHandle('日本')).toBe('agent')
    expect(nameHandle('abcdefghijklmnopqrstuvwxyz abcd x')).toBe('abcdefghijklmnopqrstuvwxyz-abcd')
  })
})
