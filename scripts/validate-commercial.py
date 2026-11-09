#!/usr/bin/env python3
"""Isolated, explicitly configured Stripe-test/SMTP validation; never use live keys.

Source the trusted private .env first. Secrets stay in env/private files.
Setup retains its owned test stores/server for interactive hosted Checkout and
inbox verification; cleanup removes only the run named by its private manifest.
"""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / 'target/debug/click-server'
RUNS = ROOT / '.local/commercial-validation'


def require_configuration():
    if os.environ.get('APP_ENV') != 'development':
        raise RuntimeError('Use APP_ENV=development for isolated commercial tests')
    if not os.environ.get('STRIPE_SECRET_KEY', '').startswith('sk_test_'):
        raise RuntimeError('Configure a Stripe sk_test_ key; live keys are refused')
    for key in ['DATABASE_URL', 'CLICKHOUSE_URL', 'CLICKHOUSE_USER',
                'CLICKHOUSE_PASSWORD', 'STRIPE_WEBHOOK_SECRET', 'STRIPE_PRICES_JSON',
                'SMTP_HOST', 'SMTP_FROM', 'COMMERCIAL_TEST_EMAIL']:
        if not os.environ.get(key):
            raise RuntimeError('Missing private configuration: ' + key)
    address = os.environ['COMMERCIAL_TEST_EMAIL']
    if not re.fullmatch(r'[^\s<>@]+@[^\s<>@]+', address):
        raise RuntimeError('COMMERCIAL_TEST_EMAIL must be an inbox you control')


def pg(query, database=None):
    parsed = urllib.parse.urlsplit(os.environ['DATABASE_URL'])
    environment = os.environ.copy()
    parameters=urllib.parse.parse_qs(parsed.query)
    for parameter,variable in [('sslmode','PGSSLMODE'),('sslrootcert','PGSSLROOTCERT'),('sslcert','PGSSLCERT'),('sslkey','PGSSLKEY')]:
        if parameter in parameters:environment[variable]=parameters[parameter][0]
    environment.update(PGHOST=parsed.hostname or '', PGPORT=str(parsed.port or 5432),
                       PGUSER=urllib.parse.unquote(parsed.username or ''),
                       PGPASSWORD=urllib.parse.unquote(parsed.password or ''),
                       PGDATABASE=database or urllib.parse.unquote(parsed.path.lstrip('/')))
    result = subprocess.run(['psql', '-X', '-v', 'ON_ERROR_STOP=1', '-At', '-c', query],
                            env=environment, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError('Commercial test database operation failed; no secrets were printed')
    return result.stdout.strip()


def ch(query):
    headers = {'X-ClickHouse-User': os.environ['CLICKHOUSE_USER'],
               'X-ClickHouse-Key': os.environ['CLICKHOUSE_PASSWORD']}
    request = urllib.request.Request(os.environ['CLICKHOUSE_URL'], data=query.encode(), headers=headers)
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read()


def private_json(path, value):
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def environment_for(run):
    parsed = urllib.parse.urlsplit(os.environ['DATABASE_URL'])
    database_url = urllib.parse.urlunsplit(parsed._replace(path='/' + run['database']))
    environment = os.environ.copy()
    environment.update(DATABASE_URL=database_url, CLICKHOUSE_DATABASE=run['database'],
                       APP_ENV='development', ALLOW_LOCALHOST='true', REQUIRE_GEOIP='false',
                       PUBLIC_URL=run['url'], BIND='127.0.0.1:' + str(run['port']),
                       RUST_LOG='click_server=warn,tower_http=warn')
    environment.pop('GEOIP_PATH', None)
    return environment


def load_run(path):
    path = Path(path).resolve()
    if path.name != 'run.json' or path.parent.parent != RUNS.resolve():
        raise RuntimeError('Use the private run.json produced by setup')
    run = json.loads(path.read_text())
    if not re.fullmatch(r'click_commercial_[0-9a-f]{32}', run.get('database', '')):
        raise RuntimeError('Unexpected test database name; refusing action')
    if run['database'] != path.parent.name:
        raise RuntimeError('Run directory and database do not match')
    return path, run


def api(client, base, path, body=None, form=False):
    headers = {'Origin': base}
    payload = None
    if body is not None:
        headers['Content-Type'] = 'application/x-www-form-urlencoded' if form else 'application/json'
        payload = urllib.parse.urlencode(body).encode() if form else json.dumps(body).encode()
    request = urllib.request.Request(base + path, data=payload, headers=headers)
    try:
        with client.open(request, timeout=30) as response:
            raw = response.read()
            if not raw:return None
            return json.loads(raw) if response.headers.get_content_type()=='application/json' else raw.decode()
    except urllib.error.HTTPError as error:
        raise RuntimeError('Commercial app request failed: ' + path + ' HTTP ' + str(error.code)) from None


def client():
    return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))


