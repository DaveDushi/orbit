/** Whether sanitized email HTML loads images from other sites (blocked until the reader allows them). */
export function hasExternalImages(html: string): boolean {
  return /<img\b[^>]*\bsrc\s*=\s*["']?https?:/i.test(html)
}

/**
 * The document of an email's frame. The HTML was cleaned on the server; the frame adds a CSP of its own (it also
 * inherits the app's): no scripts, no requests but the app's own (inline attachments) and, once allowed, images.
 */
export function emailDocument(html: string, showImages: boolean): string {
  const images = showImages ? "'self' data: https: http:" : "'self' data:"
  return `<!doctype html><html><head><meta charset="utf-8">`
    + `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${images}; style-src 'self'; font-src 'none'; form-action 'none'">`
    + `<link rel="stylesheet" href="/support-email.css"><base target="_blank"></head><body>${html}</body></html>`
}
