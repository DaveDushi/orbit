import { useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useMemo, useState } from 'react'
import { Alert, Pressable, ScrollView, View } from 'react-native'

import { Segmented } from '../components/settings/Shell'
import { askDonnaAgent } from '../data/ask-donna'
import { conversationTitle, newNonce } from '../data/chat'
import * as chat from '../data/chat-api'
import { driveChildren, uploadToDrive } from '../data/drive'
import { keys } from '../data/keys'
import { shareConversations, sharedContent, type SharedFile, shareTargets, type ShareTarget, shareText, shareToNewTask, taskTitleOf } from '../data/share'
import * as taskApi from '../data/task-api'
import { defaultStatusOf, fileSize } from '../data/tasks'
import { driveFileOf } from '../drive-file'
import { useAgentStatus } from '../hooks/agent'
import { useIsGuest, useOrbit } from '../orbit-context'
import { useConversations, usePeople, useProjects, useStatuses } from '../queries'
import { errorMessage } from '../session-context'
import { useIncomingShareItems } from '../share-intent'
import { alpha, useTheme } from '../theme'
import { Button, EmptyState, Icon, IconButton, Input, LoadingView, PaneHeader, SafeAreaView, Text } from '../ui'
import { uploadPart } from '../upload-part'

const TARGET_LABELS: Record<ShareTarget, string> = { chat: 'Chat', drive: 'Drive', task: 'New task' }

/**
 * Share to Orbit: what another app shared (photos, videos, files, links, text) goes to a conversation (also the DM with
 * the agent), a Drive folder, or a new task. The server checks each write as the person.
 */
