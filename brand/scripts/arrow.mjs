#!/usr/bin/env node
// Round seven. The note on round 6: "Almost good. Add accent curve arrow above
// the u or up. U can be capitalized."
//
//   node scripts/arrow.mjs   ->  brand/arrow/*.png
//
// The mark is frozen — it was signed off in round 5 and round 6 only moved it
// into the word. Nothing here redraws it. What this round decides is a new
// element in the *word*: an ember curved arrow set over the u, and whether that
// u becomes a capital U.
//
// Three things get rendered rather than argued about:
//
//   7a  what the arrow sits over — the u, or "up", or the u with no air between
//       the two. The last one matters more than it looks: the mark's own written
//       rule is "nothing floats", and an ember object hanging in space over a
//       letter is the first thing in this identity that would.
//   7b  capital U or lowercase u. The note permits the capital; it does not ask
//       for it, so it has to earn the place.
//   7c  how big the arrow is. The band above the lowercase is nearly empty — the
//       word's own ascender line is at y=4 and the lockup's envelope runs up to
//       the fire's tip at y=-38 — so "as big as it fits" and "as small as a
//       diacritic" are 40 units apart, and the 96px minimum is the only judge.
//
// The arrow's own clearance is not a taste decision. The word already carries a
// floating accent over a letter: the i's tittle sits 3 units above its stem.
// Anything else set over a letter uses the same 3, read off the master as
// ACCENT_CLEARANCE rather than retyped.
//
// Wordmark construction (logo/wordmark.svg, via word.mjs): baseline y=50,
// x-height top y=20, ascender top y=4, descender y=66, stroke 8. The u's stems
// stand on x=120 and x=150, so with the stroke it spans x=116..154; "up" runs
// on to the p's bowl at x=200.
// Mark construction (logo/mark.svg): F foot y=60, cap top y=16, bar weight 12,
// middle arm ends x=28, bar ends x=36, fire tip y=1, fire out to x=56.
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { master, innerOf, recolour, spell as spellNow, W, dotAsOf, accentClearanceAsOf } from './word.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'


// The record of round 7, pinned to the letters that round drew. Round 11
// moved the i's tittle, the capital U and the accent over it; unpinned, every
// sheet below would silently re-render with letters this round never saw. See
// HISTORY in word.mjs.
const AS_OF = 'round-10'
const spell = (names, options = {}) => spellNow(names, { ...options, asOf: AS_OF })
const DOT = dotAsOf(AS_OF)
// Rounds 7 and 8 placed the arrow with this, so it is geometry here, not a
// caption: left reading the live tittle it would move these drawings.
const ACCENT_CLEARANCE = accentClearanceAsOf(AS_OF)

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

const INK = tokens.color.ink
const PAPER = tokens.color.paper
const EMBER = tokens.color.ember
const EMBER_DEEP = tokens.color['ember-deep']
const MUTED = tokens.color['paper-muted']

// --- the lockup, exactly as round 6 settled it --------------------------------
// Read out of mark.svg and wordmark.svg, not re-derived: the F stands on the
// word's baseline at 1.5x with its stem on x=0, and "ireup" starts one letter
// gap after the middle arm.
const M = { foot: 60, capTop: 16, bar: 12, armEnd: 28, barEnd: 36, fireTip: 1, fireEnd: 56, stemL: 6 }
const MARK_SCALE = 1.5

const markX = (u) => (u - M.stemL) * MARK_SCALE
const markY = (v) => W.baseline + (v - M.foot) * MARK_SCALE
const WORD_X = markX(M.armEnd) + W.letterGap
const SHIFT = WORD_X - W.ireupL

