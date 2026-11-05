#!/usr/bin/env node
// The wordmark master, taken apart by name.
//
// Everything that composes a lockup needs the same two things: the word's
// construction numbers, and some subset of its letters. Both used to be
// retyped in every script that wanted them — build.mjs had its own copy of the
// baseline and the i's tittle, and both build.mjs and mix.mjs selected "ireup"
// as "every path after the first two". That is fine until a letter is added or
// reordered, at which point the lockup quietly draws the wrong thing. Round 6
// already cost one wrong constant that way.
//
// So: the numbers live on the root of logo/wordmark.svg as data-*, the letters
// live in flat groups tagged data-glyph, and this module is the only thing that
// knows how to read either.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const logo = join(dirname(fileURLToPath(import.meta.url)), '..', 'logo')

export const master = (name) => readFileSync(join(logo, `${name}.svg`), 'utf8')

export const innerOf = (svg) => svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '')

export const recolour = (svg, fg, accent) => svg.replaceAll('#FFFFFF', fg).replaceAll('#FF5A2B', accent)

const number = (svg, attr) => {
  const found = svg.match(new RegExp(`data-${attr}="([-\\d.]+)"`))
  if (!found) throw new Error(`master is missing data-${attr}`)
  return Number(found[1])
}

const rootTag = (svg) => svg.match(/<svg[^>]*>/)[0]

// The presentation attributes on the master's root, which every letter group
// inherits. A composer that lifts groups out of the master has to carry these
// with them or the letters come out unpainted.
const PAINT = ['fill', 'stroke', 'stroke-width', 'stroke-linecap', 'stroke-linejoin']
const paintOf = (svg) => {
  const tag = rootTag(svg)
  return PAINT.map((name) => {
    const found = tag.match(new RegExp(`\\s${name}="([^"]*)"`))
    return found ? `${name}="${found[1]}"` : null
  })
    .filter(Boolean)
    .join(' ')
}

// name -> markup, in drawing order.
const glyphsOf = (svg) => {
  const found = new Map()
  const groups = svg.match(/<g data-glyph="[^"]+">[\s\S]*?<\/g>/g) ?? []
  for (const group of groups) {
    const name = group.match(/data-glyph="([^"]+)"/)[1]
    found.set(name, group.replace(/^<g [^>]*>/, '').replace(/<\/g>$/, '').trim())
  }
  return found
}

const WORDMARK = master('wordmark')

// Construction numbers, read once from the drawing that defines them.
export const W = {
  baseline: number(WORDMARK, 'baseline'),
  xTop: number(WORDMARK, 'x-top'),
  ascTop: number(WORDMARK, 'asc-top'),
  descender: number(WORDMARK, 'descender'),
  stroke: number(WORDMARK, 'stroke'),
  ireupL: number(WORDMARK, 'ireup-left'),
  letterGap: number(WORDMARK, 'letter-gap'),
}

export const GLYPHS = glyphsOf(WORDMARK)
export const WORD_PAINT = paintOf(WORDMARK)

// The i's tittle, read off its own drawing rather than retyped. It is also the
// reference for any other accent over a letter: its underside sits
// `W.xTop - W.stroke / 2 - (cy + r)` above the stem it floats over.
const tittle = GLYPHS.get('tittle').match(/cx="([\d.]+)"[^>]*cy="([\d.]+)"[^>]*r="([\d.]+)"/)
export const DOT = { cx: Number(tittle[1]), cy: Number(tittle[2]), r: Number(tittle[3]) }

// How far the tittle's underside sits above the x-height LINE — not above the
// i's ink.
//
// Rounds 7 to 9 used this as "the gap the word keeps between a floating accent
// and the letter under it", and placed the arrow over the U with it. It is not
// that gap. The i's stem is a butt-capped vertical: it terminates on y=20
// rather than straddling it, so its topmost ink is 4 units below where
// `W.xTop - W.stroke / 2` puts it, and the tittle really keeps 7. The capital U
// is built the same way, so the arrow ended up 7 clear of the letter while
// every comment in the identity said 3.
//
// The value is kept because it is what rounds 7 and 8 drew against, and their
// sheets have to keep reproducing. Anything new measures the ink instead — see
// scripts/tighten.mjs, which reads both edges off the raster every run.
export const ACCENT_CLEARANCE = W.xTop - W.stroke / 2 - (DOT.cy + DOT.r)