export default function ShareScreen() {
  const { colors } = useTheme()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const guest = useIsGuest()
  const share = useIncomingShareItems()
  const content = useMemo(() => sharedContent(share.payloads), [share.payloads])
  const targets = shareTargets(guest)
  const [target, setTarget] = useState<ShareTarget>('chat')
  const [note, setNote] = useState('')
  const [conversationId, setConversationId] = useState<string | null>(null)
  // the Drive folders opened, from the top level down
  const [folderPath, setFolderPath] = useState<{ id: string; name: string }[]>([])
  const [projectId, setProjectId] = useState<string | null>(null)
  const [title, setTitle] = useState<string | null>(null)
  const [progress, setProgress] = useState<string | null>(null)
  const [failure, setFailure] = useState<string | null>(null)

  const conversations = useConversations()
  const people = usePeople()
  const agent = askDonnaAgent(useAgentStatus(!guest).data, guest)
  const projects = useProjects()
  const statuses = useStatuses()
  const folder = folderPath.at(-1) ?? null
  const folderId = folder?.id ?? null
  const folders = useQuery({
    queryKey: keys.driveChildren(workspaceId, folderId),
    queryFn: () => driveChildren(client, workspaceId, folderId),
    enabled: target === 'drive',
    select: (items) => items.filter((item) => item.kind === 'folder'),
  })
  const chats = useMemo(() => shareConversations(conversations.data ?? [], user.id, agent?.botId ?? null), [agent?.botId, conversations.data, user.id])
  // the agent's DM is made on send when it does not exist yet
  const agentDm = agent ? chats.find((item) => item.kind === 'dm' && item.member_ids.length === 2 && item.member_ids.includes(agent.botId)) : undefined
  const AGENT_KEY = 'agent'
  const taskTitle = title ?? taskTitleOf(content)
  const project = (projects.data ?? []).find((item) => item.id === projectId) ?? projects.data?.[0]
  const muted = alpha(colors.mutedForeground, 0.7)

  const done = (path: string) => {
    share.clear()
    router.replace(path as never)
  }
  const cancel = () => {
    share.clear()
    if (router.canGoBack()) router.back()
    else router.replace('/')
  }

  const run = async (work: () => Promise<void>) => {
    setFailure(null)
    try {
      await work()
    } catch (error) {
      setFailure(errorMessage(error))
    } finally {
      setProgress(null)
    }
  }
  const counted = (verb: string, index: number, files: readonly SharedFile[]) => `${verb} ${index + 1} of ${files.length}: ${files[index]?.name ?? ''}`

  const sendToChat = () =>
    run(async () => {
      let id = conversationId === AGENT_KEY ? (agentDm?.id ?? null) : conversationId
      if (conversationId === AGENT_KEY && !id && agent) id = (await chat.openDm(client, workspaceId, [agent.botId])).result.id
      if (!id) throw new Error('Choose a conversation.')
      const fileIds: string[] = []
      for (const [index, file] of content.files.entries()) {
        setProgress(counted('Uploading', index, content.files))
        fileIds.push((await chat.uploadFile(client, workspaceId, { part: await uploadPart(file.uri, file.name) })).id)
      }
      setProgress('Sending…')
      await chat.send(client, workspaceId, id, { body: shareText(note, content.texts), nonce: newNonce(), fileIds })
      done(`/chat/${id}`)
    })

  const saveToDrive = () =>
    run(async () => {
      for (const [index, file] of content.files.entries()) {
        setProgress(counted('Uploading', index, content.files))
        await uploadToDrive(client, workspaceId, folderId, await driveFileOf(file.uri, file.name, file.mimeType))
      }
      void queryClient.invalidateQueries({ queryKey: keys.drive(workspaceId) })
      done(folderId ? `/drive/${folderId}` : '/drive')
    })

  const createTask = () =>
    run(async () => {
      const status = project && statuses.data ? defaultStatusOf(statuses.data, project.id) : undefined
      if (!project || !status) throw new Error('Choose a project.')
      if (!taskTitle.trim()) throw new Error('Give the task a title.')
      setProgress('Creating the task…')
      let index = 0
      const { task, failed } = await shareToNewTask(
        {
          createTask: () =>
            taskApi.createTaskWith(
              client,
              workspaceId,
              taskApi.newTaskBody({ title: taskTitle, description: shareText(note, content.texts), projectId: project.id, statusId: status.id, priority: 'none', assigneeIds: [], labelIds: [], dueAt: null, dueStartAt: null, parentId: null }),
            ),
          attach: async (taskId, file) => {
            setProgress(counted('Attaching', index++, content.files))
            await taskApi.uploadTaskFile(client, workspaceId, taskId, { name: file.name, blob: await uploadPart(file.uri, file.name) })
          },
        },
        content.files,
      )
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      if (failed.length > 0) Alert.alert('Task created', `The task was created, but these files were not attached: ${failed.join(', ')}.`)
      done(`/tasks/${task.id}`)
    })

  const empty = !share.resolving && content.files.length === 0 && content.texts.length === 0
  const busy = progress !== null
  const filesOnly = target === 'drive'
  return (
    <SafeAreaView edges={['top', 'left', 'right', 'bottom']} className="flex-1 bg-background">
      <PaneHeader>
        <IconButton icon="close" label="Cancel" color={muted} onPress={cancel} />
        <Text className="font-noto-semibold text-[13px] flex-1">Share to Orbit</Text>
      </PaneHeader>
      {share.resolving ? (
        <LoadingView />
      ) : empty ? (
        <EmptyState icon="import" title="Nothing to share" description="Share photos, videos, files or links to Orbit from another app." />
      ) : (
        <ScrollView contentContainerClassName="px-4 pb-6 gap-4" keyboardShouldPersistTaps="handled">
          <View accessibilityLabel="Shared items" className="rounded-lg border border-border">
            {content.files.map((file, index) => (
              <View key={`${file.uri}-${index}`} className="h-11 flex-row items-center gap-2.5 px-3 border-b border-border">
                <Icon name={file.mimeType.startsWith('image/') || file.mimeType.startsWith('video/') ? 'play' : 'attach'} size={15} color={muted} />
                <Text numberOfLines={1} className="text-[13px] flex-1">
                  {file.name}
                </Text>
                {file.size !== null ? <Text className="text-[12px] text-muted-foreground">{fileSize(file.size)}</Text> : null}
              </View>
            ))}
            {content.texts.map((text, index) => (
              <View key={`text-${index}`} className="min-h-11 flex-row items-center gap-2.5 px-3 py-2 border-b border-border">
                <Icon name="text" size={15} color={muted} />
                <Text numberOfLines={2} className="text-[13px] flex-1">
                  {text}
                </Text>
              </View>
            ))}
          </View>

          {targets.length > 1 ? <Segmented label="Share to" value={target} options={targets.map((value) => ({ value, label: TARGET_LABELS[value] }))} onChange={setTarget} /> : null}
          {filesOnly && content.files.length === 0 ? <Text className="text-[13px] text-muted-foreground">Drive takes files: share links and text to Chat or a new task.</Text> : null}
          {filesOnly && content.texts.length > 0 && content.files.length > 0 ? <Text className="text-[13px] text-muted-foreground">Only the files go to Drive.</Text> : null}

          {target === 'chat' ? (
            <View className="gap-3">
              <Input label="Message" placeholder="Add a message" multiline value={note} onChangeText={setNote} />
              <View accessibilityRole="radiogroup" accessibilityLabel="Conversation" className="rounded-lg border border-border overflow-hidden">
                {agent && !agentDm ? <ChoiceRow label={`${agent.name} (your conversation)`} icon="magicStar" selected={conversationId === AGENT_KEY} onPress={() => setConversationId(AGENT_KEY)} /> : null}
                {conversations.isLoading ? <LoadingView /> : null}
                {chats.map((item) => {
                  const isAgent = item.id === agentDm?.id
                  return (
                    <ChoiceRow
                      key={item.id}
                      label={isAgent && agent ? agent.name : item.kind === 'dm' ? conversationTitle(item, people, user.id) : `#${item.name}`}
                      icon={isAgent ? 'magicStar' : item.kind === 'dm' ? 'user' : item.kind === 'private' ? 'lock' : 'hashtag'}
                      selected={conversationId === item.id || (isAgent && conversationId === AGENT_KEY)}
                      onPress={() => setConversationId(item.id)}
                    />
                  )
                })}
              </View>
            </View>
          ) : null}

          {target === 'drive' ? (
            <View accessibilityRole="radiogroup" accessibilityLabel="Drive folder" className="rounded-lg border border-border overflow-hidden">
              <View className="h-11 flex-row items-center gap-2 px-3 bg-muted">
                {folder ? <IconButton icon="back" label="Up one folder" size={24} iconSize={14} color={muted} onPress={() => setFolderPath((path) => path.slice(0, -1))} /> : null}
                <Icon name="folder" size={15} color={colors.primary} />
                <Text numberOfLines={1} className="font-noto-medium text-[13px] flex-1">
                  {folder?.name ?? 'Drive'}
                </Text>
              </View>
              {folders.isLoading ? <LoadingView /> : null}
              {folders.error ? <Text className="text-[13px] p-3 text-destructive">{errorMessage(folders.error)}</Text> : null}
              {(folders.data ?? []).map((item) => (
                <ChoiceRow key={item.id} label={item.name} icon="folder" chevron onPress={() => setFolderPath((path) => [...path, { id: item.id, name: item.name }])} />
              ))}
            </View>
          ) : null}

          {target === 'task' ? (
            <View className="gap-3">
              <Input label="Title" required placeholder="Task title" value={taskTitle} onChangeText={setTitle} />
              <Input label="Description" placeholder="Add a description" multiline value={note} onChangeText={setNote} />
              <Text className="font-noto-medium text-[14px]">Project</Text>
              <View accessibilityRole="radiogroup" accessibilityLabel="Project" className="rounded-lg border border-border overflow-hidden">
                {(projects.data ?? []).map((item) => (
                  <ChoiceRow key={item.id} label={item.name} icon="tasks" selected={project?.id === item.id} onPress={() => setProjectId(item.id)} />
                ))}
              </View>
              {content.files.length > 0 ? <Text className="text-[13px] text-muted-foreground">The files are attached to the task.</Text> : null}
            </View>
          ) : null}

          {progress ? <Text accessibilityRole="progressbar" className="text-[13px] text-muted-foreground">{progress}</Text> : null}
          {failure ? <Text accessibilityRole="alert" className="text-[13px] text-destructive">{failure}</Text> : null}
          {target === 'chat' ? <Button label="Send" block loading={busy} disabled={busy || conversationId === null} onPress={() => void sendToChat()} /> : null}
          {target === 'drive' ? <Button label={`Save to ${folder?.name ?? 'Drive'}`} block loading={busy} disabled={busy || content.files.length === 0} onPress={() => void saveToDrive()} /> : null}
          {target === 'task' ? <Button label="Create task" block loading={busy} disabled={busy || !project || !taskTitle.trim()} onPress={() => void createTask()} /> : null}
        </ScrollView>
      )}
    </SafeAreaView>
  )
}

function ChoiceRow({ label, icon, selected = false, chevron = false, onPress }: { label: string; icon: Parameters<typeof Icon>[0]['name']; selected?: boolean; chevron?: boolean; onPress: () => void }) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole={chevron ? 'button' : 'radio'}
      accessibilityLabel={label}
      accessibilityState={chevron ? undefined : { selected }}
      onPress={onPress}
      className={`h-11 flex-row items-center gap-2.5 px-3 border-b border-border active:bg-muted ${selected ? 'bg-primary/10' : ''}`}
    >
      <Icon name={icon} size={15} color={selected ? colors.primary : alpha(colors.mutedForeground, 0.8)} />
      <Text numberOfLines={1} className={`text-[13px] flex-1 ${selected ? 'font-noto-medium text-primary' : ''}`}>
        {label}
      </Text>
      {selected ? <Icon name="check" size={15} color={colors.primary} /> : null}
      {chevron ? <Icon name="chevronRight" size={14} color={alpha(colors.mutedForeground, 0.7)} /> : null}
    </Pressable>
  )
}