const MARK_PLACED = (fg, accent) =>
  `<g transform="translate(${(-M.stemL * MARK_SCALE).toFixed(2)} ${(W.baseline - M.foot * MARK_SCALE).toFixed(2)}) scale(${MARK_SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

// Where the letters land once the word is shifted under the F.
const at = (x) => x + SHIFT
const U = { left: at(120 - W.stroke / 2), right: at(150 + W.stroke / 2), top: W.xTop - W.stroke / 2 }
const P = { right: at(196 + W.stroke / 2) }
const CAP_TOP = W.ascTop - W.stroke / 2 // a capital's top edge, on the word's ascender line

// --- the two letters this round replaced --------------------------------------
// The master draws a capital U and a capital F now, so the lowercase u and f it
// used to draw live here, in the script that replaced them. Reaching for a
// glyph the master no longer has throws rather than drawing the wrong thing —
// which is how this script found out it needed these.
const LOWER_U = `<path d="M120 ${W.xTop} V35 A15 15 0 0 0 150 35"/>
    <path d="M150 ${W.xTop} V${W.baseline}"/>`
const LOWER_F = `<path d="M12 ${W.baseline} V13 A8 8 0 0 1 20 5"/>
    <path d="M4 ${W.xTop} H23"/>`

// The capital U: drawn on the lowercase u's own stems, so the word's rhythm is
// untouched and only the height changes. Both stems run from the ascender line
// to the bowl and the right one stops there instead of carrying on to the
// baseline. The master does not overshoot its round letters — the e's bowl tops
// out on the x-height line, not above it — so the U does not either.
const CAP_U = `<path d="M120 ${W.ascTop} V35 A15 15 0 0 0 150 35 V${W.ascTop}"/>`

// --- the arrow ----------------------------------------------------------------
// A stroked arc plus a solid head: the arc is monoline like the letters, the
// head is solid like the fire.
//
// The weight is not a variable. The wordmark is a monoline — every letter is
// the same 8 units, and its one existing accent, the i's tittle, is an 8-unit
// circle. An accent drawn at any other weight is a second voice. So the shaft
// is W.stroke, in wordmark units, and it does *not* scale when the arrow gets
// longer. The first cut of this script built the arrow in its own units and
// placed it with a scale factor, which quietly made 7c a sheet about weight as
// much as about size: the "over up" option came out at a 7.7-unit shaft and
// the "over the u" one at 3.5.
const ARROW_W = W.stroke

// Head width 2.8x the shaft, length 1.7x. Below about 2.5x the head stops
// reading as a head and the whole thing reads as a comma.
const HEAD = { half: ARROW_W * 1.4, long: ARROW_W * 1.7 }

// The curve bows the way a growth curve does — flat out of the tail, turning up
// at the head — because the control point sits level with the tail, 72% of the
// way along. Bowing it the other way reads as arriving and flattening out,
// which is the opposite of what the word says.
const BOW = 0.72

// Built at the origin: tail at (0,0), head pointing up and to the right, the
// same way the fire blows. `run` and `rise` are the shaft's chord, in wordmark
// units; the head sticks out past both.
const arrowBody = ({ run, rise }) => {
  const tail = { x: 0, y: 0 }
  const ctrl = { x: run * BOW, y: 0 }
  const neck = { x: run, y: -rise }
  const span = { x: neck.x - ctrl.x, y: neck.y - ctrl.y }
  const length = Math.hypot(span.x, span.y)
  const u = { x: span.x / length, y: span.y / length }
  const n = { x: -u.y, y: u.x }
  const point = (p) => `${p.x.toFixed(2)} ${p.y.toFixed(2)}`
  const tip = { x: neck.x + u.x * HEAD.long, y: neck.y + u.y * HEAD.long }
  const left = { x: neck.x + n.x * HEAD.half, y: neck.y + n.y * HEAD.half }
  const right = { x: neck.x - n.x * HEAD.half, y: neck.y - n.y * HEAD.half }
  return (
    `<path d="M${point(tail)} Q${point(ctrl)} ${point(neck)}" fill="none" stroke="#FF5A2B" ` +
    `stroke-width="${ARROW_W}" stroke-linecap="butt"/>` +
    `<path d="M${point(left)} L${point(tip)} L${point(right)} Z" fill="#FF5A2B" stroke="none"/>`
  )
}

// An arrow of this shape, centred on `over` and sitting with its lowest ink on
// `bottom`. The box is measured off the raster rather than counted: the widest
// point is the tip or a head corner depending on the rise, and the lowest is
// the tail's bottom edge — neither is worth working out by hand.
const arrowOver = ({ over, bottom, run, rise }) => {
  const body = arrowBody({ run, rise })
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40 -80 200 140">${body}</svg>`)
  const tx = over - (box.x + box.w / 2)
  const ty = bottom - (box.y + box.h)
  return {
    run,
    rise,
    width: box.w,
    height: box.h,
    bottom,
    top: ty + box.y,
    left: tx + box.x,
    right: tx + box.x + box.w,
    draw: (accent, dx = 0) =>
      `<g transform="translate(${(tx + dx).toFixed(2)} ${ty.toFixed(2)})">${body.replaceAll('#FF5A2B', accent)}</g>`,
  }
}

