#!/usr/bin/env node
// Proves the raster pipeline end to end: render a known SVG, then assert the
// PNG is a real PNG of the requested size with the expected pixel colours.
// Also asserts no master draws outside its own viewBox.
// Run with `npm run smoke` in brand/.
import { renderPng, contentBox } from './render.mjs'
import { master } from './word.mjs'
import { inflateSync } from 'node:zlib'

const SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
  <rect width="16" height="16" fill="#0E1726"/>
  <rect x="4" y="4" width="8" height="8" fill="#FF5A2B"/>
</svg>`

const readU32 = (buf, at) => buf.readUInt32BE(at)

const pixelAt = (png, width, x, y) => {
  // Minimal PNG reader: we only need IHDR geometry plus a couple of pixels, and
  // the IDAT stream is one zlib blob for an image this small.
  let at = 8
  let idat = Buffer.alloc(0)
  let bitDepth, colorType
  while (at < png.length) {
    const len = readU32(png, at)
    const type = png.subarray(at + 4, at + 8).toString('ascii')
    const data = png.subarray(at + 8, at + 8 + len)
    if (type === 'IHDR') {
      bitDepth = data[8]
      colorType = data[9]
    }
    if (type === 'IDAT') idat = Buffer.concat([idat, data])
    at += len + 12
  }
  if (bitDepth !== 8 || colorType !== 6) throw new Error(`unexpected PNG format ${bitDepth}/${colorType}`)
  const pixels = unfilter(inflateSync(idat), width, readU32(png, 20))
  const off = y * width * 4 + x * 4
  return [pixels[off], pixels[off + 1], pixels[off + 2], pixels[off + 3]]
}

// Reverses the five PNG scanline filters (RFC 2083 §6) for 8-bit RGBA.
const unfilter = (raw, width, height) => {
  const bpp = 4
  const rowBytes = width * bpp
  const out = Buffer.alloc(rowBytes * height)
  const paeth = (a, b, c) => {
    const p = a + b - c
    const pa = Math.abs(p - a)
    const pb = Math.abs(p - b)
    const pc = Math.abs(p - c)
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c
  }
  for (let y = 0; y < height; y += 1) {
    const type = raw[y * (rowBytes + 1)]
    const src = y * (rowBytes + 1) + 1
    const dst = y * rowBytes
    const prev = dst - rowBytes
    for (let i = 0; i < rowBytes; i += 1) {
      const x = raw[src + i]
      const a = i >= bpp ? out[dst + i - bpp] : 0
      const b = y > 0 ? out[prev + i] : 0
      const c = i >= bpp && y > 0 ? out[prev + i - bpp] : 0
      const value =
        type === 0 ? x
        : type === 1 ? x + a
        : type === 2 ? x + b
        : type === 3 ? x + ((a + b) >> 1)
        : type === 4 ? x + paeth(a, b, c)
        : (() => { throw new Error(`unknown filter ${type}`) })()
      out[dst + i] = value & 0xff
    }
  }
  return out
}

const checks = []
const check = (name, fn) => {
  try {
    fn()
    checks.push(['ok', name])
  } catch (error) {
    checks.push(['FAIL', `${name} — ${error.message}`])
  }
}

const png512 = renderPng(SVG, 512)
const png16 = renderPng(SVG, 16)

check('output is a PNG', () => {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  if (!png512.subarray(0, 8).equals(sig)) throw new Error('bad signature')
})

check('512 request produces a 512x512 image', () => {
  const w = readU32(png512, 16)
  const h = readU32(png512, 20)
  if (w !== 512 || h !== 512) throw new Error(`got ${w}x${h}`)
})

check('16 request produces a 16x16 image', () => {
  const w = readU32(png16, 16)
  const h = readU32(png16, 20)
  if (w !== 16 || h !== 16) throw new Error(`got ${w}x${h}`)
})

check('background pixel is the ink colour', () => {
  const [r, g, b, a] = pixelAt(png512, 512, 4, 4)
  if (r !== 0x0e || g !== 0x17 || b !== 0x26 || a !== 255) throw new Error(`got ${r},${g},${b},${a}`)
})

check('foreground pixel is the ember colour', () => {
  const [r, g, b, a] = pixelAt(png512, 512, 256, 256)
  if (r !== 0xff || g !== 0x5a || b !== 0x2b || a !== 255) throw new Error(`got ${r},${g},${b},${a}`)
})

// Round 4 shipped a flame that ran past its own viewBox. The renderer cut the
// outer lobe flat and nothing below 512px showed it, because a master is only
// ever rendered through its own viewBox — so ink outside it is invisible rather
// than wrong-looking. Measuring the drawing through a *widened* viewBox is the
// only way to see it, and that is what this does. It fails the build.
const MASTERS = ['mark', 'wordmark', 'favicon-16', 'favicon-32']

check('no master draws outside its own viewBox', () => {
  const clipped = []
  for (const name of MASTERS) {
    const svg = master(name)
    const [x, y, w, h] = svg
      .match(/viewBox="([^"]+)"/)[1]
      .trim()
      .split(/[\s,]+/)
      .map(Number)
    const pad = { x: w / 2, y: h / 2 }
    const wide = svg.replace(
      /viewBox="[^"]+"/,
      `viewBox="${x - pad.x} ${y - pad.y} ${w + pad.x * 2} ${h + pad.y * 2}"`,
    )
    const box = contentBox(wide)
    // The box comes off a raster, so an edge sitting exactly on the viewBox
    // edge lands within one sampled pixel of it. One pixel of slack.
    const slack = (w + pad.x * 2) / 1024
    const over = [
      box.x < x - slack && `left by ${(x - box.x).toFixed(1)}`,
      box.y < y - slack && `top by ${(y - box.y).toFixed(1)}`,
      box.x + box.w > x + w + slack && `right by ${(box.x + box.w - x - w).toFixed(1)}`,
      box.y + box.h > y + h + slack && `bottom by ${(box.y + box.h - y - h).toFixed(1)}`,
    ].filter(Boolean)
    if (over.length) clipped.push(`${name}.svg overflows ${over.join(', ')}`)
  }
  if (clipped.length) throw new Error(clipped.join('; '))
})

for (const [status, name] of checks) console.log(`${status.padEnd(4)} ${name}`)
const failed = checks.filter(([status]) => status !== 'ok').length
console.log(`\n${checks.length - failed}/${checks.length} passed`)
process.exit(failed ? 1 : 0)
