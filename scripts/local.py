#!/usr/bin/env python3
"""Project-owned native development services. Normal commands never invoke Docker.

Credentials are read from the trusted environment, never printed or put in argv.
The optional, explicit legacy migration preserves Docker volumes for rollback.
"""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local'
NATIVE = LOCAL / 'native'
BIN = LOCAL / 'bin'
APP = BIN / 'click-server'
VERSIONS = json.loads((ROOT / 'scripts/native-versions.json').read_text())
BARRIER = 78422990

class LocalError(Exception):
    pass

def fail(message):
    raise LocalError(message)

def private_dir(path):
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.chmod(0o700)

def atomic(path, data, mode=0o600):
    private_dir(path.parent)
    temp = path.with_name(path.name + '.tmp-' + secrets.token_hex(6))
    try:
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data.encode() if isinstance(data, str) else data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)

def run(args, *, env=None, stdin=None, stdout=None, timeout=120, message='Command failed'):
    result = subprocess.run([str(x) for x in args], env=env, stdin=stdin,
                            stdout=stdout if stdout is not None else subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=timeout, cwd=ROOT)
    if result.returncode:
        fail(message)
    return result.stdout or b''

def identifier(value):
    if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]{0,62}', value):
        fail('Use a simple database/user identifier for managed local databases')
    return value

def config():
    if os.environ.get('APP_ENV', 'development') != 'development':
        fail('run.sh manages local development only; use deployment services in production')
    mode = os.environ.get('LOCAL_DB_MODE', 'native')
    if mode not in ('native', 'external'):
        fail('LOCAL_DB_MODE must be native or external')
    pg = urllib.parse.urlsplit(os.environ.get('DATABASE_URL', ''))
    ch = urllib.parse.urlsplit(os.environ.get('CLICKHOUSE_URL', ''))
    public = urllib.parse.urlsplit(os.environ.get('PUBLIC_URL', 'http://localhost:8082'))
    if pg.scheme not in ('postgres', 'postgresql') or not pg.hostname or not pg.username or not pg.password:
        fail('DATABASE_URL must specify a PostgreSQL host, user, password and database')
    database = identifier(urllib.parse.unquote(pg.path.lstrip('/')))
    if ch.scheme not in ('http', 'https') or not ch.hostname or ch.username or ch.password or ch.query or ch.fragment or ch.path not in ('', '/'):
        fail('CLICKHOUSE_URL must be a clean HTTP(S) origin')
    bind = os.environ.get('BIND', '127.0.0.1:8082').rsplit(':', 1)
    if len(bind) != 2 or bind[0] not in ('127.0.0.1', 'localhost'):
        fail('Local BIND must use a loopback address and port')
    if mode == 'native' and (pg.hostname not in ('127.0.0.1', 'localhost') or ch.hostname not in ('127.0.0.1', 'localhost') or ch.scheme != 'http'):
        fail('Managed native databases must use loopback PostgreSQL and HTTP ClickHouse URLs; use LOCAL_DB_MODE=external for other servers')
    user = identifier(urllib.parse.unquote(pg.username))
    ch_user = identifier(os.environ.get('CLICKHOUSE_USER', 'click'))
    password = os.environ.get('CLICKHOUSE_PASSWORD', '')
    if not password:
        fail('CLICKHOUSE_PASSWORD is required')
    ports = [pg.port or 5432, ch.port or (443 if ch.scheme=='https' else 8123), int(bind[1]), int(os.environ.get('LOCAL_CLICKHOUSE_TCP_PORT', '59000'))]
    if (mode=='native' and (any(not 1024 <= p <= 65535 for p in ports) or len(set(ports)) != len(ports))) or not 1024<=ports[2]<=65535:
        fail('Use distinct nonprivileged local service ports')
    return dict(mode=mode, pg=pg, ch=ch, database=database, user=user,
                password=urllib.parse.unquote(pg.password), ch_user=ch_user,
                ch_password=password, ch_database=identifier(os.environ.get('CLICKHOUSE_DATABASE', 'click')),
                pg_port=ports[0], ch_port=ports[1], app_port=ports[2], tcp_port=ports[3],
                public=urllib.parse.urlunsplit(public).rstrip('/'))

def init_env():
    path = ROOT / '.env'
    if path.exists():
        return
    pg, ch = secrets.token_hex(24), secrets.token_hex(24)
    atomic(path, '\n'.join([
        '# Private native local development configuration. Never commit.',
        'APP_ENV=development', 'LOCAL_DB_MODE=native', 'BIND=127.0.0.1:8082',
        'PUBLIC_URL=http://localhost:8082', 'ALLOW_LOCALHOST=true',
        f'DATABASE_URL=postgres://click:{pg}@127.0.0.1:55432/click',
        'CLICKHOUSE_URL=http://127.0.0.1:58123', 'CLICKHOUSE_USER=click',
        f'CLICKHOUSE_PASSWORD={ch}', 'CLICKHOUSE_DATABASE=click',
        'RUST_LOG=click_server=info,tower_http=info', ''
    ]))
    print('Created private .env for native local databases.')