// --- compositions -------------------------------------------------------------
// An option is the frozen mark, a spelling of the word, and optionally an arrow.
// The U is drawn here rather than named, because both cases of it have to be on
// the same sheet and the master only carries the one it ships.
const PAINT = 'fill="none" stroke="#FFFFFF" stroke-width="8" stroke-linecap="butt" stroke-linejoin="round"'
const drawn = (markup, fg, accent, shift = 0) =>
  `<g ${recolour(PAINT, fg, accent)} transform="translate(${shift.toFixed(2)} 0)">${markup}</g>`

const compose = ({ cap = true, arrow = null }) => (fg, accent) => {
  const body = [
    MARK_PLACED(fg, accent),
    spell(['i', 'r', 'e', 'p', 'tittle'], { shift: SHIFT, fg, accent }),
    drawn(cap ? CAP_U : LOWER_U, fg, accent, SHIFT),
  ]
  if (arrow) body.push(arrow.draw(accent))
  return body.join('\n  ')
}

// The ceiling the arrow lives under: the top of the letter it sits over, less
// the word's own accent clearance. For a capital U that ceiling is 16 units
// higher, which is the whole of 7b in one line.
const ceilingOf = (cap) => (cap ? CAP_TOP : U.top) - ACCENT_CLEARANCE

// Reference shape: the shaft's chord runs stem to stem across the u, and rises
// by 43% of that — the shallowest rise where the head still points up rather
// than along. The whole arrow comes out wider than its chord, because the head
// overhangs it.
const RUN_U = 150 - 120
const RISE = 0.43
const MID_U = (U.left + U.right) / 2
const MID_UP = (U.left + P.right) / 2

// --- 7a: what it sits over ----------------------------------------------------
const A_OVER_U = arrowOver({ over: MID_U, bottom: ceilingOf(false), run: RUN_U, rise: RUN_U * RISE })
const A_OVER_UP = arrowOver({
  over: MID_UP,
  bottom: ceilingOf(false),
  run: P.right - U.left - W.stroke,
  rise: (P.right - U.left - W.stroke) * RISE,
})
const A_TOUCHING = arrowOver({ over: MID_U, bottom: U.top, run: RUN_U, rise: RUN_U * RISE })

const PLACEMENTS = [
  {
    id: 'over-u',
    label: 'A · Over the u — accent width, accent clearance',
    body: compose({ arrow: A_OVER_U }),
    arrow: A_OVER_U,
    note:
      "As wide as the u and 3 units above it —\n" +
      "the same air the i's tittle keeps over its\n" +
      'own stem, so the arrow is spaced like the\n' +
      'accent the word already has.',
  },
  {
    id: 'over-up',
    label: 'B · Over "up" — spanning both letters',
    body: compose({ arrow: A_OVER_UP }),
    arrow: A_OVER_UP,
    note:
      'The arc runs from the u to the end of the\n' +
      "p's bowl. Twice the run for the same rise,\n" +
      'so the curve flattens and the head has to\n' +
      'grow with it.',
  },
  {
    id: 'touching',
    label: 'C · Touching — tail on the u, no air',
    body: compose({ arrow: A_TOUCHING }),
    arrow: A_TOUCHING,
    note:
      "Clearance 0: the tail sits on the u's top\n" +
      'edge, so the arrow grows out of the letter\n' +
      'rather than floating over it. That is the\n' +
      "mark's own rule — nothing floats.",
  },
  {
    id: 'none',
    label: 'D · Round 6 — lowercase u, no arrow (today)',
    body: compose({ cap: false }),
    arrow: null,
    note: 'What is on the PR now, for comparison.',
  },
]

