/** The queries and writes of the native settings screens: custom emoji, notification devices and project settings. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { deletePushSubscription, listPushSubscriptions, sendTestPush } from '@orbit/api/generated/sdk.gen'
import type { LabelRecord, ProjectRecord, ProjectUpdateBody, ReorderItem, StatusRecord } from '@orbit/api/generated/types.gen'

import { chatQueryKeys } from '../chat-cache'
import type { UploadFile } from '../data/account-api'
import { customEmoji } from '../data/chat-api'
import { addEmoji, removeEmoji } from '../data/emoji'
import { keys } from '../data/keys'
import * as project from '../data/project-settings'
import { useOrbit } from '../orbit-context'

/** The workspace's custom emoji, the same cache the chat's picker reads. */
export function useCustomEmojiList(enabled: boolean) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: chatQueryKeys.customEmoji(workspaceId), queryFn: () => customEmoji(client, workspaceId), enabled })
}

export function useEmojiWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const refresh = () => void queryClient.invalidateQueries({ queryKey: chatQueryKeys.customEmoji(workspaceId) })
  return {
    create: useMutation({ mutationFn: ({ name, file }: { name: string; file: UploadFile }) => addEmoji(client, workspaceId, name, file), onSuccess: refresh }),
    remove: useMutation({ mutationFn: (id: string) => removeEmoji(client, workspaceId, { id }), onSettled: refresh }),
  }
}

const devicesKey = ['push-devices'] as const

/** The browsers that get this person's notifications (web push). The server does not list iPhones here. */
export function usePushBrowsers() {
  const { client } = useOrbit()
  return useQuery({ queryKey: devicesKey, queryFn: async () => (await listPushSubscriptions({ client, throwOnError: true })).data })
}

export function useRemovePushBrowser() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => void (await deletePushSubscription({ client, path: { id }, throwOnError: true })),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: devicesKey }),
  })
}

/** Resolves to the number of browsers and iPhones the test notification went to. */
export function useSendTestPush() {
  const { client } = useOrbit()
  return useMutation({ mutationFn: async () => (await sendTestPush({ client, throwOnError: true })).data.sent })
}

/** The project settings writes. Each one refreshes the lists the task screens read. */
export function useProjectWrites(projectId: string) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const refresh = (...lists: ('projects' | 'statuses' | 'labels' | 'tasks')[]) => () => {
    for (const list of lists) void queryClient.invalidateQueries({ queryKey: [list, workspaceId] })
  }
  return {
    update: useMutation({
      mutationFn: (body: ProjectUpdateBody) => project.saveProject(client, workspaceId, projectId, body),
      onSuccess: (saved) => queryClient.setQueryData<ProjectRecord[]>(keys.projects(workspaceId), (list) => list?.map((item) => (item.id === saved.id ? saved : item))),
      onSettled: refresh('projects'),
    }),
    remove: useMutation({ mutationFn: (record: ProjectRecord) => project.removeProject(client, workspaceId, record), onSettled: refresh('projects', 'statuses', 'labels', 'tasks') }),
    addStatus: useMutation({ mutationFn: ({ category, values }: { category: string; values: project.StatusValues }) => project.addStatus(client, workspaceId, projectId, category, values), onSettled: refresh('statuses') }),
    saveStatus: useMutation({ mutationFn: ({ status, values }: { status: StatusRecord; values: project.StatusValues }) => project.saveStatus(client, workspaceId, status, values), onSettled: refresh('statuses') }),
    removeStatus: useMutation({ mutationFn: (status: StatusRecord) => project.removeStatus(client, workspaceId, status), onSettled: refresh('statuses', 'tasks') }),
    reorder: useMutation({ mutationFn: (items: ReorderItem[]) => project.reorder(client, workspaceId, projectId, items), onSettled: refresh('statuses') }),
    addLabel: useMutation({ mutationFn: (body: { name: string; color: string }) => project.newLabel(client, workspaceId, projectId, body), onSettled: refresh('labels') }),
    saveLabel: useMutation({ mutationFn: ({ label, body }: { label: LabelRecord; body: { name: string; color: string } }) => project.saveLabel(client, workspaceId, label, body), onSettled: refresh('labels') }),
    removeLabel: useMutation({ mutationFn: (label: LabelRecord) => project.removeLabel(client, workspaceId, label), onSettled: refresh('labels', 'tasks') }),
  }
}

const projectMembersKey = (workspaceId: string, projectId: string) => ['projects', workspaceId, projectId, 'members'] as const

/** Who is listed on a private project. */
export function useProjectMembers(projectId: string, enabled: boolean) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: projectMembersKey(workspaceId, projectId), queryFn: () => project.members(client, workspaceId, projectId), enabled })
}

export function useProjectMemberWrites(projectId: string) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const refresh = () => void queryClient.invalidateQueries({ queryKey: ['projects', workspaceId] })
  return {
    add: useMutation({ mutationFn: async (userIds: string[]) => { for (const id of userIds) await project.addMember(client, workspaceId, projectId, id) }, onSettled: refresh }),
    remove: useMutation({ mutationFn: (userId: string) => project.removeMember(client, workspaceId, projectId, userId), onSettled: refresh }),
  }
}
