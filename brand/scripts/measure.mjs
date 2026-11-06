#!/usr/bin/env node
// How much background is actually left between two shapes.
//
// Both measurements here were written for round 10 (scripts/tighten.mjs) and
// wanted again unchanged by round 11, so they live here rather than being
// copied. They answer the only question a clearance number is really asked in:
// not "what does the arithmetic say", but "is there a slot left once the
// renderer has drawn it at the size it is used at".
import { Resvg } from '@resvg/resvg-js'

// Rasterise a body through a given viewBox at a given width, as a 1-bit ink
// mask plus the raw pixels.
const layerOf = (viewBox, body, width) => {
  const { pixels, width: w, height: h } = new Resvg(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}">${body}</svg>`,
    { fitTo: { mode: 'width', value: width } },
  ).render()
  const ink = new Uint8Array(w * h)
  for (let i = 0; i < w * h; i += 1) ink[i] = pixels[i * 4 + 3] > 8 ? 1 : 0
  return { pixels, ink, w, h }
}

// The shortest distance from any pixel of `b` to any pixel of `a`, in device
// pixels at `width`. A two-pass chamfer transform, weights 1 and sqrt(2) —
// within a few percent of true Euclidean, far inside what matters here.
//
// 0 means the two shapes have fused into one silhouette, and anything under
// about 1 means the renderer has nothing left to put the slot in.
export const inkGap = (viewBox, a, b, width) => {
  const A = layerOf(viewBox, a, width)
  const B = layerOf(viewBox, b, width)
  if (A.w !== B.w || A.h !== B.h) throw new Error('inkGap: layers disagree on size')
  const { w, h } = A
  const FAR = 1e9
  const dist = new Float64Array(w * h)
  for (let i = 0; i < w * h; i += 1) dist[i] = A.ink[i] ? 0 : FAR
  const D = Math.SQRT2
  const relax = (to, from, cost) => {
    const candidate = dist[from] + cost
    if (candidate < dist[to]) dist[to] = candidate
  }
  for (let y = 0; y < h; y += 1) {
    for (let x = 0; x < w; x += 1) {
      const i = y * w + x
      if (x > 0) relax(i, i - 1, 1)
      if (y > 0) relax(i, i - w, 1)
      if (y > 0 && x > 0) relax(i, i - w - 1, D)
      if (y > 0 && x < w - 1) relax(i, i - w + 1, D)
    }
  }
  for (let y = h - 1; y >= 0; y -= 1) {
    for (let x = w - 1; x >= 0; x -= 1) {
      const i = y * w + x
      if (x < w - 1) relax(i, i + 1, 1)
      if (y < h - 1) relax(i, i + w, 1)
      if (y < h - 1 && x < w - 1) relax(i, i + w + 1, D)
      if (y < h - 1 && x > 0) relax(i, i + w - 1, D)
    }
  }
  let best = FAR
  for (let i = 0; i < w * h; i += 1) if (B.ink[i] && dist[i] < best) best = dist[i]
  return best
}

// The slot between an upper shape and a lower one, column by column, in the
// viewBox's own units: how long it runs, how tight it gets, and the air
// averaged over its run.
//
// The average is the point. A dot over a stem pinches at one point and opens
// out on both sides; a flat tail over a flat cap is parallel, so its mean air
// is its minimum. Two slots with the same minimum do not read the same if one
// of them is parallel, which is why the identity's two accents could not simply
// be given the same number.
//
// `limit` discards columns where the two shapes are not really facing each
// other across the slot — without it, the far side of a letter counts as air.
export const slotRun = ({ viewBox, boxWidth, upper, lower, limit, width = 1400 }) => {
  const A = layerOf(viewBox, upper, width)
  const B = layerOf(viewBox, lower, width)
  const unit = boxWidth / A.w
  const ceiling = limit / unit
  const gaps = []
  for (let x = 0; x < A.w; x += 1) {
    let lowestUpper = -1
    let highestLower = -1
    for (let y = 0; y < A.h; y += 1) if (A.ink[y * A.w + x]) lowestUpper = y
    if (lowestUpper < 0) continue
    for (let y = B.h - 1; y >= 0; y -= 1) if (B.ink[y * B.w + x]) highestLower = y
    if (highestLower < 0) continue
    const air = highestLower - lowestUpper
    if (air > 0 && air <= ceiling) gaps.push(air * unit)
  }
  return {
    run: gaps.length * unit,
    mean: gaps.length ? gaps.reduce((sum, g) => sum + g, 0) / gaps.length : 0,
    min: gaps.length ? Math.min(...gaps) : 0,
  }
}
