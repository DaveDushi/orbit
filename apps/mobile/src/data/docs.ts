/**
 * Docs as the web's sidebar tree (`features/docs/pageTree.ts`), a page's BlockNote blocks as readable lines, and the
 * plain paragraphs the app can add to a page. No React.
 */
import type { CreatePageBody } from '@orbit/api/generated/types.gen'

import type { PageSummary, Teamspace } from './orbit'

export const pageTitle = (page: Pick<PageSummary, 'title'>) => page.title.trim() || 'Untitled'

export interface TreeRow {
  page: PageSummary
  depth: number
  hasChildren: boolean
}

export interface DocSpace {
  /** `teamspace:<id>` or `private`, as the web keys a space. */
  key: string
  name: string
  rows: TreeRow[]
}

const byPosition = (a: PageSummary, b: PageSummary) => a.position - b.position || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
const spaceOf = (page: PageSummary) => (page.private || !page.teamspace_id ? 'private' : `teamspace:${page.teamspace_id}`)

/**
 * Teamspaces by position, then Private: each with its pages depth first in the server's order. The children of a page
 * in `collapsed` are left out.
 */
export function docSpaces(pages: readonly PageSummary[], teamspaces: readonly Teamspace[], collapsed: ReadonlySet<string>): { teamspaces: DocSpace[]; private: DocSpace } {
  const children = (parentId: string) => pages.filter((page) => page.parent_id === parentId).sort(byPosition)
  const rowsOf = (roots: PageSummary[], depth: number): TreeRow[] =>
    roots.flatMap((page) => {
      const below = children(page.id)
      return [{ page, depth, hasChildren: below.length > 0 }, ...(collapsed.has(page.id) ? [] : rowsOf(below, depth + 1))]
    })
  const space = (key: string, name: string): DocSpace => ({ key, name, rows: rowsOf(pages.filter((page) => page.parent_id === null && spaceOf(page) === key).sort(byPosition), 0) })
  return {
    teamspaces: [...teamspaces].sort((a, b) => a.position - b.position).map((teamspace) => space(`teamspace:${teamspace.id}`, teamspace.name)),
    private: space('private', 'Private'),
  }
}

export type DocBlockKind = 'paragraph' | 'heading' | 'bullet' | 'numbered' | 'check' | 'quote' | 'code' | 'divider' | 'other'

export interface DocBlock {
  id: string
  kind: DocBlockKind
  text: string
  /** Nesting under a parent block (list indents). */
  depth: number
  /** A heading's level, 1 to 3. */
  level?: number
  /** A numbered item's number within its run. */
  number?: number
  checked?: boolean
}

interface RawBlock {
  id?: unknown
  type?: unknown
  props?: Record<string, unknown>
  content?: unknown
  children?: unknown
}

const KIND: Record<string, DocBlockKind> = {
  paragraph: 'paragraph',
  heading: 'heading',
  bulletListItem: 'bullet',
  numberedListItem: 'numbered',
  checkListItem: 'check',
  quote: 'quote',
  codeBlock: 'code',
  divider: 'divider',
}

/** The text of inline content: text runs, links and mentions (by their label). Tables join their cells. */
function inlineText(content: unknown): string {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) return content.map(inlineText).join('')
  if (typeof content !== 'object' || content === null) return ''
  const node = content as { type?: unknown; text?: unknown; content?: unknown; props?: Record<string, unknown>; rows?: unknown; cells?: unknown }
  if (typeof node.text === 'string') return node.text
  if (node.type === 'link') return inlineText(node.content)
  if (node.type === 'tableContent' && Array.isArray(node.rows)) {
    return node.rows.map((row: { cells?: unknown[] }) => (row.cells ?? []).map(inlineText).join(' · ')).join('\n')
  }
  const label = node.props?.label ?? node.props?.title ?? node.props?.name
  if (typeof label === 'string') return node.type === 'mention' ? `@${label}` : label
  return inlineText(node.content)
}

/** A page's blocks, depth first, as lines to read. Unknown blocks keep their text or a caption. */
export function docBlocks(content: readonly unknown[]): DocBlock[] {
  const out: DocBlock[] = []
  const walk = (blocks: readonly unknown[], depth: number) => {
    let number = 0
    blocks.forEach((value, index) => {
      const block = (value ?? {}) as RawBlock
      const type = typeof block.type === 'string' ? block.type : ''
      const kind = KIND[type] ?? 'other'
      number = kind === 'numbered' ? number + 1 : 0
      const props = block.props ?? {}
      const caption = typeof props.caption === 'string' && props.caption ? props.caption : typeof props.name === 'string' ? props.name : ''
      const text = inlineText(block.content) || (kind === 'other' && type ? caption || `[${type}]` : '')
      out.push({
        id: typeof block.id === 'string' ? block.id : `${depth}-${index}`,
        kind,
        text,
        depth,
        ...(kind === 'heading' ? { level: Math.min(3, Math.max(1, Number(props.level) || 1)) } : {}),
        ...(kind === 'numbered' ? { number } : {}),
        ...(kind === 'check' ? { checked: props.checked === true } : {}),
      })
      if (Array.isArray(block.children) && block.children.length > 0) walk(block.children, depth + 1)
    })
  }
  walk(content, 0)
  return out
}

