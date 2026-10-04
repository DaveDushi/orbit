import { afterEach, expect, test } from 'bun:test'
import { copyImage } from './copyImage'

const originalImage = globalThis.Image
const originalItem = globalThis.ClipboardItem
const originalClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
const originalContext = HTMLCanvasElement.prototype.getContext
const originalToBlob = HTMLCanvasElement.prototype.toBlob

afterEach(() => {
  globalThis.Image = originalImage
  globalThis.ClipboardItem = originalItem
  if (originalClipboard) Object.defineProperty(navigator, 'clipboard', originalClipboard)
  else Reflect.deleteProperty(navigator, 'clipboard')
  HTMLCanvasElement.prototype.getContext = originalContext
  HTMLCanvasElement.prototype.toBlob = originalToBlob
})

function setup(decode: () => Promise<void>, write: (png: Promise<Blob>) => Promise<void>) {
  globalThis.Image = class {
    naturalWidth = 32
    naturalHeight = 24
    decode = decode
  } as unknown as typeof Image
  globalThis.ClipboardItem = class {
    data: Record<string, Promise<Blob>>
    constructor(data: Record<string, Promise<Blob>>) { this.data = data }
  } as unknown as typeof ClipboardItem
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
    write: (items: { data: Record<string, Promise<Blob>> }[]) => write(items[0].data['image/png']),
  } })
  HTMLCanvasElement.prototype.getContext = (() => ({ drawImage() {} })) as unknown as typeof originalContext
  HTMLCanvasElement.prototype.toBlob = function (callback, type) {
    expect(this.width).toBe(32)
    expect(this.height).toBe(24)
    callback(new Blob(['encoded image'], { type }))
  }
}

test('starts writing before decoding finishes and resolves only after the PNG is written', async () => {
  let finishDecode!: () => void
  let started = false
  let copied: Blob | undefined
  setup(() => new Promise<void>((resolve) => { finishDecode = resolve }), async (png) => {
    started = true
    copied = await png
  })
  const result = copyImage('/image.jpg')
  expect(started).toBe(true)
  expect(copied).toBeUndefined()
  finishDecode()
  await result
  expect(copied?.type).toBe('image/png')
  expect(await copied?.text()).toBe('encoded image')
})

test('a failed image decode fails the copy', async () => {
  setup(async () => { throw new Error('Invalid image') }, async (png) => { await png })
  await expect(copyImage('/broken.jpg')).rejects.toThrow('Invalid image')
})

test('a denied clipboard write fails the copy', async () => {
  setup(async () => {}, async () => { throw new Error('Permission denied') })
  await expect(copyImage('/image.png')).rejects.toThrow('Permission denied')
})

test('a failed PNG conversion fails the copy', async () => {
  setup(async () => {}, async (png) => { await png })
  HTMLCanvasElement.prototype.toBlob = (callback) => callback(null)
  await expect(copyImage('/image.jpg')).rejects.toThrow('Image conversion failed')
})

test('unsupported browsers fail without starting image conversion', async () => {
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined })
  await expect(copyImage('/image.png')).rejects.toThrow('Image copying is unavailable')
})
