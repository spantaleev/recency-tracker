# Rust v3 candidate

The Rust server is an unreleased replacement candidate. The Node v2 implementation
remains the reference in this branch; its Dockerfile and release workflow are
unchanged. This candidate has its own Dockerfile and validation workflow, with no
image publication. The browser client and PHP publisher work without modification.

The implementation forbids unsafe Rust in application code. Typed resource/version
models validate protocol input. A single tracker task owns cache and subscription
state and orders operations; it does not hold a shared state mutex across I/O. Disk
operations run outside the event loop. This intentionally favors simple ordering
and predictable errors over parallel filesystem publications. Independent health
checks, WebSocket I/O and heartbeat timers remain responsive during storage work.

Build and test from the repository root:

```sh
cargo +1.98.0 fmt --manifest-path rust/Cargo.toml --check
cargo +1.98.0 clippy --manifest-path rust/Cargo.toml --locked --all-targets -- -D warnings
cargo +1.98.0 test --manifest-path rust/Cargo.toml --locked
cargo +1.98.0 build --manifest-path rust/Cargo.toml --locked
npm ci --engine-strict
RECENCY_TEST_IMPLEMENTATION=node node --test rust/contract/suite.cjs
RECENCY_TEST_IMPLEMENTATION=rust node --test rust/contract/suite.cjs
```

Set `RECENCY_TEST_PHP_SOURCE` to the directory containing `Publisher.php` and
`Exception.php` to include the actual PHP-library test. Fixtures use synthetic
resource paths and JSON only. Set `RECENCY_TEST_HEARTBEAT=1` to include the real
30-second heartbeat test, which takes about one minute. CI runs that check and uses
a pinned publisher source revision. The contract suite checks both implementations;
additional Rust checks cover explicit limits and storage failure recovery.

```sh
docker build -f rust/Dockerfile -t recency-tracker:rust-candidate .
node rust/contract/smoke-container.cjs recency-tracker:rust-candidate
```

The pinned Rust Alpine builder produces a static musl executable; the runtime image
contains that binary, user/group entries and the license. Its default numeric user
is `1000:1000` with the existing name `node`. The native build architecture determines
the executable architecture. CI validates amd64 and arm64 on native runners; a
Docker platform flag alone does not convert a host-built executable. The supported
container platform remains Linux. Other operating systems have no publishing or
validation promise.

Use the existing configuration schema and mounts: set `RECENCY_TRACKER_CONFIG`,
mount `/config/config.json` read-only, and provide writable `/data`. An explicitly
empty `RECENCY_TRACKER_PUBLISHER_SECRET` prevents startup. Both listener `ip` fields
must be literal IP addresses. Bind publisher port 9998 privately; expose subscriber
port 9999 through a TLS/WebSocket reverse proxy. `/ping` and `/recency-tracker.js`
keep their existing paths and bodies. Explicit UID/GID selection and a read-only
root filesystem are supported. The storage directory must already exist.

Optional `limits` settings supplement existing configuration:

```json
{
  "limits": {
    "connections": 10000,
    "subscriptions_per_connection": 1024,
    "queued_updates": 256,
    "queued_bytes_per_connection": 1048576
  }
}
```

All values must be positive. Upper bounds are 1,000,000 connections, 65,536
subscriptions, 65,536 queued messages and 64 MiB queued bytes per connection. A
connection limit rejects new upgrades with HTTP 503. Subscription or outbound queue
limits close the socket with 1013; the unchanged browser client reconnects and
catches up from its last seen version. Queue bytes are charged per client even
though fan-out shares immutable message strings. A socket write has a five-second
timeout. Incoming messages remain limited to 64 KiB; the initial read buffer is
4 KiB and may grow for larger messages. There is no WebSocket compression.

