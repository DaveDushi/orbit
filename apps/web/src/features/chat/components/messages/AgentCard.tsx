import { catalog, type UiSpec } from '@agent/ui-catalog'
import { defineRegistry, JSONUIProvider, Renderer } from '@json-render/react'
import { cn } from 'cn'
import { createContext, useContext, useState, type FormEvent, type ReactNode } from 'react'
import { Link, useNavigate } from 'react-router'
import { Check } from 'reicon-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { taskPath } from '@/lib/taskLinks'
import { useSendCardAction } from '../../api/mutations'
import { useSentActions } from '../../api/queries'
import { ChatError } from '../../api/types'
import { useChatHost } from '../../chatHost'
import { useElementWidth } from '../../pages/useChatViewport'
import { sentOf, type AgentCard as Card } from '../../lib/agentCard'
import { Markdown } from './MessageBlocks'
import { findTypedTaskIds } from './taskIds'

type Values = Record<string, string | boolean>

/**
 * What the buttons and forms of one card need: the message, whether the agent still writes the card (no clicks yet),
 * which click is on its way, what the reader sent (with a form's values), and how to send one.
 */
interface CardState {
  messageId: string
  live: boolean
  pending: string | null
  sent: ReadonlyMap<string, Values | null>
  send: (elementId: string, values?: Values) => void
}

const CardContext = createContext<CardState>({ messageId: '', live: false, pending: null, sent: new Map(), send: () => {} })

/** "Sent" beside a button or form that the reader sent: the agent answers in the thread. */
function SentMark() {
  return (
    <span data-slot="agent-card-sent" className="inline-flex items-center gap-1 text-xs text-muted-foreground">
      <Check className="size-3.5" aria-hidden="true" />
      Sent
    </span>
  )
}

/** The element's id, which json-render does not pass to a component: `withIds` puts it in the props of the render copy. */
const idOf = (props: object): string => (props as { id?: string }).id ?? ''

/** A text that is a task id of the workspace (`GEN-12`) links to its task; any other text stays as it is. */
function TaskText({ text }: { text: string }) {
  const { taskKeys } = useChatHost()
  const [match] = findTypedTaskIds(text, taskKeys)
  if (!match || match.escaped || match.start !== 0 || match.end !== text.length) return <>{text}</>
  return (
    <Link to={taskPath({ id: match.identifier })} className="font-medium whitespace-nowrap text-primary hover:underline">
      {text}
    </Link>
  )
}

const SERIES_COLORS = ['var(--chart-2)', 'var(--chart-4)', 'var(--chart-1)']

type ChartProps = {
  kind: 'bar' | 'line'
  title?: string
  x_label?: string
  y_label?: string
  series: { name: string; points: { x: string | number; y: number }[] }[]
}

/**
 * A small bar or line chart in SVG: one y axis from zero, at most 3 series, a legend for two or more. It is drawn at
 * its own width (240–480px), so the text stays 11px on a phone; x labels are left out until the rest fit.
 */
