// the chat reference CodeBlock + highlightCode (MessageItem.tsx): fenced code with copy button and
// hand-rolled JS/Rust token colors.
import { CopyCodeButton } from './CopyCodeButton'

/* ---------- code block (the chat reference CodeBlock + highlightCode) ---------- */

const TOKEN_CLASS: Record<string, string> = {
  comment: 'text-[#6e7781] dark:text-[#8b949e]',
  string: 'text-[#0a3069] dark:text-[#a5d6ff]',
  number: 'text-[#0550ae] dark:text-[#79c0ff]',
  type: 'text-[#953800] dark:text-[#ffa657]',
  keyword: 'text-[#cf222e] dark:text-[#ff7b72]',
}

function highlightCode(code: string, language: string): React.ReactNode[] {
  const normalizedLanguage = language.toLowerCase()
  const keywords =
    normalizedLanguage === 'rust'
      ? 'as|async|await|break|const|continue|crate|dyn|else|enum|extern|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|type|unsafe|use|where|while'
      : 'break|case|catch|class|const|continue|default|else|export|for|function|if|import|let|new|return|switch|throw|try|var|while'
  const types =
    normalizedLanguage === 'rust'
      ? 'bool|char|f32|f64|i8|i16|i32|i64|i128|isize|str|String|u8|u16|u32|u64|u128|usize|Option|Result|Vec'
      : 'boolean|number|string|Array|Promise|Record|Set|Map'
  const tokenRegex = new RegExp(
    `(//.*|#.*|"(?:\\\\.|[^"\\\\])*"|'(?:\\\\.|[^'\\\\])*'|\\b(?:${keywords})\\b|\\b(?:${types})\\b|\\b\\d+(?:\\.\\d+)?\\b)`,
    'g',
  )
  const typeRegex = new RegExp(`^(?:${types})$`)
  const parts: React.ReactNode[] = []
  let lastIndex = 0
  let tokenIndex = 0
  let match: RegExpExecArray | null

  while ((match = tokenRegex.exec(code)) !== null) {
    if (match.index > lastIndex) parts.push(code.slice(lastIndex, match.index))
    const token = match[0]
    let kind = 'keyword'
    if (token.startsWith('//') || token.startsWith('#')) kind = 'comment'
    else if (token.startsWith('"') || token.startsWith("'")) kind = 'string'
    else if (/^\d/.test(token)) kind = 'number'
    else if (typeRegex.test(token)) kind = 'type'
    parts.push(
      <span key={`code-token-${match.index}-${tokenIndex}`} className={TOKEN_CLASS[kind]} data-kind={kind}>
        {token}
      </span>,
    )
    lastIndex = match.index + token.length
    tokenIndex += 1
  }

  if (lastIndex < code.length) parts.push(code.slice(lastIndex))
  return parts.length > 0 ? parts : [code]
}

export function CodeBlock({ code, language = '' }: { code: string; language?: string }) {
  return (
    <div className="group relative my-1 w-fit max-w-full min-[900px]:max-w-[80%]">
      <pre className="w-full rounded-lg border border-border bg-muted/20 py-3 pr-12 pl-3 font-mono text-[13px] leading-5 font-semibold whitespace-pre-wrap text-foreground/85 [overflow-wrap:anywhere]">
        <code className="font-[inherit] whitespace-pre-wrap [overflow-wrap:anywhere]">{highlightCode(code, language)}</code>
      </pre>
      <CopyCodeButton getText={() => code} className="absolute top-2 right-2" />
    </div>
  )
}