def setup(port, owner_only=False):
    require_configuration()
    with socket.socket() as check:
        check.bind(('127.0.0.1', port))
    name = 'click_commercial_' + uuid.uuid4().hex
    RUNS.mkdir(parents=True, exist_ok=True, mode=0o700)
    folder = RUNS / name
    folder.mkdir(mode=0o700)
    run = {'database': name, 'port': port, 'url': 'http://localhost:' + str(port),
           'created_at': int(time.time()), 'pid': None}
    manifest = folder / 'run.json'
    private_json(manifest, run)
    created_pg = created_ch = False
    process = None
    try:
        pg('CREATE DATABASE ' + name)
        created_pg = True
        ch('CREATE DATABASE ' + name)
        created_ch = True
        environment = environment_for(run)
        descriptor = os.open(folder / 'server.log', os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, 'wb') as log:
            process = subprocess.Popen([str(BINARY), 'serve'], cwd=ROOT, env=environment,
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
        run['pid'] = process.pid
        run['process_started'] = subprocess.check_output(['ps','-p',str(process.pid),'-o','lstart='],text=True).strip()
        with manifest.open('w') as stream:
            json.dump(run, stream, indent=2)
        for _ in range(150):
            if process.poll() is not None:
                raise RuntimeError('Isolated server failed; inspect its private log')
            try:
                with urllib.request.urlopen(run['url'] + '/readyz', timeout=1) as response:
                    if response.status == 200:
                        break
            except OSError:
                pass
            time.sleep(.2)
        else:
            raise RuntimeError('Isolated server readiness timed out')
        password = secrets.token_urlsafe(32)
        email = os.environ['COMMERCIAL_TEST_EMAIL']
        customer = client()
        api(customer, run['url'], '/v1/auth/register',
            {'email': email, 'password': password, 'organization': 'Commercial validation · test only'})
        private_json(folder / 'credentials.json', {'email': email, 'password': password})
        if owner_only:
            owner_reset(manifest)
        else:
            api(customer, run['url'], '/v1/account/verification', {})
            api(customer, run['url'], '/v1/account/reset/request', {'email': email})
            issuer = client()
            issuer_email = 'issuer-' + uuid.uuid4().hex + '@commercial.invalid'
            api(issuer, run['url'], '/v1/auth/register',
                {'email': issuer_email, 'password': secrets.token_urlsafe(32), 'organization': 'Invitation validation · test only'})
            organizations = api(issuer, run['url'], '/v1/organizations')
            api(issuer, run['url'], '/v1/organizations/' + organizations[0]['id'] + '/invitations',
                {'email': email, 'role': 'viewer'})
        print('Isolated commercial app ready: ' + run['url'])
        print('Credentials: ' + str(folder / 'credentials.json'))
        print('Run manifest: ' + str(manifest))
        if owner_only:
            print('Owner-only recovery run; customer billing validation is kept separate.')
        else:
            print('Verification, reset and invitation are queued only to the configured inbox.')
            print('Open the app to test hosted Checkout/portal interactively. Forward only Stripe test webhooks to ' + run['url'] + '/v1/billing/webhooks/stripe')
        print('Confirm inbox receipt and sender authentication; use status to inspect delivery/plan state.')
    except BaseException:
        if process and process.poll() is None:
            process.terminate()
            process.wait(timeout=20)
        if created_ch:
            ch('DROP DATABASE ' + name)
        if created_pg:
            pg('DROP DATABASE ' + name + ' WITH (FORCE)')
        raise


def status(path):
    _, run = load_run(path)
    query = "SELECT json_build_object('subscriptions',(SELECT json_agg(json_build_object('plan',plan_id,'status',status,'provider',provider)) FROM subscriptions),'mail',(SELECT json_agg(row_to_json(m)) FROM (SELECT CASE WHEN subject LIKE '%%owner password' THEN 'owner_reset' WHEN subject LIKE 'Reset%%' THEN 'reset' WHEN subject LIKE 'Verify%%' THEN 'verify' ELSE 'invitation' END AS purpose,state,count(*) FROM mail_outbox GROUP BY 1,2) m),'completed_checkouts',(SELECT count(*) FROM billing_checkouts WHERE completed_at IS NOT NULL),'billing_events',(SELECT count(*) FROM billing_events))"
    print(json.dumps(json.loads(pg(query, run['database'])), indent=2))
    print('SMTP accepted/delivered state does not independently confirm inbox receipt.')


def owner_reset(path):
    path, run = load_run(path)
    credentials = json.loads((path.parent / 'credentials.json').read_text())
    result = subprocess.run([str(BINARY), 'grant-owner', credentials['email']], cwd=ROOT,
                            env=environment_for(run), capture_output=True)
    if result.returncode:
        raise RuntimeError('Isolated owner promotion failed; no credentials printed')
    api(client(), run['url'], '/owner/account/reset/request', {'email': credentials['email']}, form=True)
    print('Owner reset queued to the configured inbox. This isolated account now uses /owner/signin.')
    print('Run this phase after customer Checkout/portal/link validation is finished.')


def cleanup(path):
    path, run = load_run(path)
    pid = run.get('pid')
    if pid:
        process = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True)
        if process.returncode == 0:
            if str(BINARY) + ' serve' not in process.stdout:
                raise RuntimeError('PID is no longer the owned test server; refusing stop')
            started=subprocess.check_output(['ps','-p',str(pid),'-o','lstart='],text=True).strip()
            listeners=subprocess.run(['lsof','-tiTCP:'+str(run['port']),'-sTCP:LISTEN'],capture_output=True,text=True).stdout.split()
            if started!=run.get('process_started') or str(pid) not in listeners:
                raise RuntimeError('Owned process identity/listener changed; refusing stop')
            os.kill(pid, signal.SIGTERM)
            for _ in range(100):
                if subprocess.run(['kill', '-0', str(pid)], capture_output=True).returncode:
                    break
                time.sleep(.1)
            else:
                raise RuntimeError('Owned test server has not stopped; stores retained')
    ch('DROP DATABASE IF EXISTS ' + run['database'])
    pg('DROP DATABASE IF EXISTS ' + run['database'] + ' WITH (FORCE)')
    # Keep only non-secret run identity as cleanup evidence.
    credentials = path.parent / 'credentials.json'
    credentials.unlink(missing_ok=True)
    print('Stopped the owned test server and removed its isolated stores/credentials.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    prepare = sub.add_parser('setup')
    prepare.add_argument('--port', type=int, default=8084)
    prepare.add_argument('--owner-only', action='store_true', help='Create a separate owner recovery run without customer billing or other account mail')
    for command in ['status', 'owner-reset', 'cleanup']:
        action = sub.add_parser(command)
        action.add_argument('manifest')
    args = parser.parse_args()
    if args.command == 'setup':
        setup(args.port, owner_only=args.owner_only)
    elif args.command == 'status':
        status(args.manifest)
    elif args.command == 'owner-reset':
        owner_reset(args.manifest)
    else:
        cleanup(args.manifest)


if __name__ == '__main__':
    try:
        main()
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, subprocess.SubprocessError):
        # urllib/server/provider errors may contain private request details.
        print('Commercial validation did not complete. Check the required configuration, local services and private run log. No secrets were printed.', file=sys.stderr)
        sys.exit(1)
