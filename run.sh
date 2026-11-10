#!/usr/bin/env bash
# Native local development; Docker is used only by the explicit legacy migration.
set -euo pipefail
cd "$(dirname "$0")"
command -v python3 >/dev/null || { echo 'Install Python 3.11+ to manage local services.' >&2; exit 1; }
task_command=${1:-start}
if [[ "$task_command" == '--stop' ]]; then task_command=stop; shift; set -- "$task_command" "$@"; fi
if [[ "$task_command" == help || "$task_command" == --help || "$task_command" == -h ]]; then
  cat <<'HELP'
Vuuut Click local commands (native databases; no Docker needed):
  ./run.sh [start]           Start app + project-owned PostgreSQL/ClickHouse
  ./run.sh stop              Stop them gracefully, retaining all data
  ./run.sh restart           Restart app + managed databases
  ./run.sh status            Show service state and local URL
  ./run.sh setup             Install native database prerequisites on macOS
  ./run.sh doctor            Check binaries, authentication and database readiness
  ./run.sh build | rebuild   Build/install Rust outside its build cache
  ./run.sh test              Run Rust workspace tests
  ./run.sh log [app|postgres|clickhouse] [-f]
  ./run.sh db-start          Start just the managed databases
  ./run.sh db-stop           Stop app, then managed databases
  ./run.sh migrate-docker    One-time, verified import of existing Docker data
  ./run.sh link-siblings     Point tests/native-bridge at the vuuut / vuuut-pro checkouts
  ./run.sh native-test [...]  Run the optional three-repository bridge suite

Default mode is native. LOCAL_DB_MODE=external uses existing databases and
never starts/stops them. Configuration: private .env (VUUUT_DIR / VUUUT_PRO_DIR locate the sibling
checkouts; default is next to this directory). Data: .local/native/.
Docker volumes are never deleted. See docs/NATIVE_LOCAL.md.
HELP
  exit 0
fi
case "$task_command" in
  start|restart|setup|migrate-docker) python3 scripts/local.py init-env ;;
esac
if [[ -f .env ]]; then
  set -a
  # Trusted, operator-owned local shell configuration; never commit credentials.
  source .env
  set +a
fi
if [[ $# -eq 0 ]]; then set -- start; fi
exec python3 scripts/local.py "$@"
