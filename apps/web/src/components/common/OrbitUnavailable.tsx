import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'

/**
 * The pixel "W" of the logo as a loader: its five strokes light up one after the other, left to right, over the shadow
 * of the mark. Sizes are multiples of 11px so the pixels stay crisp.
 */
export function OrbitLoader() {
  return (
    <svg data-slot="orbit-loader" viewBox="0 0 11 11" shapeRendering="crispEdges" aria-hidden="true" className="size-[66px]">
      <path fill="#742f4d" d="M1 1h2v8H1zM3 9h2v2H3zM5 5h2v4H5zM7 9h2v2H7zM9 1h2v8H9z" />
      <path fill="#f2458f" d="M0 0h2v8H0z" className="animate-orbit-chase motion-reduce:animate-none" />
      <path fill="#f2458f" d="M2 8h2v2H2z" className="animate-orbit-chase [animation-delay:320ms] motion-reduce:animate-none" />
      <path fill="#f2458f" d="M4 4h2v4H4z" className="animate-orbit-chase [animation-delay:640ms] motion-reduce:animate-none" />
      <path fill="#f2458f" d="M6 8h2v2H6z" className="animate-orbit-chase [animation-delay:960ms] motion-reduce:animate-none" />
      <path fill="#f2458f" d="M8 0h2v8H8z" className="animate-orbit-chase [animation-delay:1280ms] motion-reduce:animate-none" />
    </svg>
  )
}

interface OrbitUnavailableProps {
  detail: string
  onRetry: () => void
  /** Over the app, which stays mounted below with its state. Without it the screen is the page. */
  overlay?: boolean
}

/** The full screen for "Orbit cannot reach the server": the loader, why, and Retry. */
export function OrbitUnavailable({ detail, onRetry, overlay = false }: OrbitUnavailableProps) {
  const [retrying, setRetrying] = useState(false)

  // Retry has no answer of its own (the screen goes away when the connection is back): the button rests a moment.
  useEffect(() => {
    if (!retrying) return
    const timer = setTimeout(() => setRetrying(false), 4000)
    return () => clearTimeout(timer)
  }, [retrying])

  return (
    <main
      data-slot="orbit-unavailable"
      data-overlay={overlay ? '' : undefined}
      className="grid min-h-full place-items-center bg-background p-6 duration-200 animate-in fade-in data-overlay:fixed data-overlay:inset-0 data-overlay:z-100"
    >
      <div role="alert" className="flex max-w-sm flex-col items-center gap-4 text-center">
        <OrbitLoader />
        <div className="flex flex-col gap-1.5">
          <h1 className="text-lg font-semibold">Orbit is unavailable</h1>
          <p className="text-sm leading-[1.6] text-muted-foreground">{detail}</p>
        </div>
        <Button
          autoFocus
          disabled={retrying}
          onClick={() => {
            setRetrying(true)
            onRetry()
          }}
        >
          {retrying ? 'Reconnecting…' : 'Retry'}
        </Button>
      </div>
    </main>
  )
}
