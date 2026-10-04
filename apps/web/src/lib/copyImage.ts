/** Encode the displayed image as PNG, the image format supported by the clipboard. */
async function imagePng(url: string): Promise<Blob> {
  const image = new Image()
  image.crossOrigin = 'anonymous'
  image.src = url
  await image.decode()
  const canvas = document.createElement('canvas')
  canvas.width = image.naturalWidth
  canvas.height = image.naturalHeight
  const context = canvas.getContext('2d')
  if (!context) throw new Error('Image conversion is unavailable')
  context.drawImage(image, 0, 0)
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('Image conversion failed')), 'image/png')
  })
}

/** Start the clipboard write during the click; Safari requires this before any await. */
export async function copyImage(url: string): Promise<void> {
  if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') {
    throw new Error('Image copying is unavailable')
  }
  const png = imagePng(url)
  // A rejected write may not consume the pending image promise.
  void png.catch(() => {})
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })])
}