function Chart({ kind, title, x_label, y_label, series }: ChartProps) {
  const [ref, measured] = useElementWidth()
  const width = Math.round(Math.min(480, Math.max(240, measured ?? 480)))
  const height = 220
  const right = 8
  const top = 12
  const bottom = 36
  const xs = [...new Set(series.flatMap((line) => line.points.map((point) => String(point.x))))]
  const values = series.flatMap((line) => line.points.map((point) => point.y))
  const low = Math.min(0, ...values)
  const high = Math.max(0, ...values)
  const step = niceStep((high - low) / 4 || 1)
  const max = Math.ceil(high / step) * step || step
  const min = Math.floor(low / step) * step
  const y = (value: number) => top + ((max - value) / (max - min)) * (height - top - bottom)
  const ticks = Array.from({ length: Math.round((max - min) / step) + 1 }, (_, index) => min + index * step)
  // room for the widest tick (about 7px a character at 11px) and the rotated y label
  const left = Math.max(...ticks.map((tick) => formatTick(tick).length)) * 7 + 10 + (y_label ? 16 : 0)
  const band = (width - left - right) / xs.length
  const center = (x: string) => left + band * (xs.indexOf(x) + 0.5)
  const thickness = Math.min(24, (band * 0.7) / series.length)
  const labelChars = Math.min(14, Math.max(...xs.map((x) => x.length)))
  const labelEvery = Math.max(1, Math.ceil(xs.length / 8), Math.ceil((labelChars * 6.5 + 8) / band))
  return (
    <figure ref={ref} data-slot="agent-card-chart" className="grid min-w-0 gap-1.5">
      {title ? <figcaption className="text-sm font-medium text-foreground">{title}</figcaption> : null}
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={title ?? 'Chart'} className="h-auto w-full max-w-[480px] overflow-visible text-muted-foreground">
        {ticks.map((tick) => (
          <g key={tick}>
            <line x1={left} x2={width - right} y1={y(tick)} y2={y(tick)} className="stroke-border" strokeWidth={1} />
            <text x={left - 6} y={y(tick)} dy="0.32em" textAnchor="end" fontSize={11} fill="currentColor">
              {formatTick(tick)}
            </text>
          </g>
        ))}
        {xs.map((x, index) =>
          index % labelEvery === 0 ? (
            <text key={x} x={center(x)} y={height - bottom + 18} textAnchor="middle" fontSize={11} fill="currentColor">
              {x.length > 14 ? `${x.slice(0, 13)}…` : x}
            </text>
          ) : null,
        )}
        {x_label ? (
          <text x={left + (width - left - right) / 2} y={height - 2} textAnchor="middle" fontSize={11} fill="currentColor">
            {x_label}
          </text>
        ) : null}
        {y_label ? (
          <text transform={`translate(8 ${top + (height - top - bottom) / 2}) rotate(-90)`} textAnchor="middle" fontSize={11} fill="currentColor">
            {y_label}
          </text>
        ) : null}
        {series.map((line, index) => {
          const color = SERIES_COLORS[index]
          if (kind === 'line') {
            const points = line.points.map((point) => `${center(String(point.x))},${y(point.y)}`).join(' ')
            return (
              <g key={line.name}>
                <polyline points={points} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
                {line.points.map((point) => (
                  <circle key={String(point.x)} cx={center(String(point.x))} cy={y(point.y)} r={4} fill={color} stroke="var(--card)" strokeWidth={2}>
                    <title>{`${line.name} · ${point.x}: ${formatNumber(point.y)}`}</title>
                  </circle>
                ))}
              </g>
            )
          }
          return line.points.map((point) => {
            // Bars of the series side by side in the band, 2px apart, rounded at the data end.
            const x = center(String(point.x)) - (thickness * series.length) / 2 + index * thickness + 1
            const barWidth = Math.max(thickness - 2, 1)
            const from = y(0)
            const to = y(point.y)
            const radius = Math.min(4, barWidth / 2, Math.abs(from - to))
            const up = to < from
            const path = up
              ? `M${x},${from}V${to + radius}Q${x},${to} ${x + radius},${to}H${x + barWidth - radius}Q${x + barWidth},${to} ${x + barWidth},${to + radius}V${from}Z`
              : `M${x},${from}V${to - radius}Q${x},${to} ${x + radius},${to}H${x + barWidth - radius}Q${x + barWidth},${to} ${x + barWidth},${to - radius}V${from}Z`
            return (
              <path key={`${line.name}-${point.x}`} d={path} fill={color}>
                <title>{`${series.length > 1 ? `${line.name} · ` : ''}${point.x}: ${formatNumber(point.y)}`}</title>
              </path>
            )
          })
        })}
        <line x1={left} x2={width - right} y1={y(0)} y2={y(0)} className="stroke-muted-foreground/50" strokeWidth={1} />
      </svg>
      {series.length > 1 ? (
        <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {series.map((line, index) => (
            <span key={line.name} className="inline-flex items-center gap-1.5">
              <span aria-hidden="true" className="size-2.5 rounded-sm" style={{ backgroundColor: SERIES_COLORS[index] }} />
              {line.name}
            </span>
          ))}
        </div>
      ) : null}
    </figure>
  )
}

function niceStep(raw: number): number {
  const power = 10 ** Math.floor(Math.log10(raw))
  const scaled = raw / power
  return (scaled <= 1 ? 1 : scaled <= 2 ? 2 : scaled <= 5 ? 5 : 10) * power
}

/** An axis number, short: 15,000,000 is "15M" (the phone app writes it the same way). */
function formatTick(value: number): string {
  const [size, unit] = ([[1e9, 'B'], [1e6, 'M'], [1e3, 'K']] as const).find(([size]) => Math.abs(value) >= size) ?? [1, '']
  return `${Number((value / size).toFixed(1)).toLocaleString()}${unit}`
}

function formatNumber(value: number): string {
  return Number.isInteger(value) ? value.toLocaleString() : value.toLocaleString(undefined, { maximumFractionDigits: 2 })
}

