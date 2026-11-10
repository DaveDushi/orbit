import { useRef, useState } from 'react'
import { Add, ArrowUp } from 'reicon-react'
import { Tip } from '@/components/common/Tip'
import { Button } from '@/components/ui/button'
import type { AskDonna } from './useAskDonna'

/** Chat's limit for one message. */
const MAX_LENGTH = 4000

/**
 * The small "Ask Donna…" box at the bottom center of every page but Chat. Enter sends the text to the person's DM with
 * the agent and opens the panel; Enter on an empty box opens the panel again. `+` takes files to the panel's composer.
 */
export function AskDonnaBar({ ask }: { ask: AskDonna }) {
  const [text, setText] = useState('')
  const picker = useRef<HTMLInputElement>(null)

  async function submit(files: File[] = []) {
    if (ask.busy) return
    if (await ask.ask(text, files)) setText('')
  }

  return (
    <form
      data-slot="ask-bar"
      aria-label={`Ask ${ask.name}`}
      className="absolute inset-x-0 bottom-4 z-20 mx-auto flex w-[min(440px,calc(100%-2rem))] items-center gap-1 rounded-full border bg-popover p-1 text-popover-foreground shadow-lg transition-[border-color,box-shadow] focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <Tip label="Attach files">
        <Button type="button" variant="ghost" size="icon" className="rounded-full" aria-label="Attach files" onClick={() => picker.current?.click()}>
          <Add className="size-5" />
        </Button>
      </Tip>
      <input
        ref={picker}
        type="file"
        multiple
        hidden
        onChange={(event) => {
          const files = Array.from(event.currentTarget.files ?? [])
          event.currentTarget.value = ''
          if (files.length) void submit(files)
        }}
      />
      <input
        aria-label={`Ask ${ask.name}`}
        placeholder={`Ask ${ask.name}…`}
        value={text}
        maxLength={MAX_LENGTH}
        autoComplete="off"
        className="h-8 min-w-0 flex-1 bg-transparent px-1 text-sm outline-none placeholder:text-muted-foreground"
        onChange={(event) => setText(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') event.currentTarget.blur()
          // also on an empty box, where the disabled Send button stops the form's own Enter
          if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
            event.preventDefault()
            void submit()
          }
        }}
      />
      <Tip label="Send">
        <Button type="submit" size="icon" className="rounded-full" aria-label="Send" disabled={ask.busy || !text.trim()}>
          <ArrowUp className="size-4" />
        </Button>
      </Tip>
    </form>
  )
}
