# Rust v3 implementation

The Rust server replaces the Node v2 runtime. The browser client and PHP publisher
work without modification. Linux amd64 and arm64 images are built and tested on
native runners; publication transfers those exact images into a combined index.

The implementation forbids unsafe Rust in application code. Typed resource/version
models validate protocol input. A single tracker task owns cache and subscription
state and orders operations; it does not hold a shared state mutex across I/O. Disk
operations run outside the event loop. This intentionally favors simple ordering
and predictable errors over parallel filesystem publications. Independent health
checks, WebSocket I/O and heartbeat timers remain responsive during storage work.

Build and test from the repository root. Run Rust commands inside `rust/` so
Rustup selects the toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml):

```sh
(cd rust && cargo fmt --check)
(cd rust && cargo clippy --locked --all-targets -- -D warnings)
(cd rust && cargo test --locked)
(cd rust && cargo build --locked)
npm ci --engine-strict
npm test
# Check out v2.0.0 (3bdfc00524e977d502ca6614642d6a302d51e00f) into node-reference/
# and run npm ci --prefix node-reference before comparing both implementations.
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
docker build -t recency-tracker:rust-candidate .
node scripts/smoke-container.js recency-tracker:rust-candidate
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
accept literal IP addresses or resolvable hostnames. Bind publisher port 9998 privately; expose subscriber
port 9999 through a TLS/WebSocket reverse proxy. `/ping` and `/recency-tracker.js`
keep their existing paths and bodies. Explicit UID/GID selection and a read-only
root filesystem are supported. The storage directory must already exist.

## Subscriber trust and browser origins

The subscriber endpoint has no built-in user authentication or resource authorization.
Any client able to reach it can request any resource and receive its notification
`data`. Resource names are not credentials. Keep sensitive payloads behind an
appropriate authenticated proxy/network boundary or publish only data suitable for
all clients that can reach the endpoint. A private publisher port and publishing
secret do not protect subscriptions.

An optional exact Origin allowlist limits which browser sites may open WebSockets:

```json
{
  "subscriber": {
    "ip": "0.0.0.0",
    "port": 9999,
    "allowedOrigins": ["https://app.example.test"]
  }
}
```

Copy the application's actual browser `location.origin`: scheme, hostname and
nondefault port, with no path or trailing slash. Wildcards, credentials and paths
are rejected. With a configured list, missing, `null`, repeated and unlisted Origin
headers receive HTTP 403 before tracker admission. An empty list denies all
WebSocket upgrades. Omitting the setting preserves unrestricted v2/v3 behavior;
explicit JSON `null` is rejected. Health checks and PHP publishing are unaffected.

For different deployment environments, `RECENCY_TRACKER_SUBSCRIBER_ALLOWED_ORIGINS`
overrides the file policy with a JSON array, for example
`["http://localhost:20080"]`. An empty/malformed override prevents startup; `[]`
denies all upgrades. At most 256 origins are supported. Non-browser WebSocket probes
must send an allowed Origin when the policy is enabled.

**Origin checking is not authentication.** Browsers enforce the header, but a
non-browser client can forge it. This policy helps prevent cross-site browser access;
it does not make publicly reachable notification data private. Review the proxy's
access controls separately before putting private application data on this channel.

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
subscriptions, 65,536 queued messages and 64 MiB queued bytes per connection. Each listener also bounds accepted TCP connections to `connections + 32` (handshake
headroom), including incomplete HTTP
requests and upgraded WebSockets. Exhausted TCP admission closes new connections;
tracker admission rejects upgrades with HTTP 503. HTTP headers must arrive within
ten seconds. Subscription or outbound queue
limits close the socket with 1013; the unchanged browser client reconnects and
catches up from its last seen version. A page exceeding the subscription limit
will repeatedly reconnect until its subscription count or the configured limit
changes; size this limit for your application before upgrading. Queue bytes are charged per client even
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
  to 1 MiB; nonregular files and, on Unix, symlinks are rejected. Only the documented browser asset is served; Express-specific cache,
  range, ETag, redirect and error-page behavior is not reproduced.
- Storage failures return HTTP 500 without announcing a successful publication.
  Subscriber storage errors close with 1011. Requests have a ten-second deadline;
  accepted tracker work continues if its HTTP client disconnects or times out.
  The ten-second header deadline also expires idle HTTP keep-alive connections;
  upgraded WebSockets retain the separate heartbeat policy.
  Subscription operations also have a ten-second deadline and remain cancellable
  by shutdown.

Filesystem mode holds an exclusive advisory lock in `.recency-tracker.lock`; use a
local filesystem that supports locking, atomic rename and sync. Atomic same-directory
temporary-file replacement avoids exposing partial JSON. The file and directory are
synced before success is acknowledged. New record modes respect the process umask;
replacements preserve explicit file permissions. Rename creates a new inode owned
by the selected process UID; ACLs, ownership and hard links are not preserved as
metadata. Temporary files left by a killed process are ignored. A failed response
can still mean a rename completed before a later directory-sync failure: consumers
must not infer that failure proves no state change. On a write error the cache
entry is invalidated and affected subscribers are disconnected to reload persisted
state on reconnect.

SIGTERM/SIGINT stop listeners, close live sockets, let active HTTP work finish, and
then drain ordered storage operations. Shutdown allows fifteen seconds for graceful work and then ends runtime cleanup
without waiting for stalled blocking filesystem operations. An
abrupt process kill or unavailable filesystem cannot guarantee acknowledgment or
completion, but committed records are not rewritten in place.

Before any deployment, retain the exact v2 image and browser asset, test upgrade and
rollback with a copied state directory, then stop the old writer before switching.
Never run Node and Rust against the same writable directory; Node does not honor the
new lock. No data conversion is required for compatible records. Keep the unchanged
browser/PHP integration in place while testing the replacement.

Cargo dependencies, Rust/musl, build images, Actions and audit tools still require
maintenance. Commit `Cargo.lock`, use locked builds, run a current `cargo audit`,
track builder/toolchain updates through Renovate, and rebuild static binaries for
security fixes. A scratch image having few OS packages does not make its embedded
libraries maintenance-free. The validation workflow has no registry credentials or publishing job.
The release workflow builds and tests both architectures natively, transfers the
exact tested images as artifacts, and only then publishes architecture-specific
images and their combined index. The index must contain exactly linux/amd64 and
linux/arm64 before stable tags become visible. No QEMU build is required.

For a release, manually set the Cargo package and matching lockfile version, then
push the matching stable `v3.X.Y` tag. Release tags publish the exact version, minor,
`3`, and `latest`, with optional `-amd64` / `-arm64` version tags. Publication is
serialized, checks the tested child digests and verifies the final aliases. An
older stable tag cannot move aliases behind a newer stable Git tag. Manual runs of
`Rust multi-arch release` perform validation without registry writes. Master pushes
publish `edge` after native validation. No tag/release is created automatically.