/** Where a new page goes: a teamspace, Private, or (no space) the server's default teamspace. */
export function spaceRequest(space: string | null): CreatePageBody {
  if (space === null) return {}
  if (space === 'private') return { private: true }
  return { teamspace_id: space.replace(/^teamspace:/, '') }
}

/** Every page below `pageId`, at any depth. */
export function descendantsOf(pages: readonly PageSummary[], pageId: string): PageSummary[] {
  const out: PageSummary[] = []
  const queue = [pageId]
  while (queue.length > 0) {
    const parent = queue.shift()
    for (const page of pages) {
      if (page.parent_id === parent && !out.includes(page)) {
        out.push(page)
        queue.push(page.id)
      }
    }
  }
  return out
}

/** The web's "Move to trash" confirmation. */
export function trashPrompt(pages: readonly PageSummary[], page: PageSummary): { title: string; description: string } {
  const below = descendantsOf(pages, page.id).length
  return {
    title: `Move “${pageTitle(page)}” to the trash?`,
    description: below > 0 ? `Its ${below} sub-page${below === 1 ? '' : 's'} move with it. You can restore them from Trash.` : 'You can restore it from Trash.',
  }
}

/** The favorite pages that still exist, in the favorites' order. */
export function favoritePages(pages: readonly PageSummary[], ids: readonly string[]): PageSummary[] {
  return ids.flatMap((id) => pages.find((page) => page.id === id) ?? [])
}

/** The web's cover URL rule: an http(s) URL up to 2048 characters. */
export function validCoverUrl(raw: string): string | null {
  const url = raw.trim()
  if (url.length === 0 || url.length > 2048) return null
  return /^https?:\/\/[^\s/]+/i.test(url) ? url : null
}

/** A page's own file URLs are paths on the server; others are absolute. */
export const absoluteUrl = (server: string, url: string) => (url.startsWith('/') ? `${server}${url}` : url)

/** A random v4 UUID: the id BlockNote gives a block. */
export function blockId(random: () => number = Math.random): string {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (char) => {
    const value = Math.floor(random() * 16)
    return (char === 'x' ? value : (value & 0x3) | 0x8).toString(16)
  })
}

/** One BlockNote paragraph per non-empty line, as the web editor stores typed text. */
export function paragraphBlocks(text: string, makeId: () => string = blockId): Record<string, unknown>[] {
  return text
    .split('\n')
    .map((line) => line.trimEnd())
    .filter((line) => line.trim() !== '')
    .map((line) => ({
      id: makeId(),
      type: 'paragraph',
      props: { backgroundColor: 'default', textAlignment: 'left', textColor: 'default' },
      content: [{ type: 'text', text: line, styles: {} }],
      children: [],
    }))
}

/** Emoji offered for a page or teamspace icon; any other one can be typed. */
export const PAGE_ICONS = ['📄', '📝', '📌', '📎', '📚', '📊', '📈', '🗂️', '🗓️', '✅', '💡', '🚀', '🎯', '🧭', '🛠️', '⚙️', '🔒', '🌱', '🔥', '⭐', '❤️', '🎉', '🧪', '🐛'] as const

/** A typed icon: its first emoji (a whole ZWJ sequence), or `null` when it is empty. */
export function iconOf(input: string): string | null {
  const trimmed = input.trim()
  if (!trimmed) return null
  // Hermes may lack `Intl.Segmenter`: then the first 16 code points, enough for any emoji sequence
  if (typeof Intl.Segmenter !== 'function') return Array.from(trimmed).slice(0, 16).join('')
  for (const part of new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(trimmed)) return part.segment
  return null
}

interface ThreadLike {
  resolved: boolean
  updated_at: string
  comments: readonly { deleted_at?: string | null }[]
}

/** Threads with a comment left, open ones first, the latest first. */
export function visibleThreads<T extends ThreadLike>(threads: readonly T[]): T[] {
  return threads
    .filter((thread) => thread.comments.some((comment) => !comment.deleted_at))
    .sort((a, b) => Number(a.resolved) - Number(b.resolved) || b.updated_at.localeCompare(a.updated_at))
}
