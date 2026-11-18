import { expect, test } from 'bun:test'
import { emailDocument, hasExternalImages } from './supportEmail'

const csp = (document: string) => /http-equiv="Content-Security-Policy" content="([^"]+)"/.exec(document)?.[1] ?? ''

test('an email frame blocks scripts, forms and external images until the reader allows them', () => {
  const blocked = csp(emailDocument('<p>x</p>', false))
  expect(blocked).toContain("default-src 'none'")
  expect(blocked).toContain("img-src 'self' data:;")
  expect(blocked).toContain("form-action 'none'")
  expect(blocked).not.toContain('https:')
  expect(csp(emailDocument('<p>x</p>', true))).toContain("img-src 'self' data: https: http:")
})

test('only images from other sites count as external', () => {
  expect(hasExternalImages('<img src="https://tracker.test/p.gif">')).toBeTrue()
  expect(hasExternalImages("<IMG alt='' SRC='http://x.test/a.png'>")).toBeTrue()
  expect(hasExternalImages('<img src="/api/v1/workspaces/w/tasks/t/attachments/a/download">')).toBeFalse()
  expect(hasExternalImages('<a href="https://example.com">link</a>')).toBeFalse()
})
