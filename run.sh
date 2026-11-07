#!/usr/bin/env bash
# Start a complete local Rust development stack without altering sibling projects.
set -euo pipefail
cd "$(dirname "$0")"
if [[ "${1:-}" == "--stop" ]]; then
  if [[ -f .local/server.pid ]]; then
    task_server_pid="$(cat .local/server.pid)"
    if [[ "$task_server_pid" =~ ^[0-9]+$ ]]; then
      task_server_command="$(ps -p "$task_server_pid" -o command= 2>/dev/null || true)"
      if [[ "$task_server_command" == *"target/debug/click-server serve" ]]; then
        kill -TERM "$task_server_pid"
      fi
    fi
    rm -f .local/server.pid
  fi
  docker compose --env-file .env -f deploy/compose.dev.yml stop
  echo 'Managed server and databases stopped; volumes retained. Stop any separately launched foreground server with Ctrl+C.'
  exit 0
fi
command -v cargo >/dev/null || { echo 'Install Rust stable: https://rustup.rs'; exit 1; }
command -v docker >/dev/null || { echo 'Install Docker with Compose before running Vuuut Click.'; exit 1; }
if ! docker info >/dev/null 2>&1; then
  if [[ "$(uname -s)" == "Darwin" ]]; then
    echo 'Starting Docker Desktop…'
    open -a Docker
    for ((i=0;i<60;i++)); do docker info >/dev/null 2>&1 && break; sleep 1; done
  fi
  docker info >/dev/null 2>&1 || { echo 'Start your Docker daemon and run ./run.sh again.'; exit 1; }
fi
if [[ ! -f .env ]]; then
  command -v openssl >/dev/null || { echo 'OpenSSL is needed to generate local secrets.'; exit 1; }
  task_pg_password="$(openssl rand -hex 24)"
  task_ch_password="$(openssl rand -hex 24)"
  (umask 077; cat > .env <<CONFIG
# Local development credentials. Never commit this file.
APP_ENV=development
BIND=127.0.0.1:8082
PUBLIC_URL=http://localhost:8082
ALLOW_LOCALHOST=true
POSTGRES_PASSWORD=$task_pg_password
DATABASE_URL=postgres://click:$task_pg_password@127.0.0.1:55432/click
CLICKHOUSE_URL=http://127.0.0.1:58123
CLICKHOUSE_USER=click
CLICKHOUSE_PASSWORD=$task_ch_password
CLICKHOUSE_DATABASE=click
RUST_LOG=click_server=info,tower_http=info
CONFIG
  )
  echo 'Created private local configuration in .env.'
fi
set -a
# .env is a trusted, local shell configuration owned by the developer.
source .env
set +a
if [[ "${APP_ENV:-}" != "development" ]]; then echo 'run.sh starts development only. Use the production deployment guide for production.'; exit 1; fi
docker compose --progress plain --env-file .env -f deploy/compose.dev.yml up -d --quiet-pull --wait
if [[ -z "${GEOIP_PATH:-}" ]]; then
  echo 'GeoLite2 is optional locally: country will show Unknown until GEOIP_PATH points to your licensed Country MMDB.'
fi
echo "Open Vuuut Click at ${PUBLIC_URL}/"
echo "User sign in: ${PUBLIC_URL}/signin | Owner sign in: ${PUBLIC_URL}/owner/signin"
echo 'Create an account, add a website, and follow the setup instructions. Ctrl+C stops Rust; ./run.sh --stop stops databases.'
cargo build --locked -p click-server
# Replace only a server started by this project, retaining both database volumes.
if [[ -f .local/server.pid ]]; then
  task_previous_pid="$(cat .local/server.pid)"
  if [[ "$task_previous_pid" =~ ^[0-9]+$ ]]; then
    task_previous_command="$(ps -p "$task_previous_pid" -o command= 2>/dev/null || true)"
    if [[ "$task_previous_command" == *"target/debug/click-server serve" ]]; then
      kill -TERM "$task_previous_pid"
      for ((i=0;i<100;i++)); do
        kill -0 "$task_previous_pid" 2>/dev/null || break
        sleep 0.1
      done
    fi
  fi
fi
mkdir -p .local
chmod 700 .local
(umask 077; echo "$$" > .local/server.pid)
exec ./target/debug/click-server serve