// --- 7b: capital U or lowercase u ---------------------------------------------
const RUN_UP = P.right - U.left - W.stroke
const B_LOWER = arrowOver({ over: MID_U, bottom: ceilingOf(false), run: RUN_U, rise: RUN_U * RISE })
const B_CAP = arrowOver({ over: MID_U, bottom: ceilingOf(true), run: RUN_U, rise: RUN_U * RISE })
const B_CAP_TIGHT = arrowOver({ over: MID_U, bottom: CAP_TOP, run: RUN_U, rise: RUN_U * RISE })
const B_CAP_UP = arrowOver({ over: MID_UP, bottom: ceilingOf(true), run: RUN_UP, rise: RUN_UP * RISE })

const CASES = [
  {
    id: 'lower-u',
    label: 'A · lowercase u — arrow in the ascender band',
    body: compose({ cap: false, arrow: B_LOWER }),
    arrow: B_LOWER,
    note:
      'The arrow occupies the empty band between\n' +
      'x-height and the ascender line, which is\n' +
      'the only part of the word with room in it.',
  },
  {
    id: 'cap-u',
    label: 'B · capital U — arrow above the ascender',
    body: compose({ cap: true, arrow: B_CAP }),
    arrow: B_CAP,
    cap: true,
    note:
      'FireUp. The U fills the band the arrow\n' +
      'wanted, so the arrow is pushed above the\n' +
      'ascender line and the lockup grows a\n' +
      'second tall thing on the right.',
  },
  {
    id: 'cap-u-touch',
    label: 'C · capital U, arrow on its shoulder',
    body: compose({ cap: true, arrow: B_CAP_TIGHT }),
    arrow: B_CAP_TIGHT,
    cap: true,
    note:
      'The same capital with the clearance taken\n' +
      'out, so the arrow rides on the U instead\n' +
      'of over it.',
  },
  {
    id: 'cap-up',
    label: 'D · capital U, arrow over the whole "Up"',
    body: compose({ cap: true, arrow: B_CAP_UP }),
    arrow: B_CAP_UP,
    cap: true,
    note:
      'Both halves of the note at once: the\n' +
      'capital, and the arc running the length of\n' +
      '"Up" rather than over one letter.',
  },
]

// --- 7c: how big ---------------------------------------------------------------
// The band above the lowercase is empty, so nothing bounds the size except the
// 96px minimum underneath and the lockup's envelope above. Same drawing, same
// 8-unit shaft, three chords: inside the u's stems, stem to stem, and out to
// the p.
const sizeOption = (run, id, label, note) => {
  const arrow = arrowOver({ over: MID_U, bottom: ceilingOf(false), run, rise: run * RISE })
  return { id, label, body: compose({ arrow }), arrow, note }
}

const SIZES = [
  sizeOption(
    20,
    'size-s',
    'A · Inside the stems — 20-unit chord',
    'Narrower than the letter it sits over.\n' +
      'The head is as long as the shaft, so there\n' +
      'is barely a curve left to read.',
  ),
  sizeOption(RUN_U, 'size-m', "B · Stem to stem — 30-unit chord", 'The letter it belongs to sets the chord.'),
  sizeOption(
    48,
    'size-l',
    'C · Out to the p — 48-unit chord',
    'Reads at 96, and stops being an accent:\n' +
      'a second ember object the size of the\n' +
      'fire, at the other end of the lockup.',
  ),
]

// The one that ships, in the wordmark's own coordinates: the lockup shifts the
// word right by SHIFT and that shift is not part of the drawing.
const SHIP_ARROW = arrowOver({
  over: (120 + 150) / 2,
  bottom: W.ascTop - W.stroke / 2 - ACCENT_CLEARANCE,
  run: RUN_U,
  rise: RUN_U * RISE,
})