def pg_bin(required=True):
    candidates = []
    if os.environ.get('LOCAL_PG_BINDIR'):
        candidates.append(Path(os.environ['LOCAL_PG_BINDIR']))
    if shutil.which('brew'):
        prefix = run(['brew', '--prefix'], message='Cannot locate Homebrew').decode().strip()
        candidates.append(Path(prefix) / 'opt/postgresql@17/bin')
    for path in candidates:
        if all((path / name).is_file() for name in ('postgres', 'pg_ctl', 'initdb', 'psql', 'pg_dump', 'pg_restore')):
            if re.search(r'\b17\.', run([path / 'postgres', '--version']).decode()):
                return path
    if required:
        fail('PostgreSQL 17 server tools are missing. Run ./run.sh setup (macOS), or set LOCAL_PG_BINDIR to PostgreSQL 17/bin.')
    return None

def install():
    private_dir(BIN)
    if pg_bin(False) is None:
        if platform.system() != 'Darwin' or not shutil.which('brew'):
            fail('Install PostgreSQL 17 server tools and set LOCAL_PG_BINDIR; macOS setup can use Homebrew')
        print('Installing native PostgreSQL 17 with Homebrew; existing PostgreSQL services are unchanged.', flush=True)
        with (LOCAL / 'postgres-install.log').open('ab') as log:
            run(['brew', 'install', 'postgresql@17'], env={**os.environ, 'HOMEBREW_NO_AUTO_UPDATE':'1', 'HOMEBREW_NO_INSTALL_CLEANUP':'1'}, stdout=log, timeout=1800, message='PostgreSQL installation failed; inspect .local/postgres-install.log')
    # Persist matching clients for backup/restore even when an older Homebrew
    # PostgreSQL installation takes precedence on the operator's shell PATH.
    tools = pg_bin()
    set_env_updates({'LOCAL_PG_BINDIR':str(tools)})
    os.environ['LOCAL_PG_BINDIR'] = str(tools)
    target = BIN / 'clickhouse'
    asset_name = 'clickhouse-macos-aarch64' if platform.machine() in ('arm64', 'aarch64') else 'clickhouse-macos'
    asset = VERSIONS['clickhouse']['assets'].get(asset_name)
    receipt = BIN / 'clickhouse-install.json'
    if target.exists():
        expected = asset['sha256'] if asset else ''
        installed = json.loads(receipt.read_text()) if receipt.exists() else {}
        if expected and installed.get('release_sha256') == expected and file_hash(target) == installed.get('installed_sha256'):
            return
        fail('Existing native ClickHouse binary does not match its verified installation receipt; preserve it and resolve the version before setup')
    if platform.system() != 'Darwin' or not asset or not re.fullmatch('[0-9a-f]{64}', asset['sha256']):
        fail('Automatic ClickHouse installation supports macOS Intel/Apple Silicon; use external services on other systems')
    print('Downloading the pinned official native ClickHouse binary (checksum verified).', flush=True)
    temp = target.with_name('clickhouse.download-' + secrets.token_hex(6))
    try:
        digest = hashlib.sha256()
        with urllib.request.urlopen(asset['url'], timeout=120) as response, temp.open('xb') as stream:
            os.chmod(temp, 0o700)
            size = 0
            while block := response.read(1024 * 1024):
                size += len(block)
                if size > 256 * 1024 * 1024:
                    fail('Native ClickHouse download exceeds the expected size limit')
                digest.update(block)
                stream.write(block)
            stream.flush()
            os.fsync(stream.fileno())
        if digest.hexdigest() != asset['sha256']:
            fail('Native ClickHouse release checksum mismatch; installation refused')
        os.replace(temp, target)
    finally:
        temp.unlink(missing_ok=True)
    version = run([target, 'client', '--version'], timeout=60, message='Cannot execute the verified native ClickHouse binary').decode()
    if VERSIONS['clickhouse']['version'] not in version:
        fail('Native ClickHouse reported an unexpected version')
    # Official single-binary distributions unpack themselves on first use.
    # Verify the compressed release before executing, then record that native
    # executable's digest for subsequent idempotent setup checks.
    atomic(receipt,json.dumps({'version':VERSIONS['clickhouse']['version'],'release_sha256':asset['sha256'],'installed_sha256':file_hash(target)},indent=2)+'\n')

