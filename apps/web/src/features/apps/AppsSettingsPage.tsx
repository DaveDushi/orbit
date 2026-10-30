import { toast } from 'sonner'
import { useState, type FormEvent } from 'react'
import { Link } from 'react-router'
import { Copy } from 'reicon-react'
import type { AppOpenMode, AppRole, ManagedApp } from '@orbit/api/apps'
import { confirmAction } from '@/components/common/confirmAction'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldDescription, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { FieldGrid, RequiredMark, RowIcon, SettingsRow } from '@/features/settings/components/SettingsParts'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useCreateManagedApp, useDeleteManagedApp, useManagedApps, useRotateManagedAppSecret, useUpdateManagedApp } from './api'
import { APP_ICONS, appIcon } from './appIcons'

const ROLES: AppRole[] = ['owner', 'admin', 'member']
const MODES = [
  { value: 'embed', label: 'Inside Orbit' },
  { value: 'new_tab', label: 'In a new tab' },
] as const
const ICON_ITEMS = Object.keys(APP_ICONS).map((value) => ({ value, label: value[0].toUpperCase() + value.slice(1) }))

const message = (error: unknown, fallback: string) => (error instanceof Error && error.message ? error.message : fallback)
const lines = (text: string) => text.split('\n').map((line) => line.trim()).filter(Boolean)

/** Workspace settings → Apps: custom web apps, opened from Orbit with one login. Owners and admins only. */
export function AppsSettingsPage() {
  const { workspace } = useWorkspace()
  const canManage = useCan('apps.manage')
  const apps = useManagedApps(workspace.id, canManage)
  // The secret of the app that was just made or rotated. It lives only here: Orbit cannot show it again.
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null)
  const onSecret = (app: ManagedApp) => { if (app.client_secret) setSecret({ name: app.name, value: app.client_secret }) }

  if (!canManage) return <SettingsCard title="Apps"><p className="m-0 text-muted-foreground">Only workspace owners and administrators can set up apps.</p></SettingsCard>

  return <>
    {secret ? <SecretOnce name={secret.name} value={secret.value} onDone={() => setSecret(null)} /> : null}
    <AddApp canSignIn={apps.data?.can_sign_in !== false} onSecret={onSecret} />
    {apps.isPending ? <p className="m-0 text-muted-foreground">Loading apps…</p> : null}
    {apps.isError ? <p className="m-0 text-destructive" role="alert">Apps could not be loaded.</p> : null}
    {apps.data?.items.map((app) => <AppCard key={app.id} app={app} canSignIn={apps.data.can_sign_in} onSecret={onSecret} />)}
  </>
}

function Copyable({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex w-full min-w-0 items-center gap-2">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-xs text-muted-foreground/70">{label}</span>
        <code className="text-[13px] [overflow-wrap:anywhere] text-foreground">{value}</code>
      </div>
      <Button variant="ghost" size="icon" type="button" aria-label={`Copy ${label}`} onClick={() => {
        void navigator.clipboard.writeText(value).then(() => toast('Copied'))
      }}><Copy /></Button>
    </div>
  )
}

function SecretOnce({ name, value, onDone }: { name: string; value: string; onDone: () => void }) {
  return <SettingsCard title={`Client secret of ${name}`} description="Copy it now and put it in the app's settings. Orbit shows it only this once." actions={<Button variant="outline" type="button" onClick={onDone}>Done</Button>}>
    <Copyable label="Client secret" value={value} />
  </SettingsCard>
}

function RolesField({ value, onChange }: { value: AppRole[]; onChange: (roles: AppRole[]) => void }) {
  return <FieldSet className="flex-row flex-wrap gap-[18px]">
    <FieldLegend variant="label">Who may open it</FieldLegend>
    {ROLES.map((role) => <Label key={role} className="font-normal capitalize"><Checkbox checked={value.includes(role)} onCheckedChange={(on) => onChange(on ? [...value, role] : value.filter((item) => item !== role))} /> {role}s</Label>)}
  </FieldSet>
}