// --- 7d: the word on its own --------------------------------------------------
// Capitalising the U has a consequence outside the lockup. site/assets ships a
// wordmark variant — the word with no mark — and that drawing still starts with
// the lowercase monoline f. Leave it and the identity ships two spellings:
// "FireUp" in the lockup, where the mark is the capital F, and "fireUp" in the
// word on its own. So the word needs a capital F of its own.
//
// Its width is the whole question. The lowercase f lives in a narrow slot: its
// crossbar ends at x=23 and the i's stem begins at 30. A capital F wants to be
// a bit over half its cap height across — about 26 of 46 here — and there is
// not 26 units of room without pushing the rest of the word right.
const capF = ({ stem, armEnd }) => {
  const left = stem - W.stroke / 2
  const middle = left + (armEnd - left) * 0.73 // the mark's F runs its middle arm to 73% of its bar
  return `<path d="M${stem} ${W.baseline} V${W.ascTop} H${armEnd}"/>
    <path d="M${stem} 27 H${middle.toFixed(1)}"/>`
}

// The word with no mark beside it: the letters, the capital U and its arrow,
// and whichever initial is being tried.
const wordAlone = ({ initial = null, shift = 0 }) => (fg, accent) =>
  [
    spell(['i', 'r', 'e', 'p', 'tittle'], { shift, fg, accent }),
    drawn(CAP_U, fg, accent, shift),
    SHIP_ARROW.draw(accent, shift),
    drawn(initial === null ? LOWER_F : capF(initial), fg, accent),
  ].join('\n  ')

const INITIALS = [
  {
    id: 'word-lower-f',
    label: 'A · the f left alone — "fireUp"',
    body: wordAlone({ initial: null, shift: 0 }),
    note:
      'What falls out of changing nothing else.\n' +
      'The lockup says FireUp and the word on its\n' +
      'own says fireUp. Two spellings of one name.',
  },
  {
    id: 'word-cap-f-tight',
    label: "B · capital F squeezed into the f's own slot",
    body: wordAlone({ initial: { stem: 12, armEnd: 23 }, shift: 0 }),
    note:
      'Nothing else moves, so the F fits the narrow\n' +
      'slot the f had: 15 units across against a\n' +
      '46-unit cap.',
  },
  {
    id: 'word-cap-f',
    label: 'C · capital F at its own width, set back into the margin',
    body: wordAlone({ initial: { stem: 2, armEnd: 22 }, shift: 0 }),
    note:
      'The F gets the 24 units a capital wants —\n' +
      'and takes them out of the left margin\n' +
      'rather than out of the word, so every other\n' +
      'letter stays exactly where it was.',
  },
]

// --- diagnostics --------------------------------------------------------------
// Printed rather than eyeballed: the clearance over the letter below, whether
// the arrow pushes the lockup's envelope out, and how big the head actually
// comes out at the 96px minimum.
const ROUND6_BOX = contentBox(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -80 420 220">${compose({ cap: false })(PAPER, EMBER)}</svg>`,
)

const framed = (option, fg, accent) => {
  const body = option.body(fg, accent)
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -90 420 230">${body}</svg>`)
  return {
    box,
    body,
    svg: (bg) =>
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.w.toFixed(2)} ${box.h.toFixed(2)}">` +
      (bg
        ? `<rect x="${box.x.toFixed(2)}" y="${box.y.toFixed(2)}" width="${box.w.toFixed(2)}" height="${box.h.toFixed(2)}" fill="${bg}"/>`
        : '') +
      `${body}</svg>`,
  }
}

const ALL = [...PLACEMENTS, ...CASES, ...SIZES, ...INITIALS]
const MIN = 96 // the stated lockup minimum, in px wide

console.log(
  `accent clearance read off the master: ${ACCENT_CLEARANCE} units ` +
    `(tittle bottom ${DOT.cy + DOT.r} to x-height top edge ${W.xTop - W.stroke / 2})`,
)
console.log(`round 6 envelope: ${ROUND6_BOX.w.toFixed(1)} x ${ROUND6_BOX.h.toFixed(1)} units, top at y=${ROUND6_BOX.y.toFixed(1)}\n`)