// The same quantity as a past round computed it. Rounds 7 and 8 *placed the
// arrow* with this number, so it is geometry in their sheets and not just a
// caption: left reading the live tittle, round 11's dot would move their
// drawings. Defined below HISTORY, which it reads.
export const accentClearanceAsOf = (round) => {
  const dot = dotAsOf(round)
  return W.xTop - W.stroke / 2 - (dot.cy + dot.r)
}

// --- letters as earlier rounds drew them --------------------------------------
// An exploration sheet is the evidence for a decision that has already been
// made, so it has to keep showing what was shown. But every one of those
// scripts composes the word by asking this module for letters, which means an
// edit to the master silently re-renders the record: round 11 moved the i's
// dot and the U, and re-running the round 7 to 10 scripts rewrote 108 of their
// sheets with letters those rounds never saw. Round 10's own sheet came back
// measuring 3 units of air under a tittle it had captioned as keeping 7.
//
// So a script that documents a past round pins itself to that round, and the
// letters it pins to live here, written out in full rather than described. A
// pin is a literal copy of a drawing that is no longer in the master — it is
// not derived from the current one, because then it would move with it again.
const HISTORY = {
  // Rounds 7 to 10. The U's stems stop on the ascender line, the i's tittle
  // sits 4 units higher, and the accent over the U is 3 units lower. All three
  // moved in round 11; nothing else in the word has changed since round 7.
  'round-10': {
    U: '<path d="M120 4 V35 A15 15 0 0 0 150 35 V4"/>',
    tittle: '<circle cx="34" cy="9" r="4" fill="#FFFFFF" stroke="none"/>',
    arrow: [
      '<path d="M115 -3 Q137 -3 145 -16" stroke="#FF5A2B" stroke-width="8" stroke-linecap="butt"/>',
      '<path d="M154.5 -10.1 L152.1 -27.6 L135.5 -21.9 Z" fill="#FF5A2B" stroke="none"/>',
    ].join('\n    '),
  },
}

const glyphsAsOf = (round) => {
  if (round === undefined) return GLYPHS
  const pinned = HISTORY[round]
  if (!pinned) throw new Error(`no pinned glyphs for "${round}" (have: ${Object.keys(HISTORY).join(', ')})`)
  return new Map([...GLYPHS, ...Object.entries(pinned)])
}

// The i's tittle as of a given round, for the scripts that measure against it
// rather than just drawing it.
export const dotAsOf = (round) => {
  const found = glyphsAsOf(round).get('tittle').match(/cx="([\d.]+)"[^>]*cy="([\d.]+)"[^>]*r="([\d.]+)"/)
  return { cx: Number(found[1]), cy: Number(found[2]), r: Number(found[3]) }
}

// Markup for a named set of glyphs, shifted horizontally and painted. Unknown
// names throw: asking for a letter the master does not draw is a bug, not an
// empty string.
//
// `asOf` pins the letters to a past round — see HISTORY above. Scripts that
// document a decision pass it; the build does not.
export const spell = (names, { shift = 0, fg = '#FFFFFF', accent = '#FF5A2B', asOf } = {}) => {
  const available = glyphsAsOf(asOf)
  const body = names
    .map((name) => {
      const found = available.get(name)
      if (!found) throw new Error(`wordmark has no glyph "${name}" (has: ${[...available.keys()].join(', ')})`)
      return found
    })
    .join('\n    ')
  const move = shift === 0 ? '' : ` transform="translate(${shift.toFixed(2)} 0)"`
  // The paint carries the monoline's own #FFFFFF, so it is recoloured with the
  // glyphs rather than after them.
  return `<g ${recolour(WORD_PAINT, fg, accent)}${move}>
    ${recolour(body, fg, accent)}
  </g>`
}
