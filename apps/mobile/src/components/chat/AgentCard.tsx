import { catalog, type UiSpec } from '@agent/ui-catalog'
import { defineRegistry, JSONUIProvider, Renderer } from '@json-render/react-native'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { createContext, use, useState } from 'react'
import { Pressable, ScrollView, View } from 'react-native'
import Svg, { Circle, G, Line, Path, Polyline, Text as SvgText } from 'react-native-svg'

import { type AgentCard as Card, type CardValues, type SentAction, sentOf } from '../../data/agent-card'
import { sendCardAction, sentCardActions } from '../../data/chat-api'
import { keys } from '../../data/keys'
import { identifier } from '../../data/tasks'
import { markdownToText } from '../../data/markdown'
import { showError } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useProjects, useTasks } from '../../queries'
import { useTheme } from '../../theme'
import { Button, Icon, Input, PickerSheet, Spinner, Text } from '../../ui'
import { DayPicker } from '../tasks/DayPicker'

/**
 * What the buttons and forms of one card need, as on the web: whether the agent still writes the card (no taps yet),
 * which tap is on its way, what the reader sent (with a form's values), and how to send one.
 */
interface CardState {
  live: boolean
  pending: string | null
  sent: ReadonlyMap<string, CardValues | null>
  send: (elementId: string, values?: CardValues) => void
}

const CardContext = createContext<CardState>({ live: false, pending: null, sent: new Map(), send: () => {} })

/** The element's id, which json-render does not pass to a component: `withIds` puts it in the props of the render copy. */
const idOf = (props: object): string => (props as { id?: string }).id ?? ''

/** The task of an id like GEN-12 in the workspace, to open it. */
function useTaskOf() {
  const tasks = useTasks()
  const projects = useProjects()
  return (ref: string) => (tasks.data ?? []).find((task) => identifier(task, projects.data ?? []) === ref)
}

/** A text that is a task id of the workspace links to its task; any other text stays as it is. */
function TaskText({ text, className = '' }: { text: string; className?: string }) {
  const task = useTaskOf()(text)
  if (!task) return <Text className={`text-[14px] leading-[20px] ${className}`}>{text}</Text>
  return (
    <Text accessibilityRole="link" onPress={() => router.push(`/tasks/${task.id}`)} className={`font-noto-medium text-[14px] leading-[20px] text-primary ${className}`}>
      {text}
    </Text>
  )
}

function SentMark() {
  const { colors } = useTheme()
  return (
    <View className="flex-row items-center gap-1">
      <Icon name="check" size={14} color={colors.mutedForeground} />
      <Text className="text-[12px] leading-[16px] text-muted-foreground">Sent</Text>
    </View>
  )
}

type ChartProps = {
  kind: 'bar' | 'line'
  title?: string
  x_label?: string
  y_label?: string
  series: { name: string; points: { x: string | number; y: number }[] }[]
}

function niceStep(raw: number): number {
  const power = 10 ** Math.floor(Math.log10(raw))
  const scaled = raw / power
  return (scaled <= 1 ? 1 : scaled <= 2 ? 2 : scaled <= 5 ? 5 : 10) * power
}

/** An axis number, short: 15,000,000 is "15M", as on the web (Hermes has no compact notation everywhere). */
function formatTick(value: number): string {
  const [size, unit] = ([[1e9, 'B'], [1e6, 'M'], [1e3, 'K']] as const).find(([size]) => Math.abs(value) >= size) ?? [1, '']
  return `${Number((value / size).toFixed(1)).toLocaleString()}${unit}`
}
const formatNumber = (value: number) => (Number.isInteger(value) ? value.toLocaleString() : value.toLocaleString(undefined, { maximumFractionDigits: 2 }))

/**
 * The web's small bar or line chart, in react-native-svg: one y axis from zero, at most 3 series, a legend for two or
 * more. Drawn at its own width (240–480), so the text stays 12px; x labels are left out until the rest fit.
 */