console.table(
  ALL.map((option) => {
    const { box } = framed(option, PAPER, EMBER)
    const px = MIN / box.w // one wordmark unit, in px, at the 96px minimum
    const letterTop = option.cap ? CAP_TOP : U.top
    return {
      id: option.id,
      aspect: `${(box.w / box.h).toFixed(2)}:1`,
      grewEnvelope: box.h > ROUND6_BOX.h + 0.5 ? `+${(box.h - ROUND6_BOX.h).toFixed(1)} tall` : 'no',
      arrowW: option.arrow ? option.arrow.width.toFixed(1) : '—',
      clearance: option.arrow ? (letterTop - option.arrow.bottom).toFixed(1) : '—',
      'arrow at 96px': option.arrow
        ? `${(option.arrow.width * px).toFixed(1)}x${(option.arrow.height * px).toFixed(1)}px, ` +
          `shaft ${(ARROW_W * px).toFixed(1)}px`
        : '—',
      'x-height at 96px': ((W.baseline - W.xTop) * px).toFixed(1),
    }
  }),
)

// --- the sheets ---------------------------------------------------------------
// A lockup is wide, so each option is a row and the sizes run across it: a doc
// or README (320), the site header (200), a dense UI row (128) and the stated
// 96px minimum.
const FONT = 'DejaVu Sans, sans-serif'
const WIDTHS = [320, 200, 128, 96]
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 330

