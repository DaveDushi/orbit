/**
 * Webhook bots (`/api/v1/workspaces/{id}/bots`): bots that answer in chat from an https URL. Owners and admins manage
 * them. Not in the OpenAPI contract, like the agent routes; bot authors read `docs/webhook-bots.md`.
 */
import type { Client } from './generated/client/index'
import { formDataBodySerializer } from './generated/client/index'

export interface WebhookBot {
  id: string
  /** Its bot user: people mention and DM it like a person. */
  user_id: string
  name: string
  avatar_url: string | null
  url: string
  /** Where people may mention it. Direct messages always work. */
  channels: { id: string; name: string; kind: 'public' | 'private' }[]
  created_at: number
  updated_at: number
  secret_rotated_at: number | null
}

export interface WebhookBots {
  bots: WebhookBot[]
  max_bots: number
  reply_window_seconds: number
}

export interface WebhookBotInput {
  name?: string
  url?: string
  channel_ids?: string[]
}

/** One message sent to a bot. Never its text. */
export interface WebhookBotDelivery {
  id: string
  status: 'pending' | 'sending' | 'answered' | 'late' | 'failed' | 'refused' | 'limited'
  http_status: number | null
  /** A short reason such as `timeout`, `http_status` or `blocked_address`. */
  error: string | null
  created_at: number
  finished_at: number | null
}

const botsUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/bots`

export async function readWebhookBots(client: Client, workspaceId: string): Promise<WebhookBots> {
  const { data } = await client.get({ url: botsUrl(workspaceId), throwOnError: true })
  return data as WebhookBots
}

/** The answer has the signing secret: the only time Orbit shows it. 409 `bot_name_taken` when an account has the name. */
export async function createWebhookBot(client: Client, workspaceId: string, input: WebhookBotInput): Promise<{ bot: WebhookBot; signing_secret: string }> {
  const { data } = await client.post({ url: botsUrl(workspaceId), body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as { bot: WebhookBot; signing_secret: string }
}

export async function updateWebhookBot(client: Client, workspaceId: string, id: string, input: WebhookBotInput): Promise<WebhookBot> {
  const { data } = await client.patch({ url: `${botsUrl(workspaceId)}/${id}`, body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as WebhookBot
}

/** Its messages and DMs stay readable. */
export async function deleteWebhookBot(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${botsUrl(workspaceId)}/${id}`, throwOnError: true })
}

/** A new signing secret, shown once. The old one stops working at once. */
export async function rotateWebhookBotSecret(client: Client, workspaceId: string, id: string): Promise<string> {
  const { data } = await client.post({ url: `${botsUrl(workspaceId)}/${id}/secret`, throwOnError: true })
  return (data as { signing_secret: string }).signing_secret
}

/** A PNG, JPEG or WebP of at most 512 KiB (`avatarImage` makes a 256 px square); `null` removes it. */
export async function setWebhookBotAvatar(client: Client, workspaceId: string, id: string, image: Blob | null): Promise<void> {
  const url = `${botsUrl(workspaceId)}/${id}/avatar`
  if (image) await client.put({ ...formDataBodySerializer, url, body: { file: image }, headers: { 'Content-Type': null }, throwOnError: true })
  else await client.delete({ url, throwOnError: true })
}

/** The bot's 20 newest deliveries, newest first. */
export async function readWebhookBotDeliveries(client: Client, workspaceId: string, id: string): Promise<WebhookBotDelivery[]> {
  const { data } = await client.get({ url: `${botsUrl(workspaceId)}/${id}/deliveries`, throwOnError: true })
  return (data as { deliveries: WebhookBotDelivery[] }).deliveries
}