function Chart({ kind, title, x_label, y_label, series }: ChartProps) {
  const { colors } = useTheme()
  const seriesColors = [colors.chart2, colors.chart4, colors.chart1]
  const [measured, setMeasured] = useState(480)
  const width = Math.round(Math.min(480, Math.max(240, measured)))
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
  // room for the widest tick (about 7.5 a character at 12) and the rotated y label
  const left = Math.max(...ticks.map((tick) => formatTick(tick).length)) * 7.5 + 10 + (y_label ? 16 : 0)
  const band = (width - left - right) / xs.length
  const center = (x: string) => left + band * (xs.indexOf(x) + 0.5)
  const thickness = Math.min(24, (band * 0.7) / series.length)
  const labelChars = Math.min(12, Math.max(...xs.map((x) => x.length)))
  const labelEvery = Math.max(1, Math.ceil(xs.length / 6), Math.ceil((labelChars * 7 + 8) / band))
  return (
    <View className="gap-1.5" onLayout={(event) => setMeasured(event.nativeEvent.layout.width)}>
      {title ? <Text className="font-noto-medium text-[14px] leading-[20px]">{title}</Text> : null}
      <Svg width="100%" height={undefined} viewBox={`0 0 ${width} ${height}`} style={{ aspectRatio: width / height }} accessibilityLabel={title ?? 'Chart'}>
        {ticks.map((tick) => (
          <G key={tick}>
            <Line x1={left} x2={width - right} y1={y(tick)} y2={y(tick)} stroke={colors.border} strokeWidth={1} />
            <SvgText x={left - 6} y={y(tick) + 4} textAnchor="end" fontSize={12} fill={colors.mutedForeground}>
              {formatTick(tick)}
            </SvgText>
          </G>
        ))}
        {xs.map((x, index) =>
          index % labelEvery === 0 ? (
            <SvgText key={x} x={center(x)} y={height - bottom + 18} textAnchor="middle" fontSize={12} fill={colors.mutedForeground}>
              {x.length > 12 ? `${x.slice(0, 11)}…` : x}
            </SvgText>
          ) : null,
        )}
        {x_label ? (
          <SvgText x={left + (width - left - right) / 2} y={height - 2} textAnchor="middle" fontSize={12} fill={colors.mutedForeground}>
            {x_label}
          </SvgText>
        ) : null}
        {y_label ? (
          <SvgText x={10} y={top + (height - top - bottom) / 2} transform={`rotate(-90 10 ${top + (height - top - bottom) / 2})`} textAnchor="middle" fontSize={12} fill={colors.mutedForeground}>
            {y_label}
          </SvgText>
        ) : null}
        {series.map((line, index) => {
          const color = seriesColors[index]
          if (kind === 'line') {
            return (
              <G key={line.name}>
                <Polyline points={line.points.map((point) => `${center(String(point.x))},${y(point.y)}`).join(' ')} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
                {line.points.map((point) => (
                  <Circle key={String(point.x)} cx={center(String(point.x))} cy={y(point.y)} r={4} fill={color} stroke={colors.card} strokeWidth={2} />
                ))}
              </G>
            )
          }
          return line.points.map((point) => {
            // Bars of the series side by side in the band, 2px apart, rounded at the data end.
            const x = center(String(point.x)) - (thickness * series.length) / 2 + index * thickness + 1
            const barWidth = Math.max(thickness - 2, 1)
            const from = y(0)
            const to = y(point.y)
            const radius = Math.min(4, barWidth / 2, Math.abs(from - to))
            const path =
              to < from
                ? `M${x},${from}V${to + radius}Q${x},${to} ${x + radius},${to}H${x + barWidth - radius}Q${x + barWidth},${to} ${x + barWidth},${to + radius}V${from}Z`
                : `M${x},${from}V${to - radius}Q${x},${to} ${x + radius},${to}H${x + barWidth - radius}Q${x + barWidth},${to} ${x + barWidth},${to - radius}V${from}Z`
            return <Path key={`${line.name}-${point.x}`} d={path} fill={color} />
          })
        })}
        <Line x1={left} x2={width - right} y1={y(0)} y2={y(0)} stroke={colors.mutedForeground} strokeOpacity={0.5} strokeWidth={1} />
      </Svg>
      {series.length > 1 ? (
        <View className="flex-row flex-wrap gap-x-3 gap-y-1">
          {series.map((line, index) => (
            <View key={line.name} className="flex-row items-center gap-1.5">
              <View className="size-2.5 rounded-sm" style={{ backgroundColor: seriesColors[index] }} />
              <Text className="text-[12px] leading-[16px] text-muted-foreground">{line.name}</Text>
            </View>
          ))}
        </View>
      ) : null}
    </View>
  )
}

function CardButton({ props }: { props: { label: string; style?: 'primary' | 'secondary' | 'danger'; action: 'reply' | 'open_task'; params?: Record<string, unknown> } }) {
  const { live, pending, sent, send } = use(CardContext)
  const taskOf = useTaskOf()
  const id = idOf(props)
  const variant = props.style === 'danger' ? 'destructive' : props.style === 'primary' ? 'primary' : 'outline'
  const task = props.action === 'open_task' && typeof props.params?.task === 'string' ? taskOf(props.params.task) : undefined
  const done = props.action === 'reply' && sent.has(id)
  return (
    <View className="max-w-full flex-row items-center gap-2">
      <Button
        label={props.label}
        // 32px: a finger's target in a card
        size="default"
        variant={variant}
        loading={pending === id}
        // One tap at a time for each card, none while the agent still writes it, and each button once.
        disabled={props.action === 'open_task' ? !task : live || done || pending !== null}
        onPress={() => (props.action === 'open_task' ? task && router.push(`/tasks/${task.id}`) : send(id))}
      />
      {done ? <SentMark /> : null}
    </View>
  )
}

