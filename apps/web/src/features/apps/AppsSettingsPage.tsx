import { toast } from 'sonner'
import { createElement, useState, type FormEvent } from 'react'
import { Link, useMatch, useNavigate } from 'react-router'
import { ArrowLeft, Copy } from 'reicon-react'
import type { AppOpenMode, AppRole, ManagedApp } from '@orbit/api/apps'
import { confirmAction } from '@/components/common/confirmAction'
import { Modal } from '@/components/common/Modal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldDescription, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
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
const sameRoles = (a: AppRole[], b: AppRole[]) => a.length === b.length && a.every((role) => b.includes(role))

/** Workspace settings → Apps: the list, and the settings of one app at `/settings/apps/:id`. Owners and admins only. */
export function AppsSettingsPage() {
  const { workspace } = useWorkspace()
  const canManage = useCan('apps.manage')
  const apps = useManagedApps(workspace.id, canManage)
  const appId = useMatch('/settings/apps/:appId')?.params.appId
  // The secret of the app that was just made or changed. It lives only here: Orbit cannot show it again.
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null)
  const onSecret = (app: ManagedApp) => { if (app.client_secret) setSecret({ name: app.name, value: app.client_secret }) }

  if (!canManage) return <SettingsCard title="Apps"><p className="m-0 text-muted-foreground">Only workspace owners and administrators can set up apps.</p></SettingsCard>
  if (apps.isPending) return <p className="m-0 text-muted-foreground">Loading apps…</p>
  if (apps.isError) return <p className="m-0 text-destructive" role="alert">Apps could not be loaded.</p>

  const secretCard = secret ? <SecretOnce name={secret.name} value={secret.value} onDone={() => setSecret(null)} /> : null
  if (!appId) return <>{secretCard}<AppList items={apps.data.items} canSignIn={apps.data.can_sign_in} onSecret={onSecret} /></>
  const app = apps.data.items.find((item) => item.id === appId)
  if (!app) return <p className="m-0 text-muted-foreground">This app does not exist. <Link className="underline" to="/settings/apps">See all apps</Link></p>
  return <>{secretCard}<AppSettings key={app.id} app={app} canSignIn={apps.data.can_sign_in} onSecret={onSecret} /></>
}

/** The address that opens an app for the person, as the sidebar does. */
function OpenApp({ app }: { app: ManagedApp }) {
  const link = app.open_mode === 'new_tab' ? <a href={app.url} target="_blank" rel="noopener noreferrer" /> : <Link to={`/apps/${app.slug}`} />
  return <Button variant="outline" size="sm" nativeButton={false} render={link}>Open</Button>
}

function AppList({ items, canSignIn, onSecret }: { items: ManagedApp[]; canSignIn: boolean; onSecret: (app: ManagedApp) => void }) {
  const [adding, setAdding] = useState(false)
  return <>
    <SettingsCard title="Apps" description="Web apps your team opens from the Orbit sidebar." actions={<Button type="button" onClick={() => setAdding(true)}>Add app</Button>} flush>
      {items.length === 0
        ? <p className="m-0 px-4 py-6 text-center text-muted-foreground">No apps yet.</p>
        : <div className="flex flex-col divide-y">
          {items.map((app) => (
            <SettingsRow key={app.id} className="flex-nowrap">
              <Link to={`/settings/apps/${app.id}`} aria-label={`Settings of ${app.name}`} className="flex min-w-0 flex-1 items-center gap-3 outline-none hover:opacity-80 focus-visible:underline">
                <RowIcon>{createElement(appIcon(app.icon))}</RowIcon>
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="truncate text-[13px] font-medium text-foreground">{app.name}</span>
                  <span className="truncate text-xs text-muted-foreground/70">{app.url}</span>
                </span>
              </Link>
              <OpenApp app={app} />
            </SettingsRow>
          ))}
        </div>}
    </SettingsCard>
    {adding ? <AddApp canSignIn={canSignIn} onClose={() => setAdding(false)} onSecret={onSecret} /> : null}
  </>
}

