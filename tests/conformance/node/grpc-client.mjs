/**
 * Raw gRPC clients built from the *vendored public protos*, for the RPCs and
 * request fields no official SDK will emit.
 *
 * Why this exists: the fidelity matrix makes claims per RPC, but a client
 * library only reaches the RPCs it chooses. `@google-cloud/firestore` never
 * calls `GetDocument` (a `.get()` goes to `BatchGetDocuments`), never calls
 * `CreateDocument`/`UpdateDocument`/`DeleteDocument` (they all go through
 * `Commit`), and offers no way at all to send `PartitionQuery`,
 * `ExecutePipeline`, a streaming `Write`, a `read_time` consistency selector,
 * or a `find_nearest` clause. Those matrix rows would be untested if the suite
 * only drove SDKs, so they are driven here instead — still against the
 * published contract, just at the wire level.
 *
 * `keepCase: true` keeps proto field names as written in the `.proto` files, so
 * a request literal below can be read against the proto without translating
 * snake_case to camelCase in your head.
 */

import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const grpc = require('@grpc/grpc-js')
const protoLoader = require('@grpc/proto-loader')

const here = dirname(fileURLToPath(import.meta.url))
export const PROTO_DIR = resolve(here, '../../../proto')

const LOADER_OPTIONS = {
  keepCase: true,
  longs: String,
  enums: String,
  defaults: false,
  oneofs: true,
  includeDirs: [PROTO_DIR],
}

const loadPackage = (protoFile) =>
  grpc.loadPackageDefinition(protoLoader.loadSync(protoFile, LOADER_OPTIONS))

/** gRPC status codes, from the public gRPC spec. */
export const GRPC = {
  OK: 0,
  INVALID_ARGUMENT: 3,
  NOT_FOUND: 5,
  ALREADY_EXISTS: 6,
  ABORTED: 10,
  UNIMPLEMENTED: 12,
}

export const connect = (host) => {
  const firestorePkg = loadPackage('google/firestore/v1/firestore.proto')
  const datastorePkg = loadPackage('google/datastore/v1/datastore.proto')
  const credentials = grpc.credentials.createInsecure()

  return {
    firestore: new firestorePkg.google.firestore.v1.Firestore(host, credentials),
    datastore: new datastorePkg.google.datastore.v1.Datastore(host, credentials),
    close() {
      this.firestore.close()
      this.datastore.close()
    },
  }
}

/** Promisify a unary call, rejecting with the `ServiceError` as-is. */
export const unary = (client, method, request) =>
  new Promise((resolvePromise, reject) => {
    client[method](request, (error, response) => {
      if (error) reject(error)
      else resolvePromise(response)
    })
  })

/**
 * Drain a server-streaming call. Resolves with every message, or rejects with
 * the stream's error — which is where an `UNIMPLEMENTED` arrives for an RPC the
 * server refuses before producing a stream.
 */
export const serverStream = (client, method, request) =>
  new Promise((resolvePromise, reject) => {
    const call = client[method](request)
    const messages = []
    call.on('data', (message) => messages.push(message))
    call.on('error', reject)
    call.on('end', () => resolvePromise(messages))
  })

/**
 * Open a bidirectional stream, send one request, and wait for the outcome.
 * Used for `Listen` and the streaming `Write`, whose refusals land on the
 * stream rather than in a callback.
 */
export const duplexFirstOutcome = (client, method, request) =>
  new Promise((resolvePromise, reject) => {
    const call = client[method]()
    let settled = false
    const settle = (fn, value) => {
      if (settled) return
      settled = true
      fn(value)
    }
    call.on('data', (message) => {
      settle(resolvePromise, message)
      call.end()
    })
    call.on('error', (error) => settle(reject, error))
    call.on('end', () => settle(resolvePromise, null))
    call.write(request)
  })

// ---- small helpers for building request literals ---------------------------

export const documentsRoot = (projectId, databaseId = '(default)') =>
  `projects/${projectId}/databases/${databaseId}/documents`

export const documentName = (projectId, path, databaseId = '(default)') =>
  `${documentsRoot(projectId, databaseId)}/${path}`

export const databaseName = (projectId, databaseId = '(default)') =>
  `projects/${projectId}/databases/${databaseId}`

/** Firestore `Value` literals, in the shape the proto declares. */
export const value = {
  string: (string_value) => ({ string_value }),
  integer: (n) => ({ integer_value: String(n) }),
  double: (double_value) => ({ double_value }),
  boolean: (boolean_value) => ({ boolean_value }),
  null: () => ({ null_value: 'NULL_VALUE' }),
}

/** Read a Firestore `Value` back out of a proto response. */
export const plain = (v) => {
  if (v == null) return undefined
  if ('string_value' in v) return v.string_value
  if ('integer_value' in v) return Number(v.integer_value)
  if ('double_value' in v) return v.double_value
  if ('boolean_value' in v) return v.boolean_value
  if ('null_value' in v) return null
  return v
}

export const plainFields = (document) =>
  Object.fromEntries(Object.entries(document?.fields ?? {}).map(([k, v]) => [k, plain(v)]))
