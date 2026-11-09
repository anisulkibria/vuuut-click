#!/usr/bin/env python3
"""Prepare private LOCAL operation keys; never provision external infrastructure."""
import os
from pathlib import Path
import secrets
import shlex
ROOT=Path(__file__).resolve().parent.parent

def prepare():
    os.umask(0o077);environment=ROOT/'.env'
    if not environment.is_file():raise SystemExit('Create the trusted private .env with run.sh first')
    before=environment.read_text();keys={line.split('=',1)[0] for line in before.splitlines() if '=' in line and not line.startswith('#')};directory=ROOT/'.local/ops';directory.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(directory,0o700)
    additions={'DISTRIBUTED_LIMITS':'true','ABUSE_HASH_KEY':secrets.token_hex(32),'METRICS_BEARER_TOKEN':secrets.token_urlsafe(48),'OTEL_EXPORTER_OTLP_ENDPOINT':'http://127.0.0.1:4318','BACKUP_PG_CONTAINER':'vuuut-click-dev-postgres-1'}
    missing={name:value for name,value in additions.items() if name not in keys}
    if missing:
        temporary=directory/'environment.tmp';fd=os.open(temporary,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
        with os.fdopen(fd,'w') as stream:stream.write(before+'\n# Local Priority 5 shared budgets and private metrics. Never commit.\n'+''.join(name+'='+shlex.quote(value)+'\n' for name,value in missing.items()))
        os.replace(temporary,environment);os.chmod(environment,0o600)
    print('Private local operational keys prepared. Source .env; no values printed.')
    print('Configure off-machine RESTIC_REPOSITORY/password escrow and production alert receiver separately.')
if __name__=='__main__':prepare()