function CardButton({ props }: { props: { label: string; style?: 'primary' | 'secondary' | 'danger'; action: 'reply' | 'open_task'; params?: Record<string, unknown> } }) {
  const { live, pending, sent, send } = useContext(CardContext)
  const navigate = useNavigate()
  const id = idOf(props)
  const variant = props.style === 'danger' ? 'destructive' : props.style === 'primary' ? 'default' : 'outline'
  const task = typeof props.params?.task === 'string' ? props.params.task : null
  const done = props.action === 'reply' && sent.has(id)
  return (
    <span className="inline-flex max-w-full items-center gap-2">
      <Button
        data-slot="agent-card-button"
        type="button"
        size="sm"
        variant={variant}
        className="max-w-full pointer-coarse:h-9"
        // One click at a time for each card, none while the agent still writes it, and each button once.
        disabled={props.action === 'open_task' ? !task : live || done || pending !== null}
        aria-busy={pending === id || undefined}
        onClick={() => (props.action === 'open_task' ? task && navigate(taskPath({ id: task })) : send(id))}
      >
        {pending === id ? <Spinner data-icon="inline-start" /> : null}
        <span className="truncate">{props.label}</span>
      </Button>
      {done ? <SentMark /> : null}
    </span>
  )
}

type FormField = { name: string; label: string; kind: 'text' | 'textarea' | 'select' | 'checkbox' | 'date'; options?: string[]; required?: boolean }

