import type { ComponentType } from 'react'
import {
  Barcode, Book, Box, Card, Category, Chart, CloudConnection, Code, Cpu, DiscountShape, Driver, Gift, Global, Instagram,
  Link2, Message, People, Receipt, Shop, ShoppingBag, ShoppingCart, Tag, Truck, Wallet,
} from 'reicon-react'

/**
 * The icons an app can have, in picker order. The server accepts these names only: keep them and their order equal to
 * `ICONS` in `apps/server/src/apps/mod.rs` (a server test checks it). Never rename one: saved apps keep their name.
 */
export const APP_ICONS: Record<string, ComponentType<{ className?: string }>> = {
  shop: Shop,
  'shopping-cart': ShoppingCart,
  'shopping-bag': ShoppingBag,
  box: Box,
  truck: Truck,
  tag: Tag,
  discount: DiscountShape,
  receipt: Receipt,
  card: Card,
  wallet: Wallet,
  barcode: Barcode,
  gift: Gift,
  people: People,
  message: Message,
  instagram: Instagram,
  global: Global,
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
