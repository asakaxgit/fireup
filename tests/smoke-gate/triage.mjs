/**
 * The mechanical first pass of the triage role (docs/local/TRIAGE.md).
 *
 *   node tests/smoke-gate/triage.mjs <results-dir>          markdown report
 *   node tests/smoke-gate/triage.mjs --json <results-dir>   the same, as JSON
 *
 * Reads the `*.json` files the smoke suites write through `results.mjs` and
 * clusters the failed checks by signature; the rule is in `cluster.mjs`. The
 * clusters are a starting point for root-cause groups, not the groups
 * themselves: two signatures can share a cause, and one signature can hide two.
 */

import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import { clusterFailures, failuresFrom } from './cluster.mjs'

const readResults = (dir) =>
  readdirSync(dir)
    .filter((file) => file.endsWith('.json'))
    .sort()
    .map((file) => JSON.parse(readFileSync(join(dir, file), 'utf8')))

const toMarkdown = (results, clusters) => {
  const total = results.reduce((sum, { checks }) => sum + checks.length, 0)
  const failed = results.reduce((sum, { checks }) => sum + checks.filter((c) => !c.ok).length, 0)
  const lines = [
    `# Triage input: ${clusters.length} signature cluster(s)`,
    '',
    `${failed} failed of ${total} checks across ${results.map((r) => r.suite).join(', ')}.`,
    '',
  ]
  for (const { family, signature, checks } of clusters) {
    lines.push(`## ${family} — ${signature}`, '')
    for (const { name, stores, storeSpecific, message } of checks) {
      const where = storeSpecific ? `**only on ${stores.join(', ')}**` : stores.join(', ')
      lines.push(`- ${name} (${where})`, `  - ${message}`)
    }
    lines.push('')
  }
  return lines.join('\n')
}

const [first, second] = process.argv.slice(2)
const asJson = first === '--json'
const dir = asJson ? second : first
if (!dir || dir.startsWith('-')) {
  console.error('usage: triage.mjs [--json] <results-dir>')
  process.exitCode = 2
} else {
  const results = readResults(dir)
  const clusters = clusterFailures(failuresFrom(results))
  console.log(asJson ? JSON.stringify({ clusters }, null, 2) : toMarkdown(results, clusters))
}
