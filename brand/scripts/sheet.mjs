#!/usr/bin/env node
// The comparison sheet: N options side by side, each rendered at every size it
// is really used at, plus its actual 16px raster magnified with no resampling.
//
// fire.mjs and tune.mjs predate this module and carry their own copy; they are
// the record of decisions already made, so they are left alone rather than
// rewritten. Anything new builds its sheets from here.
import { pixelMap } from './render.mjs'

const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28

export const text = (x, y, body, { size = 15, fill = '#FFFFFF', weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

const noteLines = (x, y, body, fill) =>
  body
    .split('\n')
    .map((line, i) => text(x, y + i * 17, line, { size: 13, fill, weight: 'normal' }))
    .join('')

// options: [{ label, note, draw(fg, accent) }] drawing into a 64-unit viewBox.
export const sheet = ({ options, fg, accent, bg, muted, caption, sizes = [96, 48, 32, 16], col = 400, zoomScale = 8 }) => {
  const rowTop = PAD + 56
  const zoomTop = rowTop + sizes[0] + 36
  const zoom = 16 * zoomScale
  const height = zoomTop + zoom + 116
  const width = PAD * 2 + col * options.length

  const columns = options
    .map((option, index) => {
      const x = PAD + col * index
      const body = option.draw(fg, accent)
      let tileX = x
      const tiles = sizes
        .map((size) => {
          const tile = `<g transform="translate(${tileX} ${rowTop + (sizes[0] - size) / 2}) scale(${size / 64})">${body}</g>`
          tileX += size + 24
          return tile
        })
        .join('')
      const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${bg}"/>${body}</svg>`
      return `
      ${text(x, PAD + 22, option.label, { fill: fg })}
      ${tiles}
      ${text(x, zoomTop - 12, `16px, magnified ${zoomScale}x — real pixels`, { size: 12, fill: muted, weight: 'normal' })}
      <g transform="translate(${x} ${zoomTop})">${pixelMap(tiny, 16, zoomScale)}</g>
      ${noteLines(x + zoom + 20, zoomTop + 18, option.note, muted)}`
    })
    .join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${text(PAD, height - 24, caption, { size: 14, fill: muted, weight: 'normal' })}
  ${columns}
</svg>`
}