def file_hash(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def pg_env(c, database=None):
    return {**os.environ, 'PGHOST':c['pg'].hostname, 'PGPORT':str(c['pg_port']),
            'PGUSER':c['user'], 'PGPASSWORD':c['password'], 'PGDATABASE':database or c['database'],
            'PGCONNECT_TIMEOUT':'5', 'PGOPTIONS':'-c timezone=UTC'}

def pg(c, query, database=None):
    tools=pg_bin(False)
    executable=tools/'psql' if tools else shutil.which('psql')
    if not executable:fail('PostgreSQL client tools are missing; run setup or install psql')
    return run([executable, '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1', '-c', query], env=pg_env(c, database), message='PostgreSQL query failed; verify the local service and private credentials').decode().strip()

def ch(c, query, database=None, *, source=None, destination=None, parameters=None):
    endpoint = c['ch']
    params = {'database':database or c['ch_database'], 'max_threads':'1', 'max_final_threads':'1', 'max_memory_usage':'268435456', 'max_execution_time':'120', 'max_block_size':'1024', 'max_read_buffer_size':'65536', 'preferred_block_size_bytes':'65536', 'use_uncompressed_cache':'0', 'min_insert_block_size_rows':'10000', 'min_insert_block_size_bytes':'8388608', 'max_insert_block_size':'10000', 'input_format_parallel_parsing':'0'}
    if parameters:
        params.update(parameters)
    if source is not None:
        params['query'] = query
        data = source
    else:
        data = query.encode()
    request = urllib.request.Request(urllib.parse.urlunsplit(endpoint._replace(query=urllib.parse.urlencode(params))), data=data,
        headers={'X-ClickHouse-User':c['ch_user'], 'X-ClickHouse-Key':c['ch_password'], 'Content-Type':'application/octet-stream'})
    if source is not None:
        request.add_header('Content-Length', str(os.fstat(source.fileno()).st_size))
    with urllib.request.urlopen(request, timeout=180) as response:
        if destination is not None:
            shutil.copyfileobj(response, destination, 1024 * 1024)
            return b''
        return response.read()

def busy(port):
    with socket.socket() as sock:
        sock.settimeout(.25)
        return sock.connect_ex(('127.0.0.1', port)) == 0

def pid_value(path):
    try:
        value = int(path.read_text().strip())
        return value if value > 1 else None
    except (ValueError, OSError):
        return None

def command(pid):
    if not pid:
        return ''
    result = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else ''

def owned(pid, markers):
    cmd = command(pid)
    return bool(cmd and any(marker in cmd for marker in markers))

def app_markers():
    return [str(APP) + ' serve', str(ROOT / 'target/debug/click-server') + ' serve', str(ROOT / 'target/release/click-server') + ' serve']

def app_pid(c):
    saved = pid_value(LOCAL / 'server.pid')
    if owned(saved, app_markers()):
        return saved
    result = subprocess.run(['lsof', '-tiTCP:' + str(c['app_port']), '-sTCP:LISTEN'], capture_output=True, text=True)
    for value in result.stdout.split():
        if value.isdigit() and owned(int(value), app_markers()):
            return int(value)
    return None

def stop_app(c):
    pid = app_pid(c)
    if pid:
        os.kill(pid, signal.SIGTERM)
        until = time.monotonic() + 30
        while owned(pid, app_markers()) and time.monotonic() < until:
            time.sleep(.1)
        if owned(pid, app_markers()):
            fail('Application did not stop gracefully; databases were left running')
        print('Application stopped.')
    elif busy(c['app_port']):
        fail('The application port belongs to another process; refusing to stop it')
    (LOCAL / 'server.pid').unlink(missing_ok=True)

def spawn(args, logfile, pidfile, env=None):
    private_dir(logfile.parent)
    fd = os.open(logfile, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    with os.fdopen(fd, 'ab', buffering=0) as log:
        proc = subprocess.Popen([str(x) for x in args], cwd=ROOT, stdin=subprocess.DEVNULL,
                                stdout=log, stderr=log, start_new_session=True, close_fds=True, env={**os.environ,**(env or {})})
    atomic(pidfile, str(proc.pid) + '\n')
    return proc

def wait_for(probe, message, process=None, seconds=40):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if process is not None and process.poll() is not None:
            fail(message)
        try:
            if probe():
                return
        except (OSError, LocalError):
            pass
        time.sleep(.2)
    fail(message)

def pg_running():
    data = NATIVE / 'postgres'
    if not (data / 'PG_VERSION').exists():
        return False
    return subprocess.run([str(pg_bin() / 'pg_ctl'), 'status', '-D', str(data)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0

def prepare_pg(c):
    data = NATIVE / 'postgres'
    if not (data / 'PG_VERSION').exists():
        if data.exists() and any(data.iterdir()):
            fail('Native PostgreSQL directory is not empty and has no cluster; restore or move it before initialization')
        private_dir(data)
        password_file = NATIVE / ('pg-password-' + secrets.token_hex(6))
        atomic(password_file, c['password'] + '\n')
        try:
            run([pg_bin() / 'initdb', '-D', data, '--username', c['user'], '--pwfile', password_file,
                 '--auth-local=scram-sha-256', '--auth-host=scram-sha-256', '--encoding=UTF8', '--locale=C'], timeout=120, message='Native PostgreSQL initialization failed')
        finally:
            password_file.unlink(missing_ok=True)
    if (data / 'PG_VERSION').read_text().strip() != '17':
        fail('Managed PostgreSQL cluster must remain on major version 17')
    atomic(data / 'postgresql.auto.conf', '\n'.join([
        "listen_addresses = '127.0.0.1'", f"port = {c['pg_port']}", "unix_socket_directories = ''",
        "timezone = 'UTC'", "shared_buffers = '64MB'", "work_mem = '4MB'", "maintenance_work_mem = '32MB'",
        'max_connections = 80', 'log_statement = \'none\'', 'log_min_error_statement = panic', ''
    ]))

def prepare_ch(c):
    folder = NATIVE / 'clickhouse'
    for name in ('data', 'tmp', 'user_files', 'format_schemas', 'access'):
        private_dir(folder / name)
    root = ET.Element('clickhouse')
    def add(parent, key, value):
        ET.SubElement(parent, key).text = str(value)
    log = ET.SubElement(root, 'logger')
    for key, value in [('level','warning'),('log',folder/'server.log'),('errorlog',folder/'error.log'),('size','10M'),('count',2),('console',False)]:
        add(log,key,str(value).lower() if isinstance(value,bool) else value)
    for key, value in [('listen_host','127.0.0.1'),('timezone','UTC'),('http_port',c['ch_port']),('tcp_port',c['tcp_port']),
                       ('path',str(folder/'data')+'/'),('tmp_path',str(folder/'tmp')+'/'),('user_files_path',str(folder/'user_files')+'/'),
                       ('format_schema_path',str(folder/'format_schemas')+'/'),('access_control_path',str(folder/'access')+'/'),
                       ('max_connections',64),('max_concurrent_queries',8),('max_server_memory_usage',1610612736),
                       # Background workers share the global pool. Leave headroom
                       # for metadata cleanup after integration databases are dropped.
                       ('mark_cache_size',33554432),('uncompressed_cache_size',16777216),('max_thread_pool_size',512),
                       ('max_thread_pool_free_size',32),('tables_loader_foreground_pool_size',2),('tables_loader_background_pool_size',2),
                       ('background_pool_size',4),('background_schedule_pool_size',4),('background_fetches_pool_size',2),
                       ('background_move_pool_size',2),('background_common_pool_size',2),('background_buffer_flush_schedule_pool_size',1),
                       ('background_message_broker_schedule_pool_size',1),('background_distributed_schedule_pool_size',1)]:
        add(root,key,value)
    merge = ET.SubElement(root,'merge_tree')
    for key in ('number_of_free_entries_in_pool_to_execute_mutation','number_of_free_entries_in_pool_to_lower_max_size_of_merge','number_of_free_entries_in_pool_to_execute_optimize_entire_partition'):
        add(merge,key,1)
    profiles = ET.SubElement(root,'profiles');default = ET.SubElement(profiles,'default')
    for key,value in [('max_threads',2),('max_final_threads',2),('max_memory_usage',268435456),('max_block_size',8192),
                      ('preferred_block_size_bytes',262144),('max_read_buffer_size',262144),
                      ('max_bytes_before_external_group_by',134217728),('max_bytes_before_external_sort',67108864)]:
        add(default,key,value)
    users=ET.SubElement(root,'users')
    for name,password in [(c['ch_user'],c['ch_password'])] + ([] if c['ch_user']=='default' else [('default',secrets.token_hex(32))]):
        user=ET.SubElement(users,name);add(user,'password_sha256_hex',hashlib.sha256(password.encode()).hexdigest())
        networks=ET.SubElement(user,'networks');add(networks,'ip','127.0.0.1');add(user,'profile','default');add(user,'quota','default');add(user,'access_management',1)
    quotas=ET.SubElement(root,'quotas');ET.SubElement(quotas,'default')
    atomic(folder/'config.xml',ET.tostring(root,encoding='unicode')+'\n')

def clickhouse_pid():
    marker=str(NATIVE/'clickhouse/config.xml')
    saved=pid_value(NATIVE/'clickhouse.pid')
    if owned(saved,[marker]):return saved
    # Watchdog changes its own argv; only its child server retains our exact
    # project config. Never signal the watchdog or another project's server.
    result=subprocess.run(['ps','-axo','pid=,ppid=,command='],capture_output=True,text=True)
    found=[]
    for line in result.stdout.splitlines():
        parts=line.strip().split(None,2)
        if len(parts)==3 and parts[0].isdigit() and str(BIN/'clickhouse') in parts[2] and marker in parts[2]:found.append(int(parts[0]))
    if len(found)>1:fail('Multiple native ClickHouse servers use this config; resolve them before local control')
    return found[0] if found else None


def db_start(c, create=True):
    if c['mode']=='external':
        wait_for(lambda:pg(c,'SELECT 1')=='1' and ch(c,'SELECT 1')==b'1\n','External databases are unavailable',seconds=10)
        return
    if pg_bin(False) is None or not (BIN/'clickhouse').exists():
        install()
    if not pg_running():
        if busy(c['pg_port']):
            fail('PostgreSQL port is occupied by an unmanaged service. For the old Docker stack run ./run.sh migrate-docker; nothing was stopped.')
        prepare_pg(c)
        run([pg_bin()/'pg_ctl','start','-D',NATIVE/'postgres','-l',NATIVE/'postgres.log','-w','-t','40'],timeout=50,message='Native PostgreSQL did not start; inspect .local/native/postgres.log')
    if create and pg(c,"SELECT count(*) FROM pg_database WHERE datname='"+c['database']+"'",'postgres')=='0':
        pg(c,'CREATE DATABASE "'+c['database']+'"','postgres')
    ch_pid=clickhouse_pid()
    marker=str(NATIVE/'clickhouse/config.xml')
    if not owned(ch_pid,[marker]):
        if busy(c['ch_port']) or busy(c['tcp_port']):
            fail('ClickHouse ports are occupied by an unmanaged service; no process was stopped')
        prepare_ch(c)
        proc=spawn([BIN/'clickhouse','server','--config-file='+marker],NATIVE/'clickhouse-console.log',NATIVE/'clickhouse.pid',env={'CLICKHOUSE_WATCHDOG_ENABLE':'0'})
        wait_for(lambda:ch(c,'SELECT 1',database='default')==b'1\n','Native ClickHouse did not start; inspect .local/native/clickhouse-console.log',proc,60)
    # An owned process may still be loading metadata after a previous start.
    wait_for(lambda:ch(c,'SELECT 1',database='default')==b'1\n','Native ClickHouse is not ready; inspect .local/native/clickhouse/server.log',seconds=60)
    if create:
        ch(c,'CREATE DATABASE IF NOT EXISTS '+c['ch_database'],database='default')
    wait_for(lambda:pg(c,'SELECT 1')=='1' and ch(c,'SELECT 1')==b'1\n','Native database authentication/readiness failed')
    atomic(NATIVE/'managed.json',json.dumps({'format':1,'postgres':17,'clickhouse':VERSIONS['clickhouse']['version'],'project':str(ROOT)})+'\n')

def db_stop(c):
    if c['mode']=='external':
        return
    pid=clickhouse_pid();marker=str(NATIVE/'clickhouse/config.xml')
    if owned(pid,[marker]):
        os.kill(pid,signal.SIGTERM)
        wait_for(lambda:not owned(pid,[marker]),'Native ClickHouse did not stop gracefully; its data was retained',seconds=30)
    elif busy(c['ch_port']):
        fail('ClickHouse port is held by an unmanaged process; refusing to stop it')
    if busy(c['ch_port']):fail('Native ClickHouse still holds its port; data paths were not moved')
    (NATIVE/'clickhouse.pid').unlink(missing_ok=True)
    if pg_running():
        run([pg_bin()/'pg_ctl','stop','-D',NATIVE/'postgres','-m','fast','-w','-t','40'],timeout=50,message='Native PostgreSQL did not stop gracefully; its data was retained')
    print('Managed native databases stopped; data retained.')

def inputs_newer():
    if not APP.exists():
        return True
    paths=[ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'docs/openapi.json']
    for parent in ('crates','migrations','dashboard','tracker/dist'):
        paths.extend(p for p in (ROOT/parent).rglob('*') if p.is_file() and (parent!='crates' or '/src/' in str(p) or p.name in ('Cargo.toml','build.rs')))
    return any(p.stat().st_mtime_ns>APP.stat().st_mtime_ns for p in paths)

def build(clean=False):
    if not shutil.which('cargo'):
        fail('Install Rust stable before building Vuuut Click')
    env={**os.environ,'CARGO_TARGET_DIR':os.environ.get('CARGO_TARGET_DIR',str(LOCAL/'build')),
         'CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0'}
    if clean:
        subprocess.run(['cargo','clean','-p','click-server'],cwd=ROOT,env=env,check=True)
    result=subprocess.run(['cargo','build','--locked','-p','click-server'],cwd=ROOT,env=env)
    if result.returncode:
        fail('Rust build failed; the installed runtime and running services were preserved')
    source=Path(env['CARGO_TARGET_DIR'])/'debug/click-server'
    if not source.is_absolute():source=ROOT/source
    private_dir(BIN)
    temp=BIN/('click-server.install-'+secrets.token_hex(6))
    shutil.copyfile(source,temp);temp.chmod(0o700);os.replace(temp,APP)
    legacy=ROOT/'target/debug/click-server';legacy.parent.mkdir(parents=True,exist_ok=True)
    if source.resolve()!=legacy.resolve():
        temp_link=legacy.with_name('click-server.link-'+secrets.token_hex(4));temp_link.symlink_to(APP);os.replace(temp_link,legacy)
    print('Installed Rust runtime outside the build cache.')

def app_start(c):
    if app_pid(c):
        if c['mode']=='native' and not (NATIVE/'managed.json').exists():fail('Application is using unmanaged databases; finish ./run.sh migrate-docker before using native mode')
        print('Application already running at '+c['public']);return
    if busy(c['app_port']):
        fail('Application port is occupied by another process; nothing was stopped')
    if inputs_newer():build()
    db_start(c)
    proc=spawn([APP,'serve'],LOCAL/'server.log',LOCAL/'server.pid')
    def ready():
        with urllib.request.urlopen('http://127.0.0.1:'+str(c['app_port'])+'/readyz',timeout=2) as response:
            return response.status==200
    try:
        wait_for(ready,'Application did not become ready; inspect .local/server.log',proc,45)
    except LocalError:
        if proc.poll() is None:proc.terminate()
        raise
    print('Started Vuuut Click: '+c['public'])
    print('Customer: '+c['public']+'/signin | Owner: '+c['public']+'/owner/signin')

def status(c):
    pid=app_pid(c)
    print('Application: '+('running (pid '+str(pid)+')' if pid else 'stopped'))
    if c['mode']=='external':
        print('Databases: external; launcher does not start or stop them')
    else:
        print('PostgreSQL: '+('running' if pg_bin(False) and pg_running() else 'stopped')+' (project-owned, port '+str(c['pg_port'])+')')
        print('ClickHouse: '+('running' if owned(clickhouse_pid(),[str(NATIVE/'clickhouse/config.xml')]) else 'stopped')+' (project-owned, port '+str(c['ch_port'])+')')
    print('URL: '+c['public'])

def doctor(c):
    status(c)
    results=[]
    checks=[('Installed application',lambda:APP.is_file()),('PostgreSQL 17 tools',lambda:pg_bin(False) is not None),
            ('PostgreSQL authentication',lambda:pg(c,'SELECT 1')=='1'),('ClickHouse authentication',lambda:ch(c,'SELECT 1')==b'1\n')]
    for label,probe in checks:
        try:ok=bool(probe())
        except (OSError,LocalError):ok=False
        print(('OK   ' if ok else 'FAIL ')+label);results.append(ok)
    if not all(results):fail('Doctor found an incomplete local setup; run ./run.sh setup or start')

def control_fingerprint(c):
    tables=json.loads(pg(c,"SELECT coalesce(json_agg(table_name ORDER BY table_name),'[]') FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE'"))
    result={}
    for table in tables:
        identifier(table)
        row=pg(c,'SELECT json_build_object(\'rows\',count(*),\'digest\',md5(coalesce(string_agg(h,\'\' ORDER BY h COLLATE "C"),\'\'))) FROM (SELECT md5(row_to_json(t)::text) AS h FROM "'+table+'" t) s')
        result[table]=json.loads(row)
    return result

def analytics_fingerprint(c):
    events=ch(c,"SELECT length(h) AS rows,arrayReduce('groupBitXor',h) AS xor,arraySum(h) AS sum FROM (SELECT groupUniqArray(cityHash64(tuple(*))) AS h FROM events) FORMAT JSONEachRow").decode().strip()
    # Aggregate table physical states can merge; compare their logical states.
    hours=ch(c,"SELECT organization_id,website_id,toString(bucket) AS bucket,uniqExactMerge(views) AS views,uniqExactMerge(visitors) AS visitors,uniqExactMerge(sessions) AS sessions FROM pageviews_hourly GROUP BY organization_id,website_id,bucket ORDER BY organization_id,website_id,bucket FORMAT JSONEachRow").decode()
    return {'events':json.loads(events),'hourly_sha256':hashlib.sha256(hours.encode()).hexdigest(),'hourly_rows':len(hours.splitlines())}

def save_snapshot(c, folder):
    private_dir(folder)
    atomic(folder/'configuration.env',(ROOT/'.env').read_bytes())
    atomic(folder/'control-before.json',json.dumps(control_fingerprint(c),indent=2)+'\n')
    atomic(folder/'analytics-before.json',json.dumps(analytics_fingerprint(c),indent=2)+'\n')
    with (folder/'postgres.dump').open('xb') as stream:
        os.chmod(stream.name,0o600)
        run([pg_bin()/'pg_dump','--format=custom','--no-owner','--no-acl'],env=pg_env(c),stdout=stream,timeout=300,message='PostgreSQL migration export failed; source data remains intact')
    tables=json.loads(ch(c,"SELECT name,engine FROM system.tables WHERE database=currentDatabase() ORDER BY name FORMAT JSON").decode())['data']
    if {row['name'] for row in tables}!={'events','pageviews_hourly','pageviews_hourly_mv'}:
        fail('Unexpected analytics tables; refusing a partial migration. Source databases remain intact.')
    # TabSeparated escapes SHOW CREATE text; JSONEachRow carries the exact DDL.
    definitions={row['name']:json.loads(ch(c,'SHOW CREATE TABLE '+identifier(row['name'])+' FORMAT JSONEachRow').decode())['statement'] for row in tables}
    atomic(folder/'schema.json',json.dumps(definitions,indent=2)+'\n')
    # Native contains bounded 1024-row blocks. Stream the raw table once:
    # repeatedly sorting wide FINAL pages can exhaust a small source VM.
    # ReplacingMergeTree duplicates are preserved; all logical fingerprints
    # and application reports still resolve them with FINAL/unique states.
    chunks=['events.native']
    with (folder/'events.native').open('xb') as stream:
        os.chmod(stream.name,0o600)
        ch(c,'SELECT * FROM events FORMAT Native',destination=stream)
    with (folder/'pageviews_hourly.native').open('xb') as stream:
        os.chmod(stream.name,0o600);ch(c,'SELECT * FROM pageviews_hourly FORMAT Native',destination=stream)
    files=['postgres.dump','schema.json','control-before.json','analytics-before.json','configuration.env','pageviews_hourly.native',*chunks]
    manifest={'format':1,'events':chunks,'files':{name:file_hash(folder/name) for name in files}}
    atomic(folder/'manifest.json',json.dumps(manifest,indent=2)+'\n')
    return manifest

def restore_snapshot(c,folder,manifest):
    for name,digest in manifest['files'].items():
        if Path(name).name!=name or file_hash(folder/name)!=digest:
            fail('Migration snapshot integrity validation failed')
    if pg(c,'SELECT count(*) FROM information_schema.tables WHERE table_schema=\'public\' AND table_type=\'BASE TABLE\'')!='0':
        fail('Native target PostgreSQL database is not empty; refusing to overwrite it')
    if int(ch(c,'SELECT count() FROM system.tables WHERE database=currentDatabase()')):
        fail('Native target ClickHouse database is not empty; refusing to overwrite it')
    with (folder/'postgres.dump').open('rb') as stream:
        run([pg_bin()/'pg_restore','--exit-on-error','--single-transaction','--no-owner','--no-acl','--dbname='+c['database']],env=pg_env(c),stdin=stream,timeout=300,message='Native PostgreSQL restore failed; the original Docker volume and snapshot were retained')
    definitions=json.loads((folder/'schema.json').read_text())
    for name in ('events','pageviews_hourly'):
        ch(c,definitions[name])
    # Load both tables before creating the MV, so imported raw events cannot
    # double-count existing hourly aggregate states.
    for name in manifest['events']:
        with (folder/name).open('rb') as source:ch(c,'INSERT INTO events FORMAT Native',source=source)
    with (folder/'pageviews_hourly.native').open('rb') as source:ch(c,'INSERT INTO pageviews_hourly FORMAT Native',source=source)
    ch(c,definitions['pageviews_hourly_mv'])
    if control_fingerprint(c)!=json.loads((folder/'control-before.json').read_text()):
        fail('PostgreSQL row fingerprints differ; native cutover was refused')
    if analytics_fingerprint(c)!=json.loads((folder/'analytics-before.json').read_text()):
        fail('Analytics logical fingerprints differ; native cutover was refused')

def set_env_updates(updates):
    path=ROOT/'.env';lines=path.read_text().splitlines();seen=set();out=[]
    import shlex
    for line in lines:
        key=line.split('=',1)[0].strip()
        if key in updates:
            if key not in seen:out.append(key+'='+shlex.quote(str(updates[key])));seen.add(key)
        else:out.append(line)
    out.extend(key+'='+shlex.quote(str(value)) for key,value in updates.items() if key not in seen)
    atomic(path,'\n'.join(out)+'\n')

def docker_compose(*args):
    if not shutil.which('docker'):
        fail('The one-time legacy migration requires Docker while reading its existing volumes; normal native commands do not')
    return run(['docker','compose','--env-file',ROOT/'.env','-f',ROOT/'deploy/compose.dev.yml',*args],timeout=90,message='Cannot manage the legacy development containers; no Docker volumes were removed')

@contextlib.contextmanager
def migration_barrier(c,folder):
    import select
    fd=os.open(folder/'migration-lock.log',os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)
    with os.fdopen(fd,'ab') as log:
        env=pg_env(c);env['PGOPTIONS']='-c timezone=UTC -c lock_timeout=15000'
        proc=subprocess.Popen([str(pg_bin()/'psql'),'-X','-q','-A','-t','-v','ON_ERROR_STOP=1'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,bufsize=0)
        try:
            proc.stdin.write(("BEGIN; SELECT pg_advisory_xact_lock("+str(BARRIER)+"); SELECT 'migration_locked';\n").encode());proc.stdin.flush()
            until=time.monotonic()+20;locked=False
            while time.monotonic()<until:
                if select.select([proc.stdout],[],[],.2)[0]:
                    line=proc.stdout.readline()
                    if line.strip()==b'migration_locked':locked=True;break
                    if not line:break
                if proc.poll() is not None:break
            if not locked:fail('Could not acquire the source maintenance barrier; no database was migrated')
            yield
        finally:
            if proc.poll() is None:
                with contextlib.suppress(BrokenPipeError,OSError):proc.stdin.write(b'COMMIT;\n\\q\n');proc.stdin.flush()
                try:proc.wait(timeout=5)
                except subprocess.TimeoutExpired:proc.terminate();proc.wait(timeout=5)
            proc.stdin.close();proc.stdout.close()


def migrate(c):
    receipt=NATIVE/'migration-complete.json'
    if receipt.exists():
        print('Native migration was already verified. Original Docker volumes are retained.');status(c);return
    if (NATIVE/'postgres/PG_VERSION').exists() or (NATIVE/'clickhouse/data/store').exists():
        fail('Native data already exists without a completed migration receipt; refusing to overwrite it')
    if c['mode']!='native':fail('Use LOCAL_DB_MODE=native for the one-time Docker migration')
    ids={name:docker_compose('ps','-q',name).decode().strip() for name in ('postgres','clickhouse')}
    if not all(re.fullmatch('[0-9a-f]{12,64}',value) for value in ids.values()):
        fail('Start the original project Docker PostgreSQL/ClickHouse containers before migrating their data')
    # Require endpoints to belong to those exact containers, not another DB.
    for name,port in [('postgres',c['pg_port']),('clickhouse',c['ch_port'])]:
        bindings=json.loads(run(['docker','inspect',ids[name],'--format','{{json .NetworkSettings.Ports}}']))
        if not any(str(port)==b.get('HostPort') and b.get('HostIp')=='127.0.0.1' for group in bindings.values() if group for b in group):
            fail('Configured database ports do not match the project Docker containers; migration refused')
    install()
    folder=LOCAL/'native-migration'/(time.strftime('%Y%m%d-%H%M%S')+'-'+secrets.token_hex(3))
    private_dir(folder)
    was_running=bool(app_pid(c));stopped_containers=False;committed=False
    try:
        stop_app(c)
        # All app workers drain/close on SIGTERM. Existing CMS/pro retries keep
        # their own durable outboxes until the same URLs become available.
        pending=int(ch(c,'SELECT count() FROM system.mutations WHERE database=currentDatabase() AND NOT is_done',database=c['ch_database']))
        if pending:fail('Analytics mutations are still running; retry migration after they finish')
        print('Exporting private PostgreSQL and ClickHouse snapshots.',flush=True)
        with migration_barrier(c,folder):
            for query in ('SYSTEM DROP UNCOMPRESSED CACHE','SYSTEM DROP MARK CACHE','SYSTEM JEMALLOC PURGE'):
                with contextlib.suppress(Exception):ch(c,query,database='system')
            for table in ('events','pageviews_hourly'):ch(c,'SYSTEM STOP MERGES '+c['ch_database']+'.'+table)
            wait_for(lambda:int(ch(c,"SELECT count() FROM system.merges WHERE database=currentDatabase()"))==0,'Source analytics merges did not quiesce; migration refused',seconds=30)
            manifest=save_snapshot(c,folder)
            docker_compose('stop','postgres','clickhouse');stopped_containers=True
        atomic(NATIVE/'migration-in-progress',str(folder)+'\n')
        print('Restoring into native databases and comparing every control table and logical analytics totals.',flush=True)
        db_start(c)
        restore_snapshot(c,folder,manifest)
        updates={'LOCAL_DB_MODE':'native','LOCAL_PG_BINDIR':str(pg_bin()),'BACKUP_PG_CONTAINER':''}
        if not os.environ.get('CARGO_TARGET_DIR') and (ROOT.parent/'.dev-build').is_dir():updates['CARGO_TARGET_DIR']=str(ROOT.parent/'.dev-build')
        # The bundled Docker collector is optional. Keep its old setting in
        # the private migration snapshot; native app startup does not need it.
        endpoint=os.environ.get('OTEL_EXPORTER_OTLP_ENDPOINT','')
        if endpoint and urllib.parse.urlsplit(endpoint).hostname in ('localhost','127.0.0.1','host.docker.internal','otel'):
            updates['OTEL_EXPORTER_OTLP_ENDPOINT']=''
        set_env_updates(updates);os.environ.update(updates)
        atomic(receipt,json.dumps({'format':1,'verified':True,'snapshot':str(folder),'postgres_tables':len(control_fingerprint(c)),'analytics':analytics_fingerprint(c),'original_containers':ids},indent=2)+'\n')
        (NATIVE/'migration-in-progress').unlink(missing_ok=True);committed=True
        if was_running:app_start(c)
        print('Native migration verified. Accounts, credentials, usage, queued events and analytics were preserved.')
        print('Original Docker volumes are retained for rollback; normal run.sh commands no longer use Docker.')
    except BaseException:
        if not committed:
            if not stopped_containers:
                for table in ('events','pageviews_hourly'):
                    with contextlib.suppress(Exception):ch(c,'SYSTEM START MERGES '+c['ch_database']+'.'+table)
            if stopped_containers:
                db_stop(c)
                if NATIVE.exists():NATIVE.rename(LOCAL/('native-failed-'+secrets.token_hex(6)))
                run(['docker','start',*ids.values()],timeout=90,message='Legacy containers need manual restart; their volumes and snapshot remain intact')
                wait_for(lambda:pg(c,'SELECT 1')=='1' and ch(c,'SELECT 1')==b'1\n','Legacy database readiness did not recover',seconds=60)
            if was_running:
                with contextlib.suppress(Exception):app_start({**c,'mode':'external'})
        raise

def logs(service,follow=False):
    paths={'app':LOCAL/'server.log','postgres':NATIVE/'postgres.log','clickhouse':NATIVE/'clickhouse/server.log'}
    path=paths[service]
    if service=='clickhouse' and not path.exists():path=NATIVE/'clickhouse-console.log'
    if not path.exists():print('No '+service+' log yet.');return
    subprocess.run(['tail',*(['-f'] if follow else ['-n','60']),str(path)],check=False)

def main(argv=None):
    os.umask(0o077)
    if sys.version_info<(3,11):fail('Install Python 3.11 or newer for native local service control')
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['init-env','setup','start','stop','restart','status','doctor','build','rebuild','test','db-start','db-stop','log','logs','migrate-docker'])
    parser.add_argument('service',nargs='?',choices=['app','postgres','clickhouse'],default='app')
    parser.add_argument('-f','--follow',action='store_true')
    args=parser.parse_args(argv)
    private_dir(LOCAL)
    if args.command=='init-env':
        if not (ROOT/'.env').exists() and (NATIVE/'postgres/PG_VERSION').exists():fail('Restore the matching private .env before starting an existing native database')
        init_env();return
    if args.command in ('log','logs'):logs(args.service,args.follow);return
    if args.command in ('build','rebuild'):
        build(args.command=='rebuild');return
    if args.command=='test':
        env={**os.environ,'CARGO_TARGET_DIR':os.environ.get('CARGO_TARGET_DIR',str(LOCAL/'build')),'CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0'}
        raise SystemExit(subprocess.call(['cargo','test','--workspace'],env=env,cwd=ROOT))
    if not (ROOT/'.env').exists():
        print('Local setup is not initialized. Run ./run.sh setup or start.');return
    c=config()
    if args.command=='status':status(c);return
    if args.command=='doctor':doctor(c);return
    with (LOCAL/'control.lock').open('a') as lock:
        try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:fail('Another local management command is running; use ./run.sh status to inspect services')
        if (NATIVE/'migration-in-progress').exists() and args.command not in ('stop','db-stop','migrate-docker'):
            fail('A native migration is incomplete; preserve its snapshot and finish recovery before starting the app')
        if args.command=='setup':
            if c['mode']=='native':install()
            print('Native prerequisites ready. ./run.sh start starts only this project\'s services.')
        elif args.command=='start':app_start(c)
        elif args.command=='stop':stop_app(c);db_stop(c)
        elif args.command=='restart':
            if inputs_newer():build()
            stop_app(c);db_stop(c);app_start(c)
        elif args.command=='db-start':db_start(c)
        elif args.command=='db-stop':stop_app(c);db_stop(c)
        elif args.command=='migrate-docker':migrate(c)

if __name__=='__main__':
    try:main()
    except KeyboardInterrupt:raise SystemExit(130)
    except (LocalError,OSError,subprocess.SubprocessError,ValueError,KeyError):
        # Never print provider bodies, credential-bearing env/URLs or raw
        # subprocess errors. LocalError messages above contain fixed guidance.
        error=sys.exc_info()[1]
        print(str(error) if isinstance(error,LocalError) else 'Local service operation failed; private logs/data were retained. Run ./run.sh doctor.',file=sys.stderr)
        raise SystemExit(1)
