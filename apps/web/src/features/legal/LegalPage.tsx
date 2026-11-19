import { useEffect } from 'react'
import { Link } from 'react-router'
import { LEGAL_DRAFT, LEGAL_EFFECTIVE_DATE, LEGAL_PATHS, PRIVACY, TERMS, type LegalDocument } from './legal'

/** Marks the [placeholders] of a draft, so a reviewer sees every one. */
function Text({ children }: { children: string }) {
  return children.split(/(\[[^\]]+\])/).map((part, index) =>
    LEGAL_DRAFT && index % 2 === 1 ? <mark key={index} className="rounded-sm bg-primary/15 px-0.5 text-foreground">{part}</mark> : part,
  )
}

/** A public page (no sign-in): the Terms of Service or the Privacy Policy, as plain text. */
export function LegalPage({ page }: { page: 'terms' | 'privacy' }) {
  const doc: LegalDocument = page === 'terms' ? TERMS : PRIVACY
  const other = page === 'terms' ? { to: LEGAL_PATHS.privacy, label: PRIVACY.title } : { to: LEGAL_PATHS.terms, label: TERMS.title }

  useEffect(() => {
    const previous = document.title
    document.title = `${doc.title} · Orbit`
    return () => {
      document.title = previous
    }
  }, [doc.title])

  return (
    <main className="min-h-screen bg-background text-foreground">
      <article className="mx-auto w-full max-w-[72ch] px-5 py-10 text-[15px] leading-relaxed sm:py-16">
        <Link to="/" className="flex w-fit items-center gap-2 text-[13px] font-bold tracking-[0.08em] text-primary uppercase">
          <img src="/logo.svg" alt="" className="size-[22px]" />Orbit
        </Link>
        <h1 className="mt-8 text-2xl font-semibold">{doc.title}</h1>
        <p className="mt-1 text-[13px] text-muted-foreground">Effective: <Text>{LEGAL_EFFECTIVE_DATE}</Text></p>
        {LEGAL_DRAFT ? (
          <p role="note" className="mt-4 rounded-md border border-primary/30 bg-primary/10 px-3 py-2 text-[13px]">
            Draft: this text is not in effect yet. Highlighted words are placeholders.
          </p>
        ) : null}
        <p className="mt-6"><Text>{doc.intro}</Text></p>
        {doc.sections.map((section) => (
          <section key={section.title} className="mt-8">
            <h2 className="text-base font-semibold">{section.title}</h2>
            {section.body.map((block, index) =>
              typeof block === 'string' ? (
                <p key={index} className="mt-2"><Text>{block}</Text></p>
              ) : (
                <ul key={index} className="mt-2 grid list-disc gap-1.5 pl-5 marker:text-muted-foreground">
                  {block.map((item) => <li key={item}><Text>{item}</Text></li>)}
                </ul>
              ),
            )}
          </section>
        ))}
        <footer className="mt-12 flex flex-wrap gap-x-4 gap-y-2 border-t pt-4 text-[13px] text-muted-foreground">
          <Link to={other.to} className="hover:text-foreground">{other.label}</Link>
          <Link to="/" className="hover:text-foreground">Open Orbit</Link>
        </footer>
      </article>
    </main>
  )
}

/** The small "Terms · Privacy" links under the sign-in card and in the account menu. */
export function LegalLinks({ className }: { className?: string }) {
  return (
    <nav aria-label="Legal" className={className}>
      <Link to={LEGAL_PATHS.terms} className="hover:text-foreground">Terms</Link>
      <span aria-hidden> · </span>
      <Link to={LEGAL_PATHS.privacy} className="hover:text-foreground">Privacy</Link>
    </nav>
  )
}
