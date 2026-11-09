#!/usr/bin/env python3
"""Encrypted consistent snapshots and non-destructive isolated restore drills.

Requires requirements-ops.txt and restic. Never restores over an existing DB.
All live application replicas must run the maintenance barrier implementation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
from psycopg import sql
from psycopg.types.json import Jsonb
from ops_common import ROOT, BARRIER, ch, connect, identifier, source_database, private_json, pg_tool

TABLES=['users','organizations','memberships','websites','api_keys','event_outbox','usage_records','subscriptions','goals','campaign_costs','mail_outbox','account_tokens','invitations','_sqlx_migrations','control_migrations']
CH_TABLES=['events','pageviews_hourly']

def file_hash(path):
    with path.open("rb") as stream:return hashlib.file_digest(stream,"sha256").hexdigest()

def restic(*arguments,stdin=None,stdout=None):
    binary=os.environ.get('RESTIC_BIN') or shutil.which('restic') or str(ROOT/'.local/ops-tools/restic')
    result=subprocess.run([binary,*arguments],stdin=stdin,stdout=stdout or subprocess.PIPE,stderr=subprocess.PIPE,timeout=900)
    if result.returncode:raise RuntimeError('Encrypted repository operation failed; verify private repository access and password')
    return result.stdout

def require_repository(allow_local):
    repository=os.environ.get('RESTIC_REPOSITORY','')
    if not repository or not (os.environ.get('RESTIC_PASSWORD') or os.environ.get('RESTIC_PASSWORD_FILE')):raise RuntimeError('Configure RESTIC_REPOSITORY and a privately escrowed repository password')
    if not allow_local and (not repository.startswith(('s3:','sftp:','rest:','rclone:')) or any(host in repository.lower() for host in ['localhost','127.0.0.1','[::1]'])):raise RuntimeError('Off-machine repository required; --allow-local is for explicit local validation')
    scratch=ROOT/'.local/ops-scratch';scratch.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(scratch,0o700)
    return scratch

def record(run,kind,status,details):
    with connect() as db:
        db.execute("INSERT INTO operational_runs(id,kind,status,details,completed_at) VALUES(%s,%s,%s,%s,CASE WHEN %s='running' THEN NULL ELSE now() END) ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,details=EXCLUDED.details,completed_at=EXCLUDED.completed_at",(run,kind,status,Jsonb(details),status))

def control_manifest(database):
    with connect(database) as db:
        counts={table:db.execute(sql.SQL('SELECT count(*) FROM {}').format(sql.Identifier(table))).fetchone()[0] for table in TABLES}
        usage=db.execute('SELECT coalesce(sum(accepted_events),0) FROM usage_records').fetchone()[0]
    return {'rows':counts,'accepted_usage':int(usage)}

def analytics_manifest(database):
    # Per-website logical fingerprints survive background merges and replay.
    queries={
      'events':"SELECT organization_id,website_id,uniqExact(event_id) AS events FROM events GROUP BY organization_id,website_id ORDER BY organization_id,website_id FORMAT JSONEachRow",
      'pageviews':"SELECT organization_id,website_id,uniqExactMerge(views) AS views,uniqExactMerge(visitors) AS visitors,uniqExactMerge(sessions) AS sessions FROM pageviews_hourly GROUP BY organization_id,website_id ORDER BY organization_id,website_id FORMAT JSONEachRow"}
    return {key:[json.loads(line) for line in ch(query,database).decode().splitlines()] for key,query in queries.items()}

def wait_quiet(database):
    until=time.monotonic()+120
    while time.monotonic()<until:
        active=int(ch("SELECT count() FROM system.processes WHERE current_database='"+identifier(database)+"' AND query_kind IN ('Insert','Alter','Delete','Create','Drop','Truncate')",database))
        mutations=int(ch("SELECT count() FROM system.mutations WHERE database='"+identifier(database)+"' AND NOT is_done",database))
        if not active and not mutations:return
        time.sleep(.5)
    raise RuntimeError('ClickHouse writes/mutations did not quiesce; no recovery point created')

def snapshot(allow_local):
    scratch=require_repository(allow_local);run=uuid.uuid4();record(run,'backup','running',{'format':2})
    try:
      with tempfile.TemporaryDirectory(dir=scratch,prefix='snapshot-') as name:
        folder=Path(name);os.chmod(folder,0o700);pg=source_database();click=identifier(os.environ['CLICKHOUSE_DATABASE']);files=[]
        with connect() as barrier:
          barrier.execute("SET LOCAL lock_timeout='60s'");barrier.execute('SELECT pg_advisory_xact_lock(%s)',(BARRIER,));wait_quiet(click)
          with (folder/'postgres.dump').open('wb') as stream:os.chmod(stream.name,0o600);pg_tool('pg_dump',pg,output=stream)
          files.append('postgres.dump')
          table_files={'events':[],'pageviews_hourly':['pageviews_hourly.native']}
          lower=None
          while True:
            params={};condition='1'
            if lower:
              params=dict(zip(['org','site','at','event'],lower));condition="tuple(organization_id,website_id,timestamp,event_id)>tuple({org:String},{site:String},toDateTime({at:String},'UTC'),{event:String})"
            boundary=json.loads(ch("SELECT count() AS rows,max(tuple(organization_id,website_id,timestamp,event_id)) AS last FROM (SELECT organization_id,website_id,timestamp,event_id FROM events WHERE "+condition+" ORDER BY organization_id,website_id,timestamp,event_id LIMIT 10000) FORMAT JSONEachRow",click,params=params))
            if not boundary['rows']:break
            upper=boundary['last'];params.update(dict(zip(['end_org','end_site','end_at','end_event'],upper)))
            upper_condition="tuple(organization_id,website_id,timestamp,event_id)<=tuple({end_org:String},{end_site:String},toDateTime({end_at:String},'UTC'),{end_event:String})"
            file='events-'+str(len(table_files['events'])).zfill(6)+'.native'
            with (folder/file).open('wb') as stream:
              os.chmod(stream.name,0o600);ch('SELECT * FROM events WHERE '+condition+' AND '+upper_condition+' ORDER BY organization_id,website_id,timestamp,event_id SETTINGS max_block_size=2048 FORMAT Native',click,destination=stream,params=params)
            files.append(file);table_files['events'].append(file);lower=upper
          file='pageviews_hourly.native'
          with (folder/file).open('wb') as stream:os.chmod(stream.name,0o600);ch('SELECT * FROM pageviews_hourly SETTINGS max_block_size=2048 FORMAT Native',click,destination=stream)
          files.append(file)
          manifest={'format':2,'table_files':table_files,'run_id':str(run),'source_postgres':pg,'source_clickhouse':click,'created_at':int(time.time()),'control':control_manifest(pg),'analytics':analytics_manifest(click),'geoip_included':False}
          with connect(pg) as control:
            scopes={(str(org),str(site)) for org,site in control.execute('SELECT organization_id,id FROM websites')}
          if any((row['organization_id'],row['website_id']) not in scopes for rows in manifest['analytics'].values() for row in rows):raise RuntimeError('The two configured databases do not share website ownership; refusing an inconsistent recovery point')
          source=Path(os.environ.get('BACKUP_SECRETS_FILE',ROOT/'.env')).resolve()
          if not source.is_file() or source.stat().st_mode&0o077:raise RuntimeError('BACKUP_SECRETS_FILE must be an existing private configuration file (0600)')
          shutil.copyfile(source,folder/'configuration.env');os.chmod(folder/'configuration.env',0o600);files.append('configuration.env')
          connections_dir=Path(os.environ.get('CONNECTIONS_DATA_DIR',ROOT/'.local/connections'))
          key_path=connections_dir/'key'
          key=bytes.fromhex(os.environ['CONNECTIONS_KEY']) if os.environ.get('CONNECTIONS_KEY') else (key_path.read_bytes() if key_path.exists() else None)
          with connect(pg) as control:
            has_connections=control.execute("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='platform_connections')").fetchone()[0]
            needs_key=has_connections and control.execute('SELECT EXISTS(SELECT 1 FROM platform_connections)').fetchone()[0]
          if needs_key and (key is None or len(key)!=32):raise RuntimeError('Dashboard connections require their matching private encryption key in the backup')
          if key is not None:
            if len(key)!=32:raise RuntimeError('Invalid dashboard connection encryption key')
            (folder/'connections.key').write_bytes(key);os.chmod(folder/'connections.key',0o600);files.append('connections.key');manifest['connections_key_included']=True
          geoip=connections_dir/'GeoLite2-Country.mmdb'
          if not geoip.is_file():geoip=os.environ.get('GEOIP_PATH')
          if geoip:
            shutil.copyfile(geoip,folder/'country.mmdb');os.chmod(folder/'country.mmdb',0o600);files.append('country.mmdb');manifest['geoip_included']=True
          manifest['sha256']={file:file_hash(folder/file) for file in files}
          private_json(folder/'manifest.json',manifest);files.append('manifest.json')
        # The freeze ends before packaging, encryption and off-site transfer.
        bundle=folder/'recovery.tar'
        with tarfile.open(bundle,'w') as archive:
          os.chmod(bundle,0o600)
          for file in files:archive.add(folder/file,arcname=file,recursive=False)
        with bundle.open('rb') as stream:raw=restic('backup','--stdin','--stdin-filename','recovery.tar','--tag','vuuut-click','--tag',str(run),'--json',stdin=stream)
        summaries=[json.loads(line) for line in raw.decode().splitlines() if line.strip()];summary=next(item for item in summaries if item.get('message_type')=='summary');snapshot_id=summary['snapshot_id']
        restic('check')
        record(run,'backup','succeeded',{'snapshot_id':snapshot_id,'format':2,'local_validation':allow_local})
        print(json.dumps({'backup_run':str(run),'snapshot_id':snapshot_id,'encrypted_repository_checked':True,'local_validation':allow_local}))
    except BaseException:
      record(run,'backup','failed',{'error':'backup_failed'});raise

def verify_files(folder):
    manifest=json.loads((folder/'manifest.json').read_text())
    if manifest.get('format') not in [1,2]:raise RuntimeError('Unsupported recovery format')
    expected={'postgres.dump','configuration.env'}
    if manifest['format']==1:expected.update(['events.native','pageviews_hourly.native'])
    else:
      import re
      tables=manifest.get('table_files',{})
      if set(tables)!=set(CH_TABLES):raise RuntimeError('Missing recovery table mapping')
      for table,names in tables.items():
        if len(names)!=len(set(names)) or any(not (re.fullmatch(r'events-[0-9]{6}\.native',name) if table=='events' else name=='pageviews_hourly.native') for name in names):raise RuntimeError('Invalid recovery chunk mapping')
        expected.update(names)
    if manifest.get('geoip_included'):expected.add('country.mmdb')
    if manifest.get('connections_key_included'):expected.add('connections.key')
    if set(manifest['sha256'])!=expected:raise RuntimeError('Recovery file manifest is incomplete')
    for file in expected:
      if file_hash(folder/file)!=manifest['sha256'][file]:raise RuntimeError('Recovery artifact integrity mismatch')
    return manifest

def restore(snapshot_id,allow_local,keep):
    started=time.monotonic()
    if snapshot_id!="latest" and not __import__("re").fullmatch(r"[a-f0-9]{8,64}",snapshot_id):raise RuntimeError("Invalid snapshot identifier")
    scratch=require_repository(allow_local);run=uuid.uuid4();target='click_restore_'+uuid.uuid4().hex;identifier(target);created_pg=created_ch=False;record(run,'restore','running',{'format':1})
    try:
      with tempfile.TemporaryDirectory(dir=scratch,prefix='restore-') as name:
        folder=Path(name);os.chmod(folder,0o700)
        with (folder/'recovery.tar').open('wb') as stream:os.chmod(stream.name,0o600);restic('dump',snapshot_id,'recovery.tar',stdout=stream)
        with tarfile.open(folder/'recovery.tar','r') as archive:
          names=set()
          for member in archive:
            if not member.isfile() or not (member.name in {'manifest.json','postgres.dump','events.native','pageviews_hourly.native','configuration.env','country.mmdb','connections.key'} or __import__('re').fullmatch(r'events-[0-9]{6}\.native',member.name)) or member.name in names:raise RuntimeError('Unsafe or duplicate recovery archive member')
            names.add(member.name)
            with (folder/member.name).open('wb') as out:os.chmod(out.name,0o600);shutil.copyfileobj(archive.extractfile(member),out)
        manifest=verify_files(folder)
        with connect(autocommit=True) as db:
          if db.execute('SELECT 1 FROM pg_database WHERE datname=%s',(target,)).fetchone():raise RuntimeError('Restore target already exists')
          db.execute(sql.SQL('CREATE DATABASE {}').format(sql.Identifier(target)));created_pg=True
        ch('CREATE DATABASE '+target);created_ch=True
        with (folder/'postgres.dump').open('rb') as stream:pg_tool('pg_restore',target,input_file=stream)
        # Restore the exact schema; detach the MV while loading raw/aggregate data.
        for path in sorted((ROOT/'migrations/clickhouse').glob('*.sql')):
          for statement in path.read_text().split(';'):
            if statement.strip():ch(statement,target)
        ch('DETACH TABLE pageviews_hourly_mv',target)
        try:
          for table in CH_TABLES:
            names=manifest.get('table_files',{}).get(table,[table+'.native'])
            for name in names:
              path=folder/name
              if path.stat().st_size:
                with path.open('rb') as stream:ch('INSERT INTO '+table+' FORMAT Native',target,payload=stream,payload_size=path.stat().st_size)
        finally:ch('ATTACH TABLE pageviews_hourly_mv',target)
        assert control_manifest(target)==manifest['control'],'Control recovery counts/usage mismatch'
        assert analytics_manifest(target)==manifest['analytics'],'Analytics recovery fingerprint mismatch'
        # Volatile coordination must not survive a recovery point.
        with connect(target) as db:db.execute('TRUNCATE request_budgets,report_leases')
        details={'snapshot_id':snapshot_id,'postgres_and_clickhouse_verified':True,'configuration_integrity_verified':True,'geoip_included':manifest['geoip_included'],'isolated_target':target,'local_validation':allow_local,'restore_seconds':time.monotonic()-started}
        if keep:
          result=ROOT/'.local/ops';result.mkdir(mode=0o700,exist_ok=True);private_json(result/(target+'.json'),{'database':target,'snapshot_id':snapshot_id,'source_run':manifest['run_id']},exclusive=True)
        record(run,'restore','succeeded',details);print(json.dumps(details))
    except BaseException:
      record(run,'restore','failed',{'error':'restore_failed'});raise
    finally:
      if not keep or sys.exc_info()[0] is not None:
        if created_ch:ch('DROP DATABASE '+target)
        if created_pg:
          with connect(autocommit=True) as db:db.execute(sql.SQL('DROP DATABASE {} WITH (FORCE)').format(sql.Identifier(target)))

def main():
    os.umask(0o077);parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--allow-local',action='store_true');sub=parser.add_subparsers(dest='command',required=True);sub.add_parser('init');sub.add_parser('snapshot');restore_parser=sub.add_parser('restore-drill');restore_parser.add_argument('snapshot');restore_parser.add_argument('--keep-isolated',action='store_true');sub.add_parser('check');retention=sub.add_parser('retention');retention.add_argument('--apply',action='store_true');args=parser.parse_args()
    if args.command=='init':require_repository(args.allow_local);restic('init');print('Encrypted repository initialized; escrow the password independently.')
    elif args.command=='snapshot':snapshot(args.allow_local)
    elif args.command=='restore-drill':restore(args.snapshot,args.allow_local,args.keep_isolated)
    elif args.command=='retention':
        require_repository(args.allow_local)
        command=['forget','--tag','vuuut-click','--group-by','host','--keep-daily','7','--keep-weekly','4','--keep-monthly','12','--json']
        command+=['--prune'] if args.apply else ['--dry-run']
        report=json.loads(restic(*command));print(json.dumps({'dry_run':not args.apply,'snapshots_selected_for_removal':sum(len(group.get('remove',[])) for group in report)}))
    else:require_repository(args.allow_local);restic('check','--read-data');print('Encrypted repository and stored data integrity verified.')
if __name__=='__main__':
    try:main()
    except Exception:
      print('Operational backup/restore failed. No credentials or raw provider/database errors were printed. Inspect configuration, matching client versions and operational_runs status.',file=sys.stderr);sys.exit(1)
