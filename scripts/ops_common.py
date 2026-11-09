"""Private operator I/O. Never log credentials, SQL rows, or provider responses."""
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.parse
import urllib.request
import psycopg

ROOT = Path(__file__).resolve().parent.parent
BARRIER = 78422990

def identifier(value):
    if not re.fullmatch(r'[a-zA-Z_][a-zA-Z0-9_]{0,62}', value):
        raise RuntimeError('Invalid database identifier')
    return value

def connect(database=None, autocommit=False):
    parsed = urllib.parse.urlsplit(os.environ['DATABASE_URL'])
    if database: parsed = parsed._replace(path='/' + identifier(database))
    return psycopg.connect(urllib.parse.urlunsplit(parsed), autocommit=autocommit, connect_timeout=5)

def source_database():
    return identifier(urllib.parse.unquote(urllib.parse.urlsplit(os.environ['DATABASE_URL']).path.lstrip('/')))

def ch(query, database=None, payload=None, destination=None, payload_size=None, params=None):
    endpoint = urllib.parse.urlsplit(os.environ['CLICKHOUSE_URL'])
    if endpoint.username or endpoint.password or endpoint.query or endpoint.fragment:
        raise RuntimeError('Use a clean private ClickHouse endpoint and credential headers')
    parameters = {'database': identifier(database or os.environ.get('CLICKHOUSE_DATABASE','click')), 'max_threads':'2', 'max_memory_usage':'268435456', 'max_execution_time':'120', 'max_block_size':'8192', 'min_insert_block_size_rows':'10000', 'min_insert_block_size_bytes':'8388608'}
    if params:parameters.update({'param_'+key:str(value) for key,value in params.items()})
    if payload is not None: parameters['query'] = query
    request = urllib.request.Request(urllib.parse.urlunsplit(endpoint._replace(query=urllib.parse.urlencode(parameters))), data=payload if payload is not None else query.encode(), headers={'Content-Type':'application/octet-stream','X-ClickHouse-User':os.environ['CLICKHOUSE_USER'],'X-ClickHouse-Key':os.environ['CLICKHOUSE_PASSWORD']})
    if payload_size is not None:request.add_header("Content-Length",str(payload_size))
    with urllib.request.urlopen(request, timeout=120) as response:
        if destination is not None:
            while block := response.read(1024*1024): destination.write(block)
            return None
        return response.read()

def private_json(path, value, exclusive=False):
    descriptor = os.open(path,os.O_CREAT|os.O_WRONLY|(os.O_EXCL if exclusive else os.O_TRUNC),0o600)
    with os.fdopen(descriptor,'w') as stream: json.dump(value,stream,indent=2);stream.write('\n')

def pg_tool(tool, database, *, output=None, input_file=None):
    database=identifier(database)
    container=os.environ.get('BACKUP_PG_CONTAINER')
    if container:
        if not re.fullmatch(r'[a-zA-Z0-9_.-]+',container):raise RuntimeError('Invalid PostgreSQL container name')
        # Container-owned credentials remain in the container environment, never argv.
        options='--format=custom --no-owner --no-acl' if tool=='pg_dump' else '--exit-on-error --single-transaction --no-owner --no-acl'
        command=['docker','exec','-i',container,'sh','-c','export PGPASSWORD="$POSTGRES_PASSWORD"; exec '+tool+' --username="$POSTGRES_USER" '+options+' --dbname="$1"','sh',database]
    else:
        command=[tool,'--no-owner','--no-acl','--dbname='+database]
        command+=['--format=custom'] if tool=='pg_dump' else ['--exit-on-error','--single-transaction']
    env=os.environ.copy();parsed=urllib.parse.urlsplit(env['DATABASE_URL']);env.update(PGHOST=parsed.hostname or '',PGPORT=str(parsed.port or 5432),PGUSER=urllib.parse.unquote(parsed.username or ''),PGPASSWORD=urllib.parse.unquote(parsed.password or ''))
    for key,value in urllib.parse.parse_qsl(parsed.query):
        if key in ['sslmode','sslrootcert','sslcert','sslkey']:env['PG'+key.upper()]=value
    result=subprocess.run(command,env=env,stdin=input_file,stdout=output or subprocess.PIPE,stderr=subprocess.PIPE,timeout=300)
    if result.returncode:raise RuntimeError('PostgreSQL backup/restore client failed; use a matching-major client or BACKUP_PG_CONTAINER')