function RolesField({ value, onChange }: { value: AppRole[]; onChange: (roles: AppRole[]) => void }) {
  return <FieldSet className="flex-row flex-wrap gap-[18px]">
    <FieldLegend variant="label">Who can see it</FieldLegend>
    {ROLES.map((role) => <Label key={role} className="font-normal capitalize"><Checkbox checked={value.includes(role)} onCheckedChange={(on) => onChange(on ? [...value, role] : value.filter((item) => item !== role))} /> {role}s</Label>)}
  </FieldSet>
}

function RedirectsField({ id, value, onChange }: { id: string; value: string; onChange: (text: string) => void }) {
  return <Field>
    <FieldLabel htmlFor={id}>Redirect URIs</FieldLabel>
    <Textarea className="field-sizing-fixed" id={id} value={value} rows={2} onChange={(event) => onChange(event.target.value)} placeholder="https://reports.example.com/auth/callback" />
    <FieldDescription>Where Orbit sends people back after they sign in. One per line, exactly as the app sends them.</FieldDescription>
  </Field>
}

/** On: the app signs people in with their Orbit account. */
function SignInSwitch({ id, checked, disabled, onChange }: { id: string; checked: boolean; disabled: boolean; onChange: (on: boolean) => void }) {
  return <div className="flex items-start justify-between gap-3">
    <div className="flex min-w-0 flex-col gap-0.5">
      <Label htmlFor={id}>Sign in with Orbit</Label>
      <span className="text-xs text-muted-foreground">{disabled ? 'This server has no ORBIT_APP_KEY, so it cannot sign people in to apps.' : 'People use their Orbit account in this app. Leave off if the app has its own login.'}</span>
    </div>
    <Switch id={id} checked={checked} disabled={disabled} onCheckedChange={onChange} />
  </div>
}

function AddApp({ canSignIn, onClose, onSecret }: { canSignIn: boolean; onClose: () => void; onSecret: (app: ManagedApp) => void }) {
  const { workspace } = useWorkspace()
  const navigate = useNavigate()
  const create = useCreateManagedApp(workspace.id)
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [roles, setRoles] = useState<AppRole[]>(ROLES)
  const [signIn, setSignIn] = useState(false)
  const [redirects, setRedirects] = useState('')
  const ready = name.trim() && url.trim() && roles.length > 0 && (!signIn || lines(redirects).length > 0)

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!ready) return
    const created = await create.mutateAsync({ name: name.trim(), url: url.trim(), allowed_roles: roles, ...(signIn ? { redirect_uris: lines(redirects) } : {}) })
    onSecret(created)
    onClose()
    // The secret and the values the app needs are on its settings page.
    if (signIn) void navigate(`/settings/apps/${created.id}`)
  }

  return <Modal title="Add app" onClose={onClose}>
    <form className="flex min-w-0 flex-col gap-4" onSubmit={(event) => { void submit(event).catch(() => undefined) }}>
      <Field><FieldLabel htmlFor="app-name">Name <RequiredMark /></FieldLabel><Input id="app-name" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} placeholder="Reports" /></Field>
      <Field>
        <FieldLabel htmlFor="app-url">Address <RequiredMark /></FieldLabel>
        <Input id="app-url" value={url} maxLength={2048} onChange={(event) => setUrl(event.target.value)} placeholder="https://reports.example.com" />
      </Field>
      <RolesField value={roles} onChange={setRoles} />
      <SignInSwitch id="app-sign-in" checked={signIn} disabled={!canSignIn} onChange={setSignIn} />
      {signIn ? <RedirectsField id="app-redirects" value={redirects} onChange={setRedirects} /> : null}
      {create.isError ? <p className="m-0 text-destructive" role="alert">{message(create.error, 'The app could not be added.')}</p> : null}
      <Button className="self-end" type="submit" disabled={create.isPending || !ready}>Add app</Button>
    </form>
  </Modal>
}

