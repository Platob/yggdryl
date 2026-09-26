// What the two canvas charts share: a canvas sized to its box at the device
// pixel ratio, the theme's colour tokens read off the element (a chart never
// names a colour of its own), and the signals that ask for a redraw - the box
// changing size or the device pixels under it changing, and the theme changing.

/**
 * Size `canvas`'s bitmap to its laid-out box at `devicePixelRatio`; answers
 * the 2D context scaled so drawing is in CSS pixels, and the box.
 */
export function fitCanvas(canvas) {
  const ratio = globalThis.devicePixelRatio || 1
  const width = canvas.clientWidth
  const height = canvas.clientHeight
  const pixelWidth = Math.max(1, Math.round(width * ratio))
  const pixelHeight = Math.max(1, Math.round(height * ratio))
  if (canvas.width !== pixelWidth) canvas.width = pixelWidth
  if (canvas.height !== pixelHeight) canvas.height = pixelHeight
  const context = canvas.getContext('2d')
  context.setTransform(ratio, 0, 0, ratio, 0, 0)
  context.clearRect(0, 0, width, height)
  return { context, width, height, ratio }
}

/** The theme's tokens as `element` resolves them: `tokens(el, ['bid'])` reads `--ygg-ui-bid`. */
export function tokens(element, names) {
  const style = getComputedStyle(element)
  return Object.fromEntries(names.map((name) => [name, style.getPropertyValue(`--ygg-ui-${name}`).trim()]))
}

/**
 * Call `redraw` whenever `element`'s box changes size, or the device pixel
 * ratio does - a zoom, a move to another screen - which leaves the box as it
 * was and the bitmap blurred; answers the stop function.
 */
export function watchSize(element, redraw) {
  const observer = new ResizeObserver(() => redraw())
  observer.observe(element)
  // A media query matches one ratio: it is asked again at the ratio it changed to.
  let media = null
  const onRatio = () => {
    arm()
    redraw()
  }
  const arm = () => {
    media = matchMedia(`(resolution: ${globalThis.devicePixelRatio || 1}dppx)`)
    media.addEventListener('change', onRatio, { once: true })
  }
  arm()
  return () => {
    observer.disconnect()
    media.removeEventListener('change', onRatio)
  }
}

/** Call `redraw` when `data-theme` on `<html>` or the colour-scheme preference changes; answers the stop function. */
export function watchTheme(redraw) {
  const media = matchMedia('(prefers-color-scheme: dark)')
  const handler = () => redraw()
  media.addEventListener('change', handler)
  const observer = new MutationObserver(handler)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
  return () => {
    media.removeEventListener('change', handler)
    observer.disconnect()
  }
}

/** A round axis label: the tick's float at twelve significant digits, trailing zeros dropped. */
export function tickLabel(value) {
  return String(Number(value.toPrecision(12)))
}
