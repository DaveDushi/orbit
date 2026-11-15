/**
 * Where the iOS keyboard covers a view. React Native measures a view from the top of its root: the app, or the
 * `Modal` it is in (`measureInWindow` stops at a modal, whose origin it takes as 0). The keyboard's frame is on the
 * screen. A root that starts at the top of the screen (the app, a screen, a transparent modal) is at 0; a page sheet
 * ends at the bottom of the screen, so it starts at the screen's height minus its own.
 */

/** The top of a page sheet on the screen, from the screen's height and the sheet's. */
export function sheetTop(screenHeight: number, sheetHeight: number): number {
  return Math.max(0, screenHeight - sheetHeight)
}

/** How much of a view the keyboard covers: the view's bottom on the screen, below the keyboard's top. */
export function keyboardCover({ rootTop, y, height, keyboardTop }: { rootTop: number; y: number; height: number; keyboardTop: number }): number {
  return Math.max(0, Math.round(rootTop + y + height - keyboardTop))
}