The memory cache is bounded by distinct resource count using FIFO eviction.
Updating an existing resource does not count as a new item; `maxItems: 0` disables
caching. Filesystem mode reloads evicted records. In `none` mode, evicted resource
state is lost, as with a bounded memory-only service. Disconnect and unsubscribe
remove empty indexes. The tracker command queue holds at most 1,024 operations;
overload returns HTTP 503 instead of retaining unbounded work.

The public compatibility boundary is:

- Form-encoded POST `/publish` with `secret`, `resource`, `version` and optional
  JSON `data`. Ordinary logical failures retain HTTP 200 and the v2 JSON errors.
- Native WebSockets at `/subscribe`, existing subscribe/unsubscribe/update message
  fields, string update versions, duplicate-subscribe behavior and ignored invalid
  commands. Binary, invalid JSON/UTF-8 and oversized messages retain codes
  1003, 1007 and 1009. Heartbeats remain every 30 seconds.
- The unchanged browser API, five-second reconnect and last-seen resubscription.
- SHA-256 of the UTF-8 resource name plus `.json`, storing `{version,data}` JSON.
  Compatible v2 files can be read directly; new records remain readable by v2.

Versions deliberately retain v2's actual UTF-16 string rule: notify when the target
is longer **or** lexically greater. This is not numeric, semantic-version or strict
length-then-lexical ordering: both `9` to `10` and `10` to `9` can notify. Publishing
an older version still replaces stored state. Changing this behavior needs a separate
versioning/migration decision.

Intentional cleanup changes affect ambiguous input and failure handling:

- Both publisher and subscriber resource names must start with `/`.
- Duplicate secret fields retain `bad secret`; other repeated form fields are
  rejected with `bad request`, rather than coercing arrays into strings.
- Forms retain a 100 KiB body limit and 1,000-parameter limit. Parser-limit error
  bodies are not Express HTML. Compressed request bodies receive HTTP 415.
- JSON parsers enforce their nesting/range/Unicode constraints. Corrupt or invalid
  persisted JSON is an error, not an unknown resource. Persisted reads are bounded
  to 1 MiB. Only the documented browser asset is served; Express-specific cache,
  range, ETag, redirect and error-page behavior is not reproduced.
- Storage failures return HTTP 500 without announcing a successful publication.
  Subscriber storage errors close with 1011. Requests have a ten-second deadline;
  accepted tracker work continues if its HTTP client disconnects or times out.

Filesystem mode holds an exclusive advisory lock in `.recency-tracker.lock`; use a
local filesystem that supports locking, atomic rename and sync. Atomic same-directory
temporary-file replacement avoids exposing partial JSON. The file and directory are
synced before success is acknowledged. New record modes respect the process umask;
replacements preserve explicit file permissions. Rename creates a new inode owned
by the selected process UID; ACLs, ownership and hard links are not preserved as
metadata. Temporary files left by a killed process are ignored. A failed response
can still mean a rename completed before a later directory-sync failure: consumers
must not infer that failure proves no state change.

SIGTERM/SIGINT stop listeners, close live sockets, let active HTTP work finish, and
then drain ordered storage operations. Shutdown has a fifteen-second deadline. An
abrupt process kill or unavailable filesystem cannot guarantee acknowledgment or
completion, but committed records are not rewritten in place.

Before any deployment, retain the exact v2 image and browser asset, test upgrade and
rollback with a copied state directory, then stop the old writer before switching.
Never run Node and Rust against the same writable directory; Node does not honor the
new lock. No data conversion is required for compatible records. Keep the unchanged
browser/PHP integration in place while testing the replacement. Publication and any
runtime upgrade require a separate decision.

Cargo dependencies, Rust/musl, build images, Actions and audit tools still require
maintenance. Commit `Cargo.lock`, use locked builds, run a current `cargo audit`,
track builder/toolchain updates through Renovate, and rebuild static binaries for
security fixes. A scratch image having few OS packages does not make its embedded
libraries maintenance-free. The candidate workflow intentionally has no registry
credentials or publishing job; the deferred release/version automation is unchanged.
