# Shared by smoke / conformance / recorded-oracle scripts.
#
# When no port is given on the command line, bind a temporary loopback socket
# to port 0 and read the kernel-assigned port. Agents share one host; a fixed
# default (4589) lets two parallel fidelity-gate runs talk to each other's
# server. An explicit port argument keeps the old behaviour for local debugging.
#
# Requires `node` on PATH (every caller already uses it).
#
# Usage:
#   source "$repo_root/scripts/lib/pick-free-port.sh"
#   port="$(fireup_resolve_port "${1:-}")"

fireup_pick_free_port() {
  node -e "
    const net = require('net')
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address()
      server.close(() => process.stdout.write(String(port)))
    })
    server.on('error', (error) => {
      console.error(error.message || error)
      process.exit(1)
    })
  "
}

# Resolve `$1` when the caller forwarded `"\${1:-}"`. Empty → free port.
fireup_resolve_port() {
  local requested="${1:-}"
  if [[ -n "$requested" ]]; then
    printf '%s' "$requested"
    return 0
  fi
  fireup_pick_free_port
}

# After starting fireup in the background: if it died before accepting, dump
# the log and name a bind collision when the log says so.
fireup_fail_if_server_died() {
  local pid="$1"
  local log="$2"
  local port="$3"
  if kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  echo "fireup local exited before accepting connections on 127.0.0.1:$port:" >&2
  if grep -Eiq 'address already in use|os error 48|os error 98|EADDRINUSE' "$log" 2>/dev/null; then
    echo "port $port is already taken — pass a free port or omit the argument to pick one" >&2
  fi
  cat "$log" >&2
  exit 1
}