const text = (x, y, body, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

const sheetOf = ({ options, fg, accent, bg, muted, caption }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  const width = PAD * 2 + WIDTHS.reduce((sum, w) => sum + w + 28, 0) + NOTE_COL
  let y = PAD + 30
  const blocks = options
    .map((option, index) => {
      const row = rows[index]
      const rowH = (WIDTHS[0] * row.box.h) / row.box.w
      const top = y
      let x = PAD
      const tiles = WIDTHS.map((w) => {
        const h = (w * row.box.h) / row.box.w
        const tile = `<svg viewBox="${row.box.x.toFixed(2)} ${row.box.y.toFixed(2)} ${row.box.w.toFixed(2)} ${row.box.h.toFixed(2)}" x="${x}" y="${(top + 22 + (rowH - h) / 2).toFixed(2)}" width="${w}" height="${h.toFixed(2)}">${row.body}</svg>`
        x += w + 28
        return tile
      }).join('')
      const notes = option.note
        .split('\n')
        .map((line, i) => text(x + 6, top + 36 + i * 17, line, { size: 13, fill: muted, weight: 'normal' }))
        .join('')
      y = top + 22 + rowH + ROW_GAP
      return `${text(PAD, top + 12, option.label, { fill: fg })}${tiles}${notes}`
    })
    .join('')

  const height = y + 30
  let labelX = PAD
  const scale = WIDTHS.map((w) => {
    const label = text(labelX, PAD + 10, `${w}px wide`, { size: 12, fill: muted, weight: 'normal' })
    labelX += w + 28
    return label
  }).join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${scale}
  ${text(labelX + 6, PAD + 10, 'actual size, left to right', { size: 12, fill: muted, weight: 'normal' })}
  ${blocks}
  ${text(PAD, height - 12, caption, { size: 14, fill: muted, weight: 'normal' })}
</svg>`
}

// 96px is too small to argue about by looking at it, and a 7-unit shaft at 96px
// is 3px. So: render each option at 96 wide, read the renderer's own pixels
// back, and redraw them as one square per pixel.
const zoomSheet = ({ options, fg, accent, bg, muted, caption, width: px = MIN, scale = 5 }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  let y = PAD + 16
  const blocks = options
    .map((option, index) => {
      const row = rows[index]
      const h = Math.round((px * row.box.h) / row.box.w)
      const top = y
      y = top + 24 + h * scale + 30
      return `${text(PAD, top + 12, option.label, { fill: fg })}
      <g transform="translate(${PAD} ${top + 24})">${pixelMap(row.svg(bg), px, scale)}</g>`
    })
    .join('')
  const width = PAD * 2 + px * scale + 40
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${y + 24}" width="${width}" height="${y + 24}">
  <rect width="${width}" height="${y + 24}" fill="${bg}"/>
  ${blocks}
  ${text(PAD, y + 8, caption, { size: 13, fill: muted, weight: 'normal' })}
</svg>`
}

const out = join(root, 'arrow')
mkdirSync(out, { recursive: true })

const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782' }

const SHEETS = [
  {
    file: 'place-dark',
    options: PLACEMENTS,
    ...DARK,
    caption: `Round 7a — what the arrow sits over   ·   on ink, ember ${EMBER}   ·   the mark is unchanged in all four`,
  },
  {
    file: 'place-light',
    options: PLACEMENTS,
    ...LIGHT,
    caption: `Round 7a — what the arrow sits over   ·   on paper, ember-deep ${EMBER_DEEP}   ·   the mark is unchanged in all four`,
  },
  {
    file: 'case-dark',
    options: CASES,
    ...DARK,
    caption: 'Round 7b — capital U or lowercase u   ·   on ink   ·   the capital is drawn on the lowercase u’s own stems',
  },
  {
    file: 'case-light',
    options: CASES,
    ...LIGHT,
    caption: 'Round 7b — capital U or lowercase u   ·   on paper   ·   the capital is drawn on the lowercase u’s own stems',
  },
  {
    file: 'size-dark',
    options: SIZES,
    ...DARK,
    caption: 'Round 7c — how big the arrow is   ·   on ink   ·   same drawing, one scale factor apart',
  },
  {
    file: 'size-light',
    options: SIZES,
    ...LIGHT,
    caption: 'Round 7c — how big the arrow is   ·   on paper   ·   same drawing, one scale factor apart',
  },
  {
    file: 'initial-dark',
    options: INITIALS,
    ...DARK,
    caption: 'Round 7d — the word on its own, with no mark to be its F   ·   on ink   ·   the lockup is unaffected by all three',
  },
  {
    file: 'initial-light',
    options: INITIALS,
    ...LIGHT,
    caption: 'Round 7d — the word on its own, with no mark to be its F   ·   on paper   ·   the lockup is unaffected by all three',
  },
]

for (const config of SHEETS) {
  const source = sheetOf(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

for (const [file, options, caption] of [
  ['zoom-place', PLACEMENTS, 'Round 7a at 96px wide, magnified 5x — real pixels, no resampling'],
  ['zoom-case', CASES, 'Round 7b at 96px wide, magnified 5x — real pixels, no resampling'],
  ['zoom-size', SIZES, 'Round 7c at 96px wide, magnified 5x — real pixels, no resampling'],
]) {
  const source = zoomSheet({ options, ...DARK, caption })
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const path = join(out, `${file}.png`)
  writeFileSync(path, renderPng(source, width))
  console.log(path)
}

// --- what goes into the master -------------------------------------------------
// 7b-B is the one that ships, so this prints its geometry in the wordmark's own
// coordinates — the lockup shifts the word right by 11 and that shift is not
// part of the drawing. The numbers go into logo/wordmark.svg by hand, which is
// the only way the master stays a drawing someone can read and correct; this
// prints them so they are transcribed rather than worked out twice.
console.log(
  `\n7b-B — chord ${RUN_U}x${(RUN_U * RISE).toFixed(2)}, shaft ${ARROW_W}, box ` +
    `${SHIP_ARROW.width.toFixed(1)}x${SHIP_ARROW.height.toFixed(1)} on x=${SHIP_ARROW.left.toFixed(1)}..${SHIP_ARROW.right.toFixed(1)}, ` +
    `y=${SHIP_ARROW.top.toFixed(1)}..${SHIP_ARROW.bottom.toFixed(1)}, in wordmark coordinates:`,
)
console.log(SHIP_ARROW.draw('#FF5A2B').replaceAll('/><', '/>\n  '))

// Each option on its own at the two widths that decide it: 96, the stated
// minimum, and 960, where a head that does not quite point shows up.
for (const option of ALL) {
  const row = framed(option, PAPER, EMBER)
  for (const w of [MIN, 960]) {
    const file = join(out, `${option.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}