function ModeField({ id, value, onChange }: { id: string; value: AppOpenMode; onChange: (mode: AppOpenMode) => void }) {
  return <Field>
    <FieldLabel htmlFor={id}>Opens</FieldLabel>
    <Select items={MODES} value={value} onValueChange={(next) => onChange(next as AppOpenMode)}>
      <SelectTrigger id={id} aria-label="Opens"><SelectValue /></SelectTrigger>
      <SelectContent>{MODES.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
    </Select>
  </Field>
}

function IconField({ id, value, onChange }: { id: string; value: string; onChange: (icon: string) => void }) {
  return <Field>
    <FieldLabel htmlFor={id}>Icon</FieldLabel>
    <Select items={ICON_ITEMS} value={value} onValueChange={(next) => onChange(next as string)}>
      <SelectTrigger id={id} aria-label="Icon"><SelectValue /></SelectTrigger>
      <SelectContent>{ICON_ITEMS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
    </Select>
  </Field>
}

function RedirectsField({ id, value, onChange, disabled }: { id: string; value: string; onChange: (text: string) => void; disabled: boolean }) {
  return <Field className="min-[900px]:col-span-2">
    <FieldLabel htmlFor={id}>Redirect URIs for “Sign in with Orbit”</FieldLabel>
    <Textarea id={id} value={value} rows={2} disabled={disabled} onChange={(event) => onChange(event.target.value)} placeholder="https://reports.example.com/auth/callback" />
    <FieldDescription>{disabled ? 'This server has no ORBIT_APP_KEY, so it cannot sign people in to apps.' : 'One per line, exactly as the app sends them. Leave empty if the app has its own login.'}</FieldDescription>
  </Field>
}

function AddApp({ canSignIn, onSecret }: { canSignIn: boolean; onSecret: (app: ManagedApp) => void }) {
  const { workspace } = useWorkspace()
  const create = useCreateManagedApp(workspace.id)
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [icon, setIcon] = useState('category')
  const [mode, setMode] = useState<AppOpenMode>('embed')
  const [roles, setRoles] = useState<AppRole[]>(['owner', 'admin', 'member'])
  const [redirects, setRedirects] = useState('')
  const ready = name.trim() && url.trim() && roles.length > 0

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!ready) return
    const created = await create.mutateAsync({ name: name.trim(), url: url.trim(), icon, open_mode: mode, allowed_roles: roles, ...(lines(redirects).length ? { redirect_uris: lines(redirects) } : {}) })
    onSecret(created)
    setName('')
    setUrl('')
    setRedirects('')
  }

  return <SettingsCard title="Add an app" description="A web app you run yourself. It gets an entry in the sidebar for the roles you choose. Orbit does not host it.">
    <form onSubmit={(event) => { void submit(event).catch(() => undefined) }}>
      <FieldGrid>
        <Field><FieldLabel htmlFor="app-name">Name <RequiredMark /></FieldLabel><Input id="app-name" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} placeholder="Reports" /></Field>
        <IconField id="app-icon" value={icon} onChange={setIcon} />
        <Field className="min-[900px]:col-span-2">
          <FieldLabel htmlFor="app-url">Address <RequiredMark /></FieldLabel>
          <Input id="app-url" value={url} maxLength={2048} onChange={(event) => setUrl(event.target.value)} placeholder="https://reports.example.com" />
          <FieldDescription>HTTPS, on its own host name: a subdomain of Orbit's domain works best. To open inside Orbit, the app must allow it with a <code>frame-ancestors</code> header.</FieldDescription>
        </Field>
        <ModeField id="app-mode" value={mode} onChange={setMode} />
        <RolesField value={roles} onChange={setRoles} />
        <RedirectsField id="app-redirects" value={redirects} onChange={setRedirects} disabled={!canSignIn} />
        <Button className="justify-self-start" type="submit" disabled={create.isPending || !ready}>Add</Button>
        {create.isError ? <p className="text-destructive min-[900px]:col-span-2" role="alert">{message(create.error, 'The app could not be added.')}</p> : null}
      </FieldGrid>
    </form>
  </SettingsCard>
}

