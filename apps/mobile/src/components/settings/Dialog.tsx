import { type ReactNode, useEffect, useState } from 'react'
import { KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { alpha, cssColors, type, useTheme } from '../../theme'
import { Button, Icon, type IconName, Input, Text } from '../../ui'

export interface SheetAction {
  label: string
  icon?: IconName
  destructive?: boolean
  disabled?: boolean
  onPress: () => void
}

/** The web's dropdown menus at phone size: a bottom sheet of rows. A row closes the sheet, then runs its action. */
export function ActionSheet({ visible, title, actions, onClose }: { visible: boolean; title?: string; actions: readonly SheetAction[]; onClose: () => void }) {
  const { colors } = useTheme()
  const insets = useSafeAreaInsets()
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      <Pressable accessibilityLabel="Close" onPress={onClose} style={{ flex: 1, backgroundColor: alpha(cssColors.black, 0.5) }} />
      <View style={{ position: 'absolute', left: 8, right: 8, bottom: 8 + insets.bottom, borderRadius: 14, backgroundColor: colors.popover, borderWidth: 1, borderColor: colors.border, paddingVertical: 6 }}>
        {title ? (
          <Text numberOfLines={1} style={[type(12, 'medium'), { paddingHorizontal: 14, paddingTop: 6, paddingBottom: 6, color: colors.mutedForeground }]}>
            {title}
          </Text>
        ) : null}
        {actions.map((action) => (
          <Pressable
            key={action.label}
            accessibilityRole="button"
            accessibilityLabel={action.label}
            accessibilityState={{ disabled: action.disabled }}
            disabled={action.disabled}
            onPress={() => {
              onClose()
              action.onPress()
            }}
            style={({ pressed }) => ({ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 10, marginHorizontal: 6, paddingHorizontal: 8, borderRadius: 8, opacity: action.disabled ? 0.5 : 1, backgroundColor: pressed ? colors.muted : 'transparent' })}
          >
            {action.icon ? <Icon name={action.icon} size={16} color={action.destructive ? colors.destructive : colors.mutedForeground} /> : null}
            <Text style={[type(14, 'medium'), { color: action.destructive ? colors.destructive : colors.foreground }]}>{action.label}</Text>
          </Pressable>
        ))}
      </View>
    </Modal>
  )
}

/** The web `Modal`: centered on both axes, title, description, body and the footer buttons. */
export function Dialog({ visible, title, description, children, footer, onClose }: { visible: boolean; title: string; description?: string; children?: ReactNode; footer: ReactNode; onClose: () => void }) {
  const { colors } = useTheme()
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={{ flex: 1 }}>
        <Pressable accessibilityLabel="Close" onPress={onClose} style={{ position: 'absolute', inset: 0, backgroundColor: alpha(cssColors.black, 0.5) }} />
        <View pointerEvents="box-none" style={{ flex: 1, justifyContent: 'center', padding: 16 }}>
          <View accessibilityRole="none" accessibilityLabel={title} style={{ maxHeight: '90%', borderRadius: 14, backgroundColor: colors.popover, borderWidth: 1, borderColor: colors.border }}>
            <ScrollView contentContainerStyle={{ padding: 16, gap: 14 }} keyboardShouldPersistTaps="handled">
              <View style={{ gap: 4 }}>
                <Text style={[type(16, 'semibold')]}>{title}</Text>
                {description ? <Text style={[type(13), { lineHeight: 19, color: colors.mutedForeground }]}>{description}</Text> : null}
              </View>
              {children}
              <View style={{ flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'flex-end', gap: 8 }}>{footer}</View>
            </ScrollView>
          </View>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  )
}

export interface ConfirmRequest {
  title: string
  description?: string
  confirmLabel: string
  danger?: boolean
  onConfirm: () => void
}

/** A confirmation before an action with an impact (`confirmAction` on the web). `request: null` hides it. */
export function ConfirmDialog({ request, onClose }: { request: ConfirmRequest | null; onClose: () => void }) {
  return (
    <Dialog
      visible={request !== null}
      title={request?.title ?? ''}
      description={request?.description}
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" onPress={onClose} />
          <Button
            label={request?.confirmLabel ?? 'Confirm'}
            variant={request?.danger ? 'destructive' : 'primary'}
            onPress={() => {
              onClose()
              request?.onConfirm()
            }}
          />
        </>
      }
    />
  )
}

export interface PromptRequest {
  title: string
  description?: string
  label: string
  initial?: string
  placeholder?: string
  confirmLabel: string
  secure?: boolean
  keyboard?: 'default' | 'email-address' | 'number-pad' | 'url'
  /** Resolves to close; rejects with the message to show under the field. */
  onSubmit: (value: string) => Promise<unknown>
}

/** A one-field form in a dialog (rename, new teamspace, confirm with your password …). `request: null` hides it. */
export function PromptDialog({ request, onClose }: { request: PromptRequest | null; onClose: () => void }) {
  const [value, setValue] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setValue(request?.initial ?? '')
    setError(null)
    setBusy(false)
  }, [request])

  const submit = () => {
    if (!request || busy || !value.trim()) return
    setBusy(true)
    setError(null)
    request.onSubmit(request.secure ? value : value.trim()).then(onClose, (reason: unknown) => {
      setBusy(false)
      setError(reason instanceof Error ? reason.message : String(reason))
    })
  }

  return (
    <Dialog
      visible={request !== null}
      title={request?.title ?? ''}
      description={request?.description}
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" onPress={onClose} />
          <Button label={request?.confirmLabel ?? 'Save'} loading={busy} disabled={!value.trim()} onPress={submit} />
        </>
      }
    >
      <Input
        label={request?.label}
        value={value}
        onChangeText={setValue}
        placeholder={request?.placeholder}
        secureTextEntry={request?.secure}
        autoComplete={request?.secure ? 'current-password' : undefined}
        autoCapitalize={request?.secure || request?.keyboard === 'email-address' || request?.keyboard === 'url' ? 'none' : 'sentences'}
        keyboardType={request?.keyboard ?? 'default'}
        autoFocus
        returnKeyType="done"
        onSubmitEditing={submit}
        error={error ?? undefined}
      />
    </Dialog>
  )
}
