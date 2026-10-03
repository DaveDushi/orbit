import { useContext } from 'react'
import { InternalLinkContext } from '@/lib/internalLinkContext'
import { appNavigate } from '@/lib/navigateBridge'

/** A bare link to this app in rendered markdown: it navigates in place, unless the host draws it (`InternalLinkContext`). */
export function InternalLink({ url, path }: { url: string; path: string }) {
  const render = useContext(InternalLinkContext)
  const plain = (
    <a
      href={path}
      className="text-primary hover:underline"
      onClick={(e) => {
        e.preventDefault()
        appNavigate(path)
      }}
    >
      {url}
    </a>
  )
  return render ? render(url, plain) : plain
}
