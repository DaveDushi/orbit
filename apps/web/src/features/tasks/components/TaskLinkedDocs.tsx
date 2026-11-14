import { Link } from 'react-router'
import { DocumentText as FileText, Xmark as X } from 'reicon-react'
import { Button } from '@/components/ui/button'
import { Emoji } from '@/components/common/Emoji'
import type { LinkedDoc } from '@orbit/api/generated/types.gen'

/** Ghost button beside Add relation and Add sub-issue: opens the page picker. */
export function AddDocButton({ onClick }: { onClick: () => void }) {
  return (
    <Button type="button" variant="ghost" className="text-xs text-muted-foreground/70" onClick={onClick}>
      <FileText className="size-3.5" aria-hidden="true" />
      Add doc
    </Button>
  )
}

/** Main-column section like "Relations": the linked Docs pages the reader sees; each row opens its page or unlinks it. */
export function TaskLinkedDocsSection({ docs, removingId, onRemove }: {
  docs: LinkedDoc[]
  removingId: string | undefined
  onRemove: (pageId: string) => void
}) {
  if (docs.length === 0) return null
  return (
    <section aria-label="Docs" className="mt-6 border-t pt-4">
      <h3 className="mb-2 flex items-baseline gap-1.5 text-xs font-semibold text-muted-foreground">
        Docs
        <span className="font-normal text-muted-foreground/60 tabular-nums">{docs.length}</span>
      </h3>
      {/* -mx-2 lets the hover fill bleed past the text edge, as in Relations */}
      <ul className="-mx-2 m-0 grid list-none gap-y-px p-0">
        {docs.map((doc) => {
          const title = doc.title || 'Untitled'
          return (
            <li key={doc.id} className="group/doc flex min-h-8 items-center rounded-md transition-colors duration-150 ease-out hover:bg-muted">
              <Link
                to={`/docs/${doc.id}`}
                className="flex min-w-0 flex-1 items-center gap-2 self-stretch rounded-md px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                {doc.icon ? <Emoji value={doc.icon} /> : <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />}
                <span className="truncate text-foreground">{title}</span>
                {doc.private ? <span className="shrink-0 text-muted-foreground/70">Private</span> : null}
              </Link>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Unlink ${title}`}
                disabled={removingId === doc.id}
                className="mr-1 shrink-0 text-muted-foreground hover-fine:opacity-0 hover-fine:group-hover/doc:opacity-100 focus-visible:opacity-100"
                onClick={() => onRemove(doc.id)}
              >
                <X className="size-3" />
              </Button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