/** A short form: its values go to the agent from the person who sends it, without a chat message. Sent once. */
function CardForm({ props }: { props: { fields: FormField[]; submit_label?: string } }) {
  const { messageId, live, pending, sent, send } = useContext(CardContext)
  const id = idOf(props)
  const [typed, setValues] = useState<Values>({})
  const done = sent.has(id)
  // A sent form shows what was sent, also after a reload.
  const values = (done ? sent.get(id) : null) ?? typed
  const set = (name: string, value: string | boolean) => setValues((old) => ({ ...old, [name]: value }))
  const submit = (event: FormEvent) => {
    event.preventDefault()
    send(id, values)
  }
  return (
    <form data-slot="agent-card-form" onSubmit={submit} className="grid gap-3">
      <fieldset disabled={live || done || pending !== null} className="grid gap-3">
        {props.fields.map((field) => {
          const inputId = `${messageId}-${id}-${field.name}`
          const value = values[field.name]
          if (field.kind === 'checkbox') {
            return (
              <label key={field.name} htmlFor={inputId} className="flex items-center gap-2 text-sm">
                <Checkbox id={inputId} checked={value === true} onCheckedChange={(checked) => set(field.name, checked === true)} />
                {field.label}
              </label>
            )
          }
          return (
            <div key={field.name} className="grid gap-1.5">
              <label htmlFor={inputId} className="text-sm font-medium">
                {field.label}
                {field.required ? <span className="text-muted-foreground"> *</span> : null}
              </label>
              {field.kind === 'textarea' ? (
                <Textarea id={inputId} required={field.required} maxLength={2000} value={String(value ?? '')} onChange={(event) => set(field.name, event.target.value)} />
              ) : field.kind === 'select' ? (
                <Select items={(field.options ?? []).map((option) => ({ value: option, label: option }))} value={typeof value === 'string' ? value : null} onValueChange={(next) => set(field.name, next ?? '')}>
                  <SelectTrigger id={inputId} className="w-56 max-w-full" aria-required={field.required}>
                    <SelectValue placeholder="Choose…" />
                  </SelectTrigger>
                  <SelectContent>
                    {(field.options ?? []).map((option) => (
                      <SelectItem key={option} value={option}>
                        {option}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <Input
                  id={inputId}
                  type={field.kind === 'date' ? 'date' : 'text'}
                  required={field.required}
                  maxLength={2000}
                  className={field.kind === 'date' ? 'w-44' : undefined}
                  value={String(value ?? '')}
                  onChange={(event) => set(field.name, event.target.value)}
                />
              )}
            </div>
          )
        })}
      </fieldset>
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" className="pointer-coarse:h-9" disabled={live || done || pending !== null || props.fields.some((field) => field.required && field.kind !== 'checkbox' && !values[field.name])}>
          {pending === id ? <Spinner data-icon="inline-start" /> : null}
          {props.submit_label || 'Send'}
        </Button>
        {done ? <SentMark /> : null}
      </div>
    </form>
  )
}

const TONES = {
  default: undefined,
  muted: 'text-muted-foreground',
  success: 'text-green-600 dark:text-green-400',
  warning: 'text-amber-600 dark:text-amber-400',
  danger: 'text-destructive',
} as const

const { registry } = defineRegistry(catalog, {
  components: {
    Card: ({ props, children }) => (
      // one column no wider than the card: a long word breaks instead of pushing the card's parts out of it
      <section data-slot="agent-card-card" className="grid min-w-0 grid-cols-1 gap-3 rounded-lg border bg-card p-3 wrap-anywhere">
        {props.title || props.description ? (
          <header className="grid gap-0.5">
            {props.title ? <h3 className="font-semibold text-foreground">{props.title}</h3> : null}
            {props.description ? <p className="text-sm text-muted-foreground">{props.description}</p> : null}
          </header>
        ) : null}
        {children}
      </section>
    ),
    Stack: ({ props, children }) => (
      <div data-slot="agent-card-stack" className={cn('flex min-w-0 gap-2', props.direction === 'row' ? 'flex-row flex-wrap items-center' : 'flex-col')}>
        {children}
      </div>
    ),
    Text: ({ props }) => (
      <Markdown text={props.text} id={idOf(props)} className={cn('leading-[1.5] wrap-anywhere text-foreground/85 [&_pre]:overflow-x-auto', TONES[props.tone ?? 'default'])} />
    ),
    Table: ({ props }) => (
      <div data-slot="agent-card-table" className="max-w-full overflow-x-auto rounded-md border">
        {/* as a Markdown table: wraps at words, scrolls sideways; numbers and one-word cells stay on one line */}
        <table className="min-w-full text-sm wrap-break-word">
          <thead className="bg-muted/50 text-left text-xs text-muted-foreground">
            <tr>
              {props.columns.map((column) => (
                <th key={column.key} scope="col" className="px-2.5 py-1.5 font-medium whitespace-nowrap" style={{ textAlign: column.align }}>
                  {column.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {props.rows.map((row, index) => (
              <tr key={index} className="border-t">
                {props.columns.map((column) => {
                  const cell = row[column.key]
                  return (
                    <td
                      key={column.key}
                      data-token={typeof cell === 'number' || (typeof cell === 'string' && cell.length <= 32 && !/\s/.test(cell)) || undefined}
                      className="max-w-64 min-w-28 px-2.5 py-1.5 align-top data-token:whitespace-nowrap"
                      style={{ textAlign: column.align ?? (typeof cell === 'number' ? 'right' : undefined) }}>
                      {typeof cell === 'string' ? <TaskText text={cell} /> : typeof cell === 'number' ? formatNumber(cell) : cell === true ? 'yes' : cell === false ? 'no' : ''}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    ),
    Chart: ({ props }) => <Chart {...props} />,
    TaskList: ({ props }) => (
      <ul data-slot="agent-card-tasks" className="grid divide-y rounded-md border text-sm">
        {props.tasks.map((task) => (
          <li key={task.id} className="flex min-w-0 flex-wrap items-baseline gap-x-2 px-2.5 py-1.5">
            <span className="shrink-0 font-medium">
              <TaskText text={task.id} />
            </span>
            <span className="min-w-0 flex-1 wrap-anywhere">{task.title}</span>
            <span className="text-xs text-muted-foreground">{[task.status, task.assignee, task.due].filter(Boolean).join(' · ')}</span>
          </li>
        ))}
      </ul>
    ),
    Form: ({ props }) => <CardForm props={props} />,
    Button: ({ props }) => <CardButton props={props} />,
  },
})

/** The render copy of a spec: each element gets its own id as a prop, for the components that send clicks. */
function withIds(spec: UiSpec): UiSpec {
  return { root: spec.root, elements: Object.fromEntries(Object.entries(spec.elements).map(([id, element]) => [id, { ...element, props: { ...element.props, id } }])) }
}

/**
 * A card that the team agent answered with, under its text. While the agent writes it (`live`), it grows with each edit
 * and takes no clicks. A click on a "reply" button or a form send goes to the agent without a chat message: the button
 * or form shows "Sent", and the agent answers in the thread. The server takes what the button does from the stored card.
 */
export function AgentCard({ card, messageId }: { card: Card; messageId: string }): ReactNode {
  const [pending, setPending] = useState<string | null>(null)
  const sentActions = useSentActions(messageId, !card.live)
  const sendAction = useSendCardAction()
  const send = (elementId: string, values?: Values) => {
    setPending(elementId)
    sendAction.mutate(
      { messageId, blockId: card.blockId, actionId: elementId, values },
      {
        onError: (error) => toast.error(error instanceof ChatError && error.message ? error.message : 'The agent did not get the click.'),
        onSettled: () => setPending(null),
      },
    )
  }
  return (
    <div data-slot="agent-card" aria-busy={card.live || undefined} className="mt-1.5 grid max-w-[640px] min-w-0 gap-1.5">
      <CardContext value={{ messageId, live: card.live, pending, sent: sentOf(sentActions.data, card.blockId), send }}>
        <JSONUIProvider registry={registry}>
          <Renderer spec={withIds(card.spec)} registry={registry} />
        </JSONUIProvider>
      </CardContext>
      {card.live ? (
        <p data-slot="agent-card-live" className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Spinner className="size-3" />
          Writing the card…
        </p>
      ) : null}
    </div>
  )
}
