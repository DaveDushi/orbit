import type { MessageBlock } from '@orbit/api/generated/types.gen'
import { checkUiSpec, UI_VERSION, type UiSpec } from '@agent/ui-catalog'

export interface AgentCard {
  blockId: string
  title: string
  spec: UiSpec
  /** The agent is still writing it: it grows with each edit and takes no clicks yet. */
  live: boolean
}

/** A click or form send of the reader on a card of the agent (`GET …/messages/{id}/actions`). */
export interface SentAction {
  blockId: string
  elementId: string
  /** A form's values, as sent. */
  values: Record<string, string | boolean> | null
}

/** The buttons and forms of one card that the reader sent, with a form's values (the last send counts). */
export function sentOf(actions: readonly SentAction[] | undefined, blockId: string): ReadonlyMap<string, Record<string, string | boolean> | null> {
  return new Map((actions ?? []).filter((action) => action.blockId === blockId).map((action) => [action.elementId, action.values]))
}

/**
 * The card of a message from the team agent (a `ui` block), checked again against the catalog this app renders:
 * `invalid` when the stored card does not fit it (an older or newer catalog), so the message shows its text alone.
 * `null` when the message has no card.
 */
export function agentCardOf(blocks: readonly MessageBlock[] | null | undefined): AgentCard | 'invalid' | null {
  const block = blocks?.find((candidate) => candidate.type === 'ui')
  if (!block || block.type !== 'ui') return null
  if (block.version !== UI_VERSION || !block.block_id) return 'invalid'
  const checked = checkUiSpec(block.spec)
  return checked.ok ? { blockId: block.block_id, title: block.title, spec: checked.spec, live: block.live === true } : 'invalid'
}