function Copyable({ label, value }: { label: string; value: string }) {
  return (
    <SettingsRow className="flex-nowrap">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-xs text-muted-foreground/70">{label}</span>
        <code className="text-[13px] [overflow-wrap:anywhere] text-foreground">{value}</code>
      </div>
      <Button variant="ghost" size="icon" type="button" aria-label={`Copy ${label}`} onClick={() => {
        void navigator.clipboard.writeText(value).then(() => toast('Copied'))
      }}><Copy /></Button>
    </SettingsRow>
  )
}

function SecretOnce({ name, value, onDone }: { name: string; value: string; onDone: () => void }) {
  return <SettingsCard title={`Client secret of ${name}`} description="Copy it now and put it in the app's settings. Orbit shows it only this once." actions={<Button variant="outline" type="button" onClick={onDone}>Done</Button>} flush>
    <Copyable label="Client secret" value={value} />
  </SettingsCard>
}

function AppSettings({ app, canSignIn, onSecret }: { app: ManagedApp; canSignIn: boolean; onSecret: (app: ManagedApp) => void }) {
  const { workspace } = useWorkspace()
  const navigate = useNavigate()
  const update = useUpdateManagedApp(workspace.id)
  const remove = useDeleteManagedApp(workspace.id)
  const [name, setName] = useState(app.name)
  const [url, setUrl] = useState(app.url)
  const [roles, setRoles] = useState(app.allowed_roles)
  const save = (input: Parameters<typeof update.mutate>[0]['input'], done: string) =>
    update.mutate({ id: app.id, input }, {
      onSuccess: (saved) => { onSecret(saved); toast(done) },
      onError: (error) => toast.error(message(error, 'The change could not be saved.')),
    })
  const changed = name.trim() !== app.name || url.trim() !== app.url || !sameRoles(roles, app.allowed_roles)
  const valid = name.trim() && url.trim() && roles.length > 0

  return <>
    <div className="flex items-center justify-between gap-3">
      <Button className="-ml-2" variant="ghost" size="sm" nativeButton={false} render={<Link to="/settings/apps" />}><ArrowLeft />All apps</Button>
      <OpenApp app={app} />
    </div>
    <SettingsCard title={app.name}>
      <form className="flex flex-col gap-4" onSubmit={(event) => {
        event.preventDefault()
        if (changed && valid) save({ name: name.trim(), url: url.trim(), allowed_roles: roles }, 'Saved')
      }}>
        <Field><FieldLabel htmlFor="app-name">Name</FieldLabel><Input id="app-name" value={name} maxLength={80} onChange={(event) => setName(event.target.value)} /></Field>
        <Field>
          <FieldLabel htmlFor="app-url">Address</FieldLabel>
          <Input id="app-url" value={url} maxLength={2048} onChange={(event) => setUrl(event.target.value)} />
          <FieldDescription>To open inside Orbit, the app must allow it with a <code>frame-ancestors</code> header. A subdomain of Orbit's domain works best.</FieldDescription>
        </Field>
        <RolesField value={roles} onChange={setRoles} />
        <Button className="self-start" type="submit" variant="outline" disabled={!changed || !valid || update.isPending}>Save</Button>
      </form>
    </SettingsCard>
    <SettingsCard title="Appearance">
      <FieldGrid>
        <Field>
          <FieldLabel htmlFor="app-mode">Opens</FieldLabel>
          <Select items={MODES} value={app.open_mode} onValueChange={(next) => save({ open_mode: next as AppOpenMode }, 'Saved')}>
            <SelectTrigger id="app-mode" aria-label="Opens"><SelectValue /></SelectTrigger>
            <SelectContent>{MODES.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
          </Select>
        </Field>
        <Field>
          <FieldLabel htmlFor="app-icon">Icon</FieldLabel>
          <Select items={ICON_ITEMS} value={app.icon} onValueChange={(next) => save({ icon: next as string }, 'Saved')}>
            <SelectTrigger id="app-icon" aria-label="Icon"><SelectValue /></SelectTrigger>
            <SelectContent>{ICON_ITEMS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent>
          </Select>
        </Field>
      </FieldGrid>
    </SettingsCard>
    <SignInSection app={app} canSignIn={canSignIn} onSecret={onSecret} />
    <SettingsCard title="Remove app">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="min-w-0 text-muted-foreground">It leaves the sidebar for everyone, and its sign-in ends. The app itself keeps running.</span>
        <Button variant="destructive" type="button" disabled={remove.isPending} onClick={async () => {
          if (!await confirmAction({ title: `Remove ${app.name}?`, description: 'It leaves the sidebar for everyone. Its sign-in client ends, with all its tokens. The app itself keeps running, and people stay signed in to it until its own session ends.', confirmLabel: 'Remove', danger: true })) return
          remove.mutate(app.id, {
            onSuccess: () => void navigate('/settings/apps'),
            onError: (error) => toast.error(message(error, 'The app could not be removed.')),
          })
        }}>Remove</Button>
      </div>
    </SettingsCard>
  </>
}

function SignInSection({ app, canSignIn, onSecret }: { app: ManagedApp; canSignIn: boolean; onSecret: (app: ManagedApp) => void }) {
  const { workspace } = useWorkspace()
  const update = useUpdateManagedApp(workspace.id)
  const rotate = useRotateManagedAppSecret(workspace.id)
  const saved = app.sign_in?.redirect_uris.join('\n') ?? ''
  const [redirects, setRedirects] = useState(saved)
  // Turned on here, not saved yet: the client exists only after the first redirect URI is saved.
  const [turningOn, setTurningOn] = useState(false)
  const on = app.sign_in !== null || turningOn

  const saveRedirects = (uris: string[], done: string) => update.mutate({ id: app.id, input: { redirect_uris: uris } }, {
    onSuccess: (next) => { onSecret(next); setTurningOn(false); toast(done) },
    onError: (error) => toast.error(message(error, 'The change could not be saved.')),
  })

  return <SettingsCard title="Sign-in">
    <div className="flex flex-col gap-4">
      <SignInSwitch id="app-sign-in" checked={on} disabled={!canSignIn || update.isPending} onChange={async (next) => {
        if (next) return setTurningOn(true)
        if (!app.sign_in) return setTurningOn(false)
        if (!await confirmAction({ title: `Turn off sign-in for ${app.name}?`, description: 'Its client ID and secret end, with all its tokens. People cannot sign in to it with Orbit until you turn this on again, which makes a new client.', confirmLabel: 'Turn off', danger: true })) return
        setRedirects('')
        saveRedirects([], 'Sign-in turned off')
      }} />
      {on ? (
        <form className="flex flex-col gap-3" onSubmit={(event) => {
          event.preventDefault()
          if (lines(redirects).length > 0) saveRedirects(lines(redirects), 'Saved')
        }}>
          <RedirectsField id="app-redirects" value={redirects} onChange={setRedirects} />
          <Button className="self-start" variant="outline" type="submit" disabled={update.isPending || lines(redirects).length === 0 || redirects.trim() === saved}>Save</Button>
        </form>
      ) : null}
      {app.sign_in ? (
        <div className="-mx-4 -mb-4 flex flex-col divide-y border-t">
          <Copyable label="Client ID" value={app.sign_in.client_id} />
          <Copyable label="Issuer" value={app.sign_in.issuer} />
          <Copyable label="Discovery URL" value={app.sign_in.discovery_url} />
          <SettingsRow className="justify-between">
            <span className="min-w-0 text-muted-foreground">Client secret is hidden. A new one ends the old one and all tokens of this app.</span>
            <Button variant="outline" type="button" disabled={rotate.isPending} onClick={async () => {
              if (!await confirmAction({ title: `Make a new secret for ${app.name}?`, description: 'The old secret stops working at once. Sign-in fails until you put the new one in the app.', confirmLabel: 'Make a new secret', danger: true })) return
              rotate.mutate(app.id, { onSuccess: onSecret, onError: (error) => toast.error(message(error, 'The secret could not be changed.')) })
            }}>Make a new secret</Button>
          </SettingsRow>
        </div>
      ) : null}
    </div>
  </SettingsCard>
}
