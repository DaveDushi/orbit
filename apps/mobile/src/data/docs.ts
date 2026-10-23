/**
 * Docs as the web's sidebar tree (`features/docs/pageTree.ts`) and a page's BlockNote blocks as readable lines. No
 * React; the app shows pages read-only.
 */
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