function AppCard({ app, canSignIn, onSecret }: { app: ManagedApp; canSignIn: boolean; onSecret: (app: ManagedApp) => void }) {
  const { workspace } = useWorkspace()
  const update = useUpdateManagedApp(workspace.id)
  const remove = useDeleteManagedApp(workspace.id)
  const rotate = useRotateManagedAppSecret(workspace.id)
  const [redirects, setRedirects] = useState(app.sign_in?.redirect_uris.join('\n') ?? '')
  const [url, setUrl] = useState(app.url)
  const Icon = appIcon(app.icon)

  const save = (input: Parameters<typeof update.mutate>[0]['input'], done?: string) =>
    update.mutate({ id: app.id, input }, {
      onSuccess: (saved) => { onSecret(saved); if (done) toast(done) },
      onError: (error) => toast.error(message(error, 'The change could not be saved.')),
    })

  const actions = <>
    <Button variant="outline" type="button" nativeButton={false} render={<Link to={`/apps/${app.slug}`} />}>Open</Button>
    <Button variant="ghost" type="button" disabled={remove.isPending} onClick={async () => {
      if (await confirmAction({ title: `Remove ${app.name}?`, description: 'It leaves the sidebar for everyone. Its sign-in client ends, with all its tokens. The app itself keeps running, and people stay signed in to it until its own session ends.', confirmLabel: 'Remove', danger: true })) remove.mutate(app.id)
    }}>Remove</Button>
  </>

  return <SettingsCard title={app.name} description={`/apps/${app.slug}`} actions={actions} flush>
    <div className="flex flex-col divide-y">
      <SettingsRow className="flex-nowrap">
        <RowIcon><Icon /></RowIcon>
        <form className="flex min-w-0 flex-1 items-end gap-2" onSubmit={(event) => { event.preventDefault(); if (url.trim() !== app.url) save({ url: url.trim() }, 'Address saved') }}>
          <Field className="min-w-0 flex-1"><FieldLabel htmlFor={`app-url-${app.id}`}>Address</FieldLabel><Input id={`app-url-${app.id}`} value={url} maxLength={2048} onChange={(event) => setUrl(event.target.value)} /></Field>
          <Button variant="outline" type="submit" disabled={update.isPending || !url.trim() || url.trim() === app.url}>Save</Button>
        </form>
      </SettingsRow>
      <SettingsRow>
        <FieldGrid>
          <ModeField id={`app-mode-${app.id}`} value={app.open_mode} onChange={(open_mode) => save({ open_mode })} />
          <IconField id={`app-icon-${app.id}`} value={app.icon} onChange={(icon) => save({ icon })} />
          <RolesField value={app.allowed_roles} onChange={(roles) => { if (roles.length) save({ allowed_roles: roles }) }} />
        </FieldGrid>
      </SettingsRow>
      <SettingsRow>
        <form className="w-full" onSubmit={(event) => { event.preventDefault(); save({ redirect_uris: lines(redirects) }, 'Redirect URIs saved') }}>
          <FieldGrid>
            <RedirectsField id={`app-redirects-${app.id}`} value={redirects} onChange={setRedirects} disabled={!canSignIn} />
            <Button className="justify-self-start" variant="outline" type="submit" disabled={update.isPending || !canSignIn}>Save redirect URIs</Button>
          </FieldGrid>
        </form>
      </SettingsRow>
      {app.sign_in ? <>
        <SettingsRow><Copyable label="Issuer" value={app.sign_in.issuer} /></SettingsRow>
        <SettingsRow><Copyable label="Discovery URL" value={app.sign_in.discovery_url} /></SettingsRow>
        <SettingsRow><Copyable label="Client ID" value={app.sign_in.client_id} /></SettingsRow>
        <SettingsRow>
          <div className="flex w-full flex-wrap items-center justify-between gap-2">
            <span className="text-muted-foreground">Client secret: hidden. A new one ends the old one and all tokens of this app.</span>
            <Button variant="outline" type="button" disabled={rotate.isPending} onClick={async () => {
              if (!await confirmAction({ title: `Make a new secret for ${app.name}?`, description: 'The old secret stops working at once. Sign-in fails until you put the new one in the app.', confirmLabel: 'Make a new secret', danger: true })) return
              rotate.mutate(app.id, { onSuccess: onSecret, onError: (error) => toast.error(message(error, 'The secret could not be changed.')) })
            }}>Make a new secret</Button>
          </div>
        </SettingsRow>
      </> : null}
    </div>
  </SettingsCard>
}
