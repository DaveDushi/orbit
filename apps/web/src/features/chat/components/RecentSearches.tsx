import type { RefObject } from 'react'
import { History } from 'reicon-react'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent } from '@/components/ui/popover'

interface RecentSearchesProps {
  open: boolean
  onClose: () => void
  /** The search field: the list opens under it and leaves the focus in it. */
  anchor: RefObject<HTMLElement | null>
  popup: RefObject<HTMLDivElement | null>
  searches: readonly string[]
  onPick: (query: string) => void
  onClear: () => void
}

/** The last searches under the empty search field of the conversation header. A click searches again. */
export function RecentSearches({ open, onClose, anchor, popup, searches, onPick, onClear }: RecentSearchesProps) {
  return (
    <Popover open={open && searches.length > 0} onOpenChange={(next) => (next ? undefined : onClose())} modal={false}>
      <PopoverContent
        ref={popup}
        anchor={anchor}
        align="start"
        initialFocus={false}
        finalFocus={false}
        aria-label="Recent searches"
        className="w-(--anchor-width) min-w-56 gap-0 p-1"
        // a click keeps the focus in the field (WebKit does not focus a clicked button), so its blur does not close the list
        onMouseDown={(event) => event.preventDefault()}
      >
        <div className="flex items-center justify-between px-2 py-1">
          <span className="text-xs font-medium text-muted-foreground">Recent searches</span>
          <Button type="button" variant="ghost" size="xs" className="text-muted-foreground" onClick={onClear}>
            Clear
          </Button>
        </div>
        <ul className="flex flex-col">
          {searches.map((query) => (
            <li key={query}>
              <Button type="button" variant="ghost" size="sm" className="w-full justify-start font-normal" onClick={() => onPick(query)}>
                <History aria-hidden className="text-muted-foreground" />
                <span className="truncate">{query}</span>
              </Button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