type FormField = { name: string; label: string; kind: 'text' | 'textarea' | 'select' | 'checkbox' | 'date'; options?: string[]; required?: boolean }

const dayText = (day: Date) => `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`

/** A short form: its values go to the agent from the person who sends it, without a chat message. Sent once. */
function CardForm({ props }: { props: { fields: FormField[]; submit_label?: string } }) {
  const { live, pending, sent, send } = use(CardContext)
  const { colors } = useTheme()
  const id = idOf(props)
  const [typed, setValues] = useState<CardValues>({})
  const [picking, setPicking] = useState<FormField | null>(null)
  const done = sent.has(id)
  // A sent form shows what was sent, also after a reload.
  const values = (done ? sent.get(id) : null) ?? typed
  const locked = live || done || pending !== null
  const set = (name: string, value: string | boolean) => setValues((old) => ({ ...old, [name]: value }))
  const missing = props.fields.some((field) => field.required && field.kind !== 'checkbox' && !values[field.name])
  return (
    <View className="gap-3">
      {props.fields.map((field) => {
        const value = values[field.name]
        if (field.kind === 'checkbox') {
          return (
            <Pressable
              key={field.name}
              accessibilityRole="checkbox"
              accessibilityState={{ checked: value === true, disabled: locked }}
              disabled={locked}
              onPress={() => set(field.name, value !== true)}
              className="flex-row items-center gap-2"
            >
              <View className={`size-4.5 items-center justify-center rounded border ${value === true ? 'border-primary bg-primary' : 'border-input'}`}>
                {value === true ? <Icon name="check" size={12} color={colors.primaryForeground} /> : null}
              </View>
              <Text className="text-[14px] leading-[20px]">{field.label}</Text>
            </Pressable>
          )
        }
        if (field.kind === 'select' || field.kind === 'date') {
          return (
            <View key={field.name} className="gap-2">
              <Text className="font-noto-medium text-[14px] leading-[19.25px]">
                {field.label}
                {field.required ? <Text className="text-muted-foreground"> *</Text> : null}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={field.label}
                disabled={locked}
                onPress={() => setPicking(field)}
                className={`min-h-8 justify-center rounded-lg border border-input px-2.5 dark:bg-input/30 ${locked ? 'opacity-50' : ''}`}
              >
                <Text className={`text-[16px] leading-[24px] ${typeof value === 'string' && value ? 'text-foreground' : 'text-muted-foreground'}`}>
                  {typeof value === 'string' && value ? value : field.kind === 'date' ? 'Choose a day…' : 'Choose…'}
                </Text>
              </Pressable>
            </View>
          )
        }
        return (
          <Input
            key={field.name}
            label={field.label}
            required={field.required}
            multiline={field.kind === 'textarea'}
            maxLength={2000}
            editable={!locked}
            value={String(value ?? '')}
            onChangeText={(text) => set(field.name, text)}
          />
        )
      })}
      <View className="flex-row items-center gap-2">
        <Button label={props.submit_label || 'Send'} loading={pending === id} disabled={locked || missing} onPress={() => send(id, values)} />
        {done ? <SentMark /> : null}
      </View>
      {picking?.kind === 'select' ? (
        <PickerSheet
          visible
          title={picking.label}
          options={(picking.options ?? []).map((option) => ({ key: option, label: option, selected: values[picking.name] === option }))}
          onSelect={(option) => {
            set(picking.name, option)
            setPicking(null)
          }}
          onClose={() => setPicking(null)}
        />
      ) : picking?.kind === 'date' ? (
        <DayPicker
          value={typeof values[picking.name] === 'string' && values[picking.name] ? new Date(`${values[picking.name]}T12:00:00`) : new Date()}
          onPick={(day) => {
            set(picking.name, dayText(day))
            setPicking(null)
          }}
          onCancel={() => setPicking(null)}
        />
      ) : null}
    </View>
  )
}

const TONES = { default: 'text-foreground/85', muted: 'text-muted-foreground', success: 'text-green-600', warning: 'text-amber-600', danger: 'text-destructive' } as const

