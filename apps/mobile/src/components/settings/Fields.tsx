/** Form parts the native settings screens share: a select, a check row, colour swatches, line tabs and web setup links. */
import * as WebBrowser from 'expo-web-browser'
import { useState } from 'react'
import { Pressable, ScrollView, View } from 'react-native'

import { useOrbit } from '../../orbit-context'
import { cssColors, useTheme } from '../../theme'
import { Icon, type IconName, PickerSheet, Text } from '../../ui'
import { FieldLabel, RowIcon, RowText, SettingsRow } from './Shell'

/** The web's `Select` field: the current choice, and a sheet with every choice. */
export function SelectField({ label, value, options, onChange, placeholder, description }: { label: string; value: string; options: readonly { value: string; label: string }[]; onChange: (value: string) => void; placeholder?: string; description?: string }) {
  const { colors } = useTheme()
  const [open, setOpen] = useState(false)
  const current = options.find((option) => option.value === value)?.label
  return (
    <View className="gap-2">
      <FieldLabel>{label}</FieldLabel>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityValue={{ text: current ?? placeholder }}
        onPress={() => setOpen(true)}
        className="min-h-8 flex-row items-center justify-between gap-2 rounded-lg border border-input px-2.5 py-[3px] dark:bg-input/30"
      >
        <Text numberOfLines={1} className={`flex-1 text-[16px] leading-[24px] ${current ? '' : 'text-muted-foreground'}`}>{current ?? placeholder ?? value}</Text>
        <Icon name="chevronDown" size={16} color={colors.mutedForeground} />
      </Pressable>
      {description ? <Text className="text-[12px] leading-[17px] text-muted-foreground">{description}</Text> : null}
      <PickerSheet
        visible={open}
        title={label}
        options={options.map((option) => ({ key: option.value, label: option.label, selected: option.value === value }))}
        onSelect={(key) => {
          setOpen(false)
          onChange(key)
        }}
        onClose={() => setOpen(false)}
      />
    </View>
  )
}

/** A checkbox row: the web's `<label><Checkbox/>…</label>`. */
export function CheckRow({ label, checked, onChange, hint, mono = false }: { label: string; checked: boolean; onChange: (checked: boolean) => void; hint?: string; mono?: boolean }) {
  const { colors } = useTheme()
  return (
    <Pressable accessibilityRole="checkbox" accessibilityLabel={label} accessibilityState={{ checked }} onPress={() => onChange(!checked)} className="min-h-9 flex-row items-center gap-2.5 py-1">
      <View className={`size-[18px] rounded-[5px] border items-center justify-center ${checked ? 'bg-primary border-primary' : 'border-input dark:bg-input/30'}`}>
        {checked ? <Icon name="check" size={13} color={colors.primaryForeground} /> : null}
      </View>
      <Text numberOfLines={1} className={`shrink ${mono ? 'font-mono text-[13px]' : 'text-[14px]'}`}>{label}</Text>
      {hint ? <Text className="text-[11px] text-muted-foreground">{hint}</Text> : null}
    </Pressable>
  )
}

/** A row of colour swatches; the chosen one has a check. */
export function ColorSwatches({ colors: palette, value, onChange, label }: { colors: readonly string[]; value: string; onChange: (color: string) => void; label: string }) {
  return (
    <View accessibilityRole="radiogroup" accessibilityLabel={label} className="flex-row flex-wrap gap-2">
      {palette.map((color) => {
        const selected = value.toLowerCase() === color.toLowerCase()
        return (
          <Pressable key={color} accessibilityRole="radio" accessibilityLabel={`Color ${color}`} accessibilityState={{ selected }} onPress={() => onChange(color)} className="size-7 rounded-md items-center justify-center" style={{ backgroundColor: color }}>
            {selected ? <Icon name="check" size={14} color={cssColors.white} /> : null}
          </Pressable>
        )
      })}
    </View>
  )
}

/** The web's line tabs (`TabsList variant="line"`): they scroll sideways when they do not fit. */
export function LineTabs<T extends string>({ value, tabs, onChange, label }: { value: T; tabs: readonly { value: T; label: string }[]; onChange: (value: T) => void; label: string }) {
  const { colors } = useTheme()
  return (
    <View className="border-b border-border">
      <ScrollView horizontal showsHorizontalScrollIndicator={false} accessibilityRole="tablist" accessibilityLabel={label} contentContainerClassName="gap-1.5">
        {tabs.map((tab) => {
          const selected = tab.value === value
          return (
            <Pressable key={tab.value} accessibilityRole="tab" accessibilityLabel={tab.label} accessibilityState={{ selected }} onPress={() => onChange(tab.value)} className="h-9 justify-center px-2.5" style={{ borderBottomWidth: 2, borderBottomColor: selected ? colors.foreground : 'transparent' }}>
              <Text className={`font-noto-medium text-[13px] ${selected ? 'text-foreground' : 'text-muted-foreground'}`}>
                {tab.label}
              </Text>
            </Pressable>
          )
        })}
      </ScrollView>
    </View>
  )
}

/**
 * A setup page that stays on the web, opened in an in-app browser (Safari View Controller on iOS). It has its own
 * sign-in: the app's session token never goes into a URL.
 */
export function WebSetupRow({ path, label, description, icon, first }: { path: string; label: string; description: string; icon: IconName; first: boolean }) {
  const { session } = useOrbit()
  const { colors } = useTheme()
  return (
    <SettingsRow first={first} label={`Open ${label} in the web app`} onPress={() => void WebBrowser.openBrowserAsync(`${session.server}${path}`, { presentationStyle: WebBrowser.WebBrowserPresentationStyle.PAGE_SHEET })}>
      <RowIcon icon={icon} />
      <RowText title={label} subtitle={description} />
      <Icon name="arrowUpRightSquare" size={14} color={colors.mutedForeground} />
    </SettingsRow>
  )
}
