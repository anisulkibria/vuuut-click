# Native local development

Local Vuuut Click uses native PostgreSQL 17 and ClickHouse on macOS. Normal
startup, shutdown, reports, SDK testing and backup/restore do not need Docker.
The Rust server remains the same application and uses the same database APIs.

## Commands

```sh
./run.sh setup       # Install native prerequisites; existing databases are untouched
./run.sh start       # Default when no command is supplied; detached background service
./run.sh status
./run.sh doctor
./run.sh restart
./run.sh stop        # Stops app, then only this project's native databases; retains data
./run.sh log         # Last 60 app log lines
./run.sh log postgres -f
./run.sh log clickhouse -f
./run.sh db-start
./run.sh db-stop     # Stops app before its databases
./run.sh build       # Builds and atomically installs the Rust runtime
./run.sh rebuild     # Cleans this server package, retaining shared dependency caches
./run.sh test        # Workspace unit/default checks; real-store tests are separate below
./run.sh link-siblings  # Symlink .local/siblings/* from VUUUT_DIR / VUUUT_PRO_DIR in .env
./run.sh native-test -- --include-ignored  # Optional three-repository bridge suite
./run.sh help
```

The old `--stop` spelling is retained as an alias. Startup verifies application
readiness before returning. Repeated starts do not create a second server.
PID ownership checks refuse to signal unrelated listeners. Detached sessions
survive closing the launcher/editor's process group. A control lock prevents
concurrent lifecycle/migration operations. Stop/restart never removes data.

The first setup needs Python 3.11+, Homebrew on macOS, Rust stable for builds,
and internet access to obtain database binaries. PostgreSQL 17 is installed
alongside older Homebrew PostgreSQL versions; their services are not changed.
The project starts its own PostgreSQL cluster, not Homebrew's global service.
ClickHouse's official Intel/Apple Silicon release is pinned with SHA-256 hashes
in `scripts/native-versions.json`. Its compressed release is verified before
execution; a private installation receipt records the executable's digest after
its first-use self-extraction. Subsequent setup checks that receipt.

The current pin matches the original development database engine version to
avoid combining storage migration with an engine-version upgrade. It is a
local development pin, not a production version/security acceptance statement.
Native macOS ClickHouse has a more limited upstream support policy than Linux.
References: https://clickhouse.com/docs/install,
https://clickhouse.com/support/platforms and
https://www.postgresql.org/download/macosx/.

## Data, configuration and resources

- `.env`: private credentials, URLs and local mode; generated once, never replaced
  on normal starts. Editing a PostgreSQL password here does not rotate its
  existing cluster password.
- `.local/native/postgres`: project-owned PostgreSQL data (SCRAM authentication).
- `.local/native/clickhouse`: server configuration, data, access files and logs.
- `.local/bin`: installed application/database executables, outside build caches.
- `.local/server.log`, `.local/server.pid`: application log and process metadata.
- `.local/connections`: provider encryption key and managed licensed GeoIP data.
- `.local/siblings`: symlinks to the Vuuut and Vuuut-pro checkouts, written by
  `./run.sh link-siblings` from `VUUUT_DIR` / `VUUUT_PRO_DIR` in `.env` (default:
  the directories of those names next to this checkout). Only the optional
  `tests/native-bridge` suite reads them; the service itself never does.

Keep these paths through cache cleanup and device migration. None belongs in
Git. Back up both databases and their matching private configuration/key using
the encrypted backup workflow. Default ports stay unchanged: app 8082,
PostgreSQL 55432 and ClickHouse HTTP 58123; native ClickHouse TCP uses 59000.
All managed listeners bind loopback. PostgreSQL has a 64 MiB shared buffer;
ClickHouse has bounded pools/caches and a 1.5 GiB server memory limit, with
256 MiB query limits. Large local benchmark/report workloads may need deliberate
operator tuning; memory limits are not capacity claims.

`LOCAL_PG_BINDIR` selects matching PostgreSQL 17 server/client tools.
`CARGO_TARGET_DIR` selects a build cache; otherwise `.local/build` is used.
Builds omit debug/incremental artifacts and atomically update `.local/bin`.
Compatibility links remain under `target/debug` for existing development tools.

Set `LOCAL_DB_MODE=external` in `.env` to use separately managed local/remote
databases. The launcher authenticates but never starts or stops them. That mode
needs PostgreSQL client tools for its readiness/doctor commands. Use deployment
services for production; this launcher intentionally accepts development only.

## Move an existing Docker installation

The one-time import still needs the original Docker containers running so their
actual data can be read. It is the only normal launcher command that invokes
Docker:

```sh
./run.sh migrate-docker
```

The migration verifies that configured ports belong to this project's exact
containers, stops the application gracefully, holds the source PostgreSQL
maintenance barrier and quiesces ClickHouse merges. It writes a private custom
PostgreSQL dump, exact analytics DDL and streamed Native blocks beneath
`.local/native-migration`. The original private `.env` is retained there too.

The source containers stop without deleting their volumes. Fresh native targets
are initialized with the same credentials, database names and ports. The
restorer loads raw events and hourly aggregate states before creating the
materialized view, preventing duplicate aggregation. Every PostgreSQL table is
checked using row counts and order-independent row fingerprints; analytics raw
logical values and hourly unique-state results are compared before cutover.

Only verified targets activate. Unknown/nonempty native targets are refused.
Failures retain the snapshot and original Docker volumes; before cutover the
script attempts to restart the original containers/application. A successful
receipt makes repeated migration commands harmless. No automatic destructive
rollback over later native writes is provided.

After success, `.env` selects native mode and matching PostgreSQL clients.
Local Docker OTLP endpoints are disabled, with their previous value retained in
the private snapshot. The optional old container-based monitoring stack can be
stopped; configure a separately hosted/native collector through Connections if
wanted. The app's private metrics and monitoring integration remain available.
Docker volumes can be kept as an additional rollback copy; normal native
operation does not access them. Do not start the old containers on the same
ports while native services are running.

## Verify and back up

```sh
./run.sh doctor
# Source only your own trusted .env:
set -a
source .env
set +a
cargo test --workspace -- --include-ignored
npm run test:e2e
```

Ignored integration checks create/drop isolated databases on the native servers;
ordinary `./run.sh test` does not claim those tests passed. Operational tools use
`LOCAL_PG_BINDIR` (or `BACKUP_PG_BINDIR`) for matching clients and need no Docker
container override. Existing production/CI container workflows remain optional
and explicit. See [operations](OPERATIONS.md) for encrypted consistent snapshots,
independent restore drills and production release gates.
