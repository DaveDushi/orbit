import { Link } from 'react-router'
import { useLinkedTasks } from '@/features/docs/api/pages'
import { useAllStatuses, useProjects } from '@/features/tasks/api/projects'
import { taskIdentifier } from '@/features/tasks/api/models'
import { TaskStatusIcon } from '@/features/tasks/components/TaskStatusIcon'

/** "Linked tasks" under the page title: the tasks linked to this page that the reader sees; nothing when there are none. */
export function LinkedTasks({ workspaceId, pageId }: { workspaceId: string; pageId: string }) {
  const linked = useLinkedTasks(workspaceId, pageId).data ?? []
  const projects = useProjects(workspaceId, linked.length > 0).data ?? []
  const statuses = useAllStatuses(workspaceId, projects.filter((project) => linked.some(({ task }) => task.project_id === project.id))).data
  if (linked.length === 0) return null
  return (
    <section aria-label="Linked tasks" className="mb-5" data-print-hide="">
      <h3 className="mb-1 flex items-baseline gap-1.5 text-xs font-semibold text-muted-foreground">
        Linked tasks
        <span className="font-normal text-muted-foreground/60 tabular-nums">{linked.length}</span>
      </h3>
      <ul className="-mx-2 m-0 grid list-none gap-y-px p-0">
        {linked.map(({ task }) => {
          const identifier = taskIdentifier(task.id, projects.find((project) => project.id === task.project_id), task.number)
          return (
            <li key={task.id}>
              <Link
                to={`/tasks/${task.id}`}
                className="flex min-h-8 min-w-0 items-center gap-2 rounded-md px-2 text-xs outline-none transition-colors duration-150 ease-out hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                <TaskStatusIcon status={statuses.find((status) => status.id === task.status_id)} />
                <span className="shrink-0 text-muted-foreground tabular-nums">{identifier}</span>
                <span className="truncate text-foreground">{task.title || 'Untitled'}</span>
              </Link>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
