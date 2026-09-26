// Scales for the charts: a linear scale over floats for prices and
// quantities (read as floats for pixels only) and its ticks. Instants use
// `instant.js`: a scale over bigint differences, never over the instant.

/** A linear scale from `[d0, d1]` to `[r0, r1]`, clamped where asked. */
export function linear(d0, d1, r0, r1, { clamp = false } = {}) {
  const span = d1 - d0
  const scale = (value) => {
    if (!Number.isFinite(value)) return NaN
    const t = span === 0 ? 0.5 : (value - d0) / span
    const held = clamp ? Math.min(1, Math.max(0, t)) : t
    return r0 + held * (r1 - r0)
  }
  scale.invert = (pixel) => {
    const t = r1 === r0 ? 0 : (pixel - r0) / (r1 - r0)
    return d0 + t * span
  }
  scale.domain = () => [d0, d1]
  scale.range = () => [r0, r1]
  scale.ticks = (count = 5) => ticks(d0, d1, count)
  return scale
}

/** About `count` round values across `[lo, hi]`, as a float axis shows them. */
export function ticks(lo, hi, count = 5) {
  if (!(count > 0) || !Number.isFinite(lo) || !Number.isFinite(hi)) return []
  if (lo === hi) return [lo]
  const [start, end] = lo < hi ? [lo, hi] : [hi, lo]
  const step = niceStep((end - start) / count)
  const first = Math.ceil(start / step) * step
  const values = []
  for (let value = first; value <= end + step / 2; value += step) values.push(Number(value.toPrecision(12)))
  return values
}

/** The 1, 2, 5 or 10 multiple nearest a raw step. */
export function niceStep(raw) {
  if (!(raw > 0)) return 1
  const power = 10 ** Math.floor(Math.log10(raw))
  // The thresholds are the geometric means between the round steps
  // (sqrt 2, sqrt 10, sqrt 50), so a raw step lands on the nearer one.
  const ratio = raw / power
  const nice = ratio < Math.SQRT2 ? 1 : ratio < Math.sqrt(10) ? 2 : ratio < Math.sqrt(50) ? 5 : 10
  return nice * power
}

/** The domain `[lo, hi]` padded by `fraction` on each side, never collapsing. */
export function padded(lo, hi, fraction = 0.05) {
  if (lo === hi) {
    const pad = Math.abs(lo) * fraction || 1
    return [lo - pad, hi + pad]
  }
  const pad = (hi - lo) * fraction
  return [lo - pad, hi + pad]
}
