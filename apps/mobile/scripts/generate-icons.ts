/**
 * Writes src/ui/icons.gen.ts with the Outline SVG markup of the reicon icons the web chrome uses, and the Orbit logo.
 * reicon-react renders through React DOM, so the app renders this markup with react-native-svg instead.
 * Run: bun apps/mobile/scripts/generate-icons.ts
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'

/** App icon name → reicon component name (the same one the web imports). */
const ICONS = {
  home: 'Home2',
  tasks: 'TaskSquare',
  docs: 'DocumentText',
  chat: 'Message',
  mail: 'Sms',
  inbox: 'Notification',
  directInbox: 'DirectInbox',
  settings: 'Setting2',
  user: 'User',
  search: 'Search4',
  plus: 'Add',
  send: 'Send2',
  back: 'ArrowLeft',
  arrowUp: 'ArrowUp',
  chevronLeft: 'ChevronLeft',
  chevronRight: 'ChevronRight',
  chevronDown: 'ChevronDown',
  check: 'Check',
  checkCircle: 'TickCircle',
  circle: 'RecordCircle',
  logout: 'Logout',
  hashtag: 'Hashtag',
  lock: 'Lock',
  calendar: 'Calendar',
  calendarTick: 'CalendarTick',
  danger: 'Danger',
  timer: 'Timer',
  admin: 'SecurityUser',
  layer: 'Layer',
  menu: 'Menu',
  sun: 'Sun',
  moon: 'Moon',
  filter: 'Filter',
  display: 'Setting4',
  more: 'MoreH',
  edit: 'Edit',
  brush: 'Brush',
  people: 'People',
  userAdd: 'UserAdd',
  relation: 'Hierarchy',
  subIssue: 'Hierarchy2',
  tag: 'Tag',
  attach: 'Paperclip2',
  smile: 'SmileCircle',
  at: 'At',
  star: 'Star',
  comment: 'Comment',
  trash: 'Trash',
  close: 'Xmark',
  shield: 'ShieldTick',
  keyboard: 'Keyboard',
  driver: 'Driver',
} as const

const output = resolve(import.meta.dir, '../src/ui/icons.gen.ts')
const reicon = dirname(createRequire(import.meta.url).resolve('reicon-react/package.json'))

function outline(component: string): string {
  const source = readFileSync(resolve(reicon, 'icons', `${component}.js`), 'utf8')
  const markup = /\bO:\s*`([^`]*)`/.exec(source)?.[1]
  if (!markup) throw new Error(`reicon ${component} has no Outline weight`)
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">${markup}</svg>`
}

const entries = Object.entries(ICONS).map(([name, component]) => `  /** reicon ${component} */\n  ${name}: ${JSON.stringify(outline(component))},`)

/** The pixel "O" of the web's sign-in card, as the web serves it. */
const logo = readFileSync(resolve(import.meta.dir, '../../web/public/logo.svg'), 'utf8').trim()

writeFileSync(
  output,
  `// Generated from reicon-react (Outline weight) and apps/web/public/logo.svg. Do not edit.
// Regenerate: bun apps/mobile/scripts/generate-icons.ts

export const icons = {
${entries.join('\n')}
} as const

export type IconName = keyof typeof icons

export const logo = ${JSON.stringify(logo)}
`,
)
console.log(`Wrote ${Object.keys(ICONS).length} icons to ${output}`)
