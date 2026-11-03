#!/usr/bin/env bash
#
# Vendor the public googleapis .proto files Fireup needs, pinned to one commit.
#
# Why a script instead of hand-copied files: the ledger
# (docs/licenses/FIREBASE_GCP_LOCAL.md) has to state an exact upstream pin, and
# the next engineer needs a one-command way to re-verify or bump it.
#
# Source: https://github.com/googleapis/googleapis (Apache-2.0).
# These are the *published* API definitions. Nothing here comes from the
# Firebase Emulator Suite or any other proprietary Google binary — see
# .cursor/rules/firebase-gcp-compliance.mdc.
#
# Usage:
#   scripts/vendor-googleapis-protos.sh                 # refresh at the pinned commit
#   GOOGLEAPIS_COMMIT=<sha> scripts/vendor-googleapis-protos.sh   # bump the pin
#
# Imports are resolved transitively, so adding a root below pulls whatever it
# needs. google/protobuf/* is skipped: protoc ships those well-known types.

set -euo pipefail

# Pinned googleapis commit. Bump deliberately and update the license ledger.
GOOGLEAPIS_COMMIT="${GOOGLEAPIS_COMMIT:-57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d}"

# Entry points. Everything they import is fetched automatically.
ROOTS=(
  "google/firestore/v1/firestore.proto"
)

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$repo_root/proto"
base_url="https://raw.githubusercontent.com/googleapis/googleapis/$GOOGLEAPIS_COMMIT"

fetched=()
queue=("${ROOTS[@]}")

already_fetched() {
  local needle="$1"
  local seen
  for seen in ${fetched[@]+"${fetched[@]}"}; do
    [[ "$seen" == "$needle" ]] && return 0
  done
  return 1
}

while [[ ${#queue[@]} -gt 0 ]]; do
  path="${queue[0]}"
  queue=("${queue[@]:1}")

  [[ "$path" == google/protobuf/* ]] && continue
  already_fetched "$path" && continue
  fetched+=("$path")

  out="$dest/$path"
  mkdir -p "$(dirname "$out")"
  echo "fetch $path"
  curl -fsSL "$base_url/$path" -o "$out"

  # Queue this file's imports.
  while read -r import; do
    [[ -n "$import" ]] && queue+=("$import")
  done < <(sed -n 's/^import \(public \)\?"\([^"]*\)";.*/\2/p' "$out")
done

printf '\nPinned googleapis commit: %s\n' "$GOOGLEAPIS_COMMIT"
printf 'Vendored %d proto file(s) under proto/:\n' "${#fetched[@]}"
printf '  %s\n' "${fetched[@]}" | sort
