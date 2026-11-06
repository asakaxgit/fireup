#!/usr/bin/env node
// The approved drawings are frozen here, and this script is what makes that
// true rather than stated.
//
//   node scripts/lock.mjs                       -> verify, exit non-zero on a change
//   node scripts/lock.mjs --adopt "<approval>"  -> re-freeze, after a sign-off
//
// WHY THIS EXISTS. Rounds 5 to 13 each moved the drawing on a note from the
// CEO, and the rule written at the top of README.md through all of them —
// "the mark does not change again without going back through the CEO" — was
// enforced by nothing but my own care. It held for the mark: eight rounds of
// edits to the word never opened mark.svg. It is a bad thing to leave resting
// on care, because the failure is silent: round 11 found that editing a letter
// had quietly re-rendered 108 historical sheets, and round 12 found a sheet
// whose caption had gone a unit stale. Both were drift nobody asked for, and
// both were found by looking rather than by a check.
//
// So the signed-off geometry is recorded in lock/signed-off.json and compared
// element by element. A drawing that moves without an approval fails the build
// and the failure names the shape, the attribute and the two values.
//
// WHAT IS LOCKED, and why each one:
//   logo/mark.svg               the mark, round 5 — "A is perfect"
//   logo/favicon-16/32.svg      their own drawings of that mark at those sizes
//   logo/wordmark.svg           the word and the accent, round 13 — signed off
//   ../site/assets/fireup-lockup-dark.svg
//                               the BUILT lockup. The masters do not pin how
//                               build.mjs assembles them — SCALE, where the
//                               word starts, how the accent is placed — so a
//                               locked master with an unlocked build still
//                               lets the approved lockup move. This catches it.
//   tokens/brand.json color     the palette. The ember was answered on round
//                               2's card and the rest is derived from it.
//
// WHAT IS NOT LOCKED, deliberately: prose, comments, whitespace, ordering of
// attributes, and anything outside the whitelist below. The masters are meant
// to be read and annotated by hand — a comment explaining why a curve is where
// it is should never fail a build. Only the drawing is frozen.
//
// `npm run flush` is the companion check and they do different jobs: flush
// measures the rendered ink and asserts a *relationship* (the accent's edges
// land on the U's, the slot is 2 units), which stays true under a uniform
// move. This asserts the coordinates themselves. A change that preserves every
// relationship and still redraws the mark passes flush and fails here.
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const lockPath = join(root, 'lock', 'signed-off.json')

// Geometry and paint. Everything else about an element is documentation or
// accessibility and is free to change: role, aria-label, data-*, xmlns, id.
const DRAWING_ATTRS = [
  'viewBox',
  'transform',
  'd',
  'x', 'y', 'width', 'height',
  'cx', 'cy', 'r', 'rx', 'ry',
  'x1', 'y1', 'x2', 'y2',
  'points',
  'fill', 'fill-rule', 'fill-opacity',
  'stroke', 'stroke-width', 'stroke-linecap', 'stroke-linejoin',
  'opacity',
]

const stripComments = (svg) => svg.replace(/<!--[\s\S]*?-->/g, '')

// One line per drawn element, in document order: `rect x=6 y=16 width=12 ...`.
// Read as a diff, which is the point — an approved change to the drawing shows
// up in review as the coordinates that moved, not as a hash that differs.
const shapesOf = (svg) => {
  const out = []
  const tags = stripComments(svg).matchAll(/<([a-zA-Z][\w-]*)\b([^>]*?)\/?>/g)
  for (const [, tag, rawAttrs] of tags) {
    const attrs = []
    for (const name of DRAWING_ATTRS) {
      // Anchored on whitespace, not \b: `\bwidth=` also matches inside
      // `stroke-width="8"`, which put a phantom `width=8` on every stroked
      // element the first time this ran.
      const found = rawAttrs.match(new RegExp(`(?:^|\\s)${name}="([^"]*)"`))
      if (found) attrs.push(`${name}=${found[1].replace(/\s+/g, ' ').trim()}`)
    }
    if (attrs.length === 0) continue // a <g> that only groups, a <title>
    out.push(`${tag} ${attrs.join(' ')}`)
  }
  return out
}

const paletteOf = () => {
  const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))
  return Object.entries(tokens.color).map(([name, value]) => `${name}=${value}`)
}

const currentFor = (file) =>
  file === 'tokens/brand.json#color'
    ? paletteOf()
    : shapesOf(readFileSync(join(root, file), 'utf8'))

const lock = JSON.parse(readFileSync(lockPath, 'utf8'))
const files = Object.keys(lock.files)

const adoptAt = process.argv.indexOf('--adopt')
if (adoptAt !== -1) {
  const approval = process.argv[adoptAt + 1]
  if (!approval || approval.startsWith('--')) {
    console.error(
      'lock: --adopt needs the approval in the words it was given in, e.g.\n' +
      '  node scripts/lock.mjs --adopt "round 14 signed off: the accent goes back to 30"\n' +
      'Re-freezing without one would make this check a formality.',
    )
    process.exit(1)
  }
  for (const file of files) lock.files[file].shapes = currentFor(file)
  lock.history = [...(lock.history ?? []), { date: new Date().toISOString().slice(0, 10), approval }]
  writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`)
  console.log(`lock: re-froze ${files.length} drawings against "${approval}".`)
  process.exit(0)
}

const problems = []
for (const file of files) {
  const was = lock.files[file].shapes
  const now = currentFor(file)
  const lines = Math.max(was.length, now.length)
  for (let i = 0; i < lines; i += 1) {
    if (was[i] === now[i]) continue
    problems.push(
      `${file}\n` +
      `  signed off as: ${was[i] ?? '(nothing — this element is new)'}\n` +
      `  now draws     : ${now[i] ?? '(nothing — this element was removed)'}`,
    )
  }
}

if (problems.length > 0) {
  console.error(
    `lock: ${problems.length} approved drawing element${problems.length === 1 ? '' : 's'}` +
    ` changed.\n\n${problems.join('\n\n')}\n\n` +
    'The mark, the lockup and the palette are signed off. A change to any of\n' +
    'them goes back through the CEO — see "Open flags" in README.md. Once it is\n' +
    'approved, re-freeze with:\n' +
    '  npm run lock -- --adopt "<the approval, in the words it was given in>"',
  )
  process.exit(1)
}

const counts = files.map((file) => `${file} ${lock.files[file].shapes.length}`).join(', ')
console.log(`lock: ${files.length} approved drawings unchanged (${counts}).`)
