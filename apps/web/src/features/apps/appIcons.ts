import type { ComponentType } from 'react'
import { Book, Category, Chart, CloudConnection, Code, Cpu, Driver, Link2 } from 'reicon-react'

/** The icons an app can have. The server accepts these names only. */
export const APP_ICONS: Record<string, ComponentType<{ className?: string }>> = {
  category: Category,
  code: Code,
  chart: Chart,
  cloud: CloudConnection,
  book: Book,
  driver: Driver,
  cpu: Cpu,
  link: Link2,
}

export const appIcon = (name: string) => APP_ICONS[name] ?? Category