const { registry } = defineRegistry(catalog, {
  components: {
    Card: ({ props, children }) => (
      <View className="gap-3 rounded-lg border border-border bg-card p-3">
        {props.title || props.description ? (
          <View className="gap-0.5">
            {props.title ? <Text className="font-noto-semibold text-[15px] leading-[20px]">{props.title}</Text> : null}
            {props.description ? <Text className="text-[13px] leading-[18px] text-muted-foreground">{props.description}</Text> : null}
          </View>
        ) : null}
        {children}
      </View>
    ),
    Stack: ({ props, children }) => <View className={props.direction === 'row' ? 'flex-row flex-wrap items-center gap-2' : 'gap-2'}>{children}</View>,
    Text: ({ props }) => <Text className={`text-[14px] leading-[21px] ${TONES[props.tone ?? 'default']}`}>{markdownToText(props.text)}</Text>,
    Table: ({ props }) => (
      <ScrollView horizontal className="rounded-md border border-border" contentContainerClassName="min-w-full">
        <View className="min-w-full">
          <View className="flex-row bg-muted/50">
            {props.columns.map((column) => (
              <Text key={column.key} className="w-32 px-2.5 py-1.5 font-noto-medium text-[12px] leading-[16px] text-muted-foreground" style={{ textAlign: column.align }}>
                {column.label}
              </Text>
            ))}
          </View>
          {props.rows.map((row, index) => (
            <View key={index} className="flex-row border-t border-border">
              {props.columns.map((column) => {
                const cell = row[column.key]
                return (
                  <View key={column.key} className="w-32 px-2.5 py-1.5">
                    {typeof cell === 'string' ? (
                      <TaskText text={cell} />
                    ) : (
                      <Text className="text-[14px] leading-[20px]" style={{ textAlign: column.align ?? (typeof cell === 'number' ? 'right' : undefined) }}>
                        {typeof cell === 'number' ? formatNumber(cell) : cell === true ? 'yes' : cell === false ? 'no' : ''}
                      </Text>
                    )}
                  </View>
                )
              })}
            </View>
          ))}
        </View>
      </ScrollView>
    ),
    Chart: ({ props }) => <Chart {...props} />,
    TaskList: ({ props }) => (
      <View className="rounded-md border border-border">
        {props.tasks.map((task, index) => (
          <View key={task.id} className={`flex-row flex-wrap items-baseline gap-x-2 px-2.5 py-1.5 ${index > 0 ? 'border-t border-border' : ''}`}>
            <TaskText text={task.id} className="font-noto-medium" />
            <Text className="min-w-0 flex-1 text-[14px] leading-[20px]">{task.title}</Text>
            <Text className="text-[12px] leading-[16px] text-muted-foreground">{[task.status, task.assignee, task.due].filter(Boolean).join(' · ')}</Text>
          </View>
        ))}
      </View>
    ),
    Form: ({ props }) => <CardForm props={props} />,
    Button: ({ props }) => <CardButton props={props} />,
  },
})

/** The render copy of a spec: each element gets its own id as a prop, for the components that send taps. */
function withIds(spec: UiSpec): UiSpec {
  return { root: spec.root, elements: Object.fromEntries(Object.entries(spec.elements).map(([id, element]) => [id, { ...element, props: { ...element.props, id } }])) }
}

/**
 * A card that the team agent answered with, under its text, as on the web. While the agent writes it (`live`), it
 * grows with each edit and takes no taps. A tap on a "reply" button or a form send goes to the agent without a chat
 * message: the button or form shows "Sent", and the agent answers in the thread.
 */
export function AgentCard({ card, messageId }: { card: Card; messageId: string }) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const key = keys.sentActions(workspaceId, messageId)
  const sentActions = useQuery({ queryKey: key, queryFn: () => sentCardActions(client, workspaceId, messageId), enabled: !card.live })
  const action = useMutation({
    mutationFn: (input: { elementId: string; values?: CardValues }) => sendCardAction(client, workspaceId, messageId, card.blockId, input.elementId, input.values),
    onSuccess: (_, input) => queryClient.setQueryData<SentAction[]>(key, (old) => [...(old ?? []), { blockId: card.blockId, elementId: input.elementId, values: input.values ?? null }]),
    onError: (error) => showError('The agent did not get the tap', error),
  })
  const pending = action.isPending ? (action.variables?.elementId ?? null) : null
  return (
    <View className="mt-1.5 gap-1.5">
      <CardContext value={{ live: card.live, pending, sent: sentOf(sentActions.data, card.blockId), send: (elementId, values) => action.mutate({ elementId, values }) }}>
        <JSONUIProvider registry={registry}>
          <Renderer spec={withIds(card.spec)} registry={registry} includeStandard={false} />
        </JSONUIProvider>
      </CardContext>
      {card.live ? (
        <View className="flex-row items-center gap-1.5">
          <Spinner />
          <Text className="text-[12px] leading-[16px] text-muted-foreground">Writing the card…</Text>
        </View>
      ) : null}
    </View>
  )
}
