import { useState } from 'react'
import { Calendar } from 'reicon-react'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'
import { DatePicker } from '@/components/common/DatePicker'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import type { Task, TaskStatusDef } from '@/features/tasks/api/models'
import { useUpdateTask } from '@/features/tasks/api/tasks'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { DueDateChip } from './TaskPropertyChips'

/**
 * Due-date chip that opens the date picker in place (list rows and board cards). Without a date it shows a
 * calendar icon only while the surrounding `group/row` is hovered. Font size comes from `className`.
 */
export function DueDatePicker({ task, status, className }: { task: Task; status: TaskStatusDef | undefined; className?: string }) {
  const { workspace } = useWorkspace()
  const updateTask = useUpdateTask(workspace.id)
  const [open, setOpen] = useState(false)
  const setDueDate = (dueStartAt: string | null, dueAt: string | null) => {
    updateTask.mutate({ taskId: task.id, body: { expected_version: task.version, due_start_at: dueStartAt, due_at: dueAt } })
    setOpen(false)
  }
  return (
    <div className={cn('flex', className)} onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              className="inline-flex h-auto cursor-pointer rounded-none border-0 bg-transparent p-0 text-[length:inherit] font-normal hover:bg-transparent aria-expanded:bg-transparent dark:hover:bg-transparent"
              aria-label={task.dueAt ? 'Due date' : 'Set due date'}
              title={task.dueAt ? undefined : 'Set due date'}
            >
              {task.dueAt
                ? <DueDateChip task={task} status={status} className="hover:text-foreground" />
                // no date yet: a quiet icon that shows on row hover, keyboard focus, or while the picker is open
                : <Calendar aria-hidden className="size-3.5 text-muted-foreground opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-visible/button:opacity-100 group-aria-expanded/button:opacity-100 hover:text-foreground" />}
            </Button>
          }
        />
        <PopoverContent align="end" className="w-auto gap-0 p-0">
          <DatePicker
            startValue={task.dueStartAt ?? null}
            value={task.dueAt}
            onClear={() => setDueDate(null, null)}
            onDone={({ start, end }) => setDueDate(start, end)}
          />
        </PopoverContent>
      </Popover>
    </div>
  )
}
