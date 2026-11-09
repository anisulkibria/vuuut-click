#!/usr/bin/env python3
"""Exercise isolated recovery, real OTLP privacy and monthly arrival-rate profiles.

This is local validation, not off-site storage, independent review or a claim
that a 100M-row production deployment is qualified. Source the private env and
use the operator venv. Every store/server here belongs to a generated namespace.
"""
import argparse
import concurrent.futures
import http.cookiejar
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
import math
import os
from pathlib import Path
import secrets
import shlex
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from psycopg import sql
from ops_common import ROOT,BARRIER,ch,connect,private_json

BINARY=ROOT/'target/debug/click-server'
class Collector(BaseHTTPRequestHandler):
    spans=[]
    def log_message(self,*args):pass
    def do_POST(self):
        payload=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))));self.spans.append(payload);self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(b'{}')

def api(client,base,path,body=None,origin=None):
    request=urllib.request.Request(base+path,data=json.dumps(body).encode() if body is not None else None,headers={'Origin':origin or base,'Content-Type':'application/json'})
    with client.open(request,timeout=35) as response:
        raw=response.read();return json.loads(raw) if raw and response.headers.get_content_type()=='application/json' else raw

def drain(env):
    for _ in range(500):
        result=subprocess.run([str(BINARY),'drain-once'],env=env,capture_output=True,timeout=60)
        if result.returncode:raise RuntimeError('Isolated replay worker failed')
        if result.stdout.strip().splitlines()[-1]==b'0':return
    raise RuntimeError('Isolated replay exceeded its bounded drain limit')

def main():
    os.umask(0o077);parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--duration',type=int,default=20);parser.add_argument('--volume-rows',type=int,default=1_000_000);parser.add_argument('--port',type=int,default=8090);args=parser.parse_args()
    if os.environ.get('APP_ENV')!='development':raise RuntimeError('Local operational validation requires APP_ENV=development')
    if not 5<=args.duration<=600 or not 0<=args.volume_rows<=100_000_000:raise RuntimeError('Invalid bounded benchmark duration/volume')
    name='click_ops_'+uuid.uuid4().hex;base='http://localhost:'+str(args.port);folder=ROOT/'.local/ops'/name;folder.mkdir(mode=0o700,parents=True);owned=[];created_pg=created_ch=False;restored=None;evidence={'local_validation':True,'monthly_profiles':[],'resident_rows_requested':args.volume_rows};collector=None
    for port in [args.port,args.port+1,args.port+2]:
        with socket.socket() as check:check.bind(('127.0.0.1',port))
    try:
        with connect(autocommit=True) as db:db.execute(sql.SQL('CREATE DATABASE {}').format(sql.Identifier(name)));created_pg=True
        ch('CREATE DATABASE '+name);created_ch=True
        env={key:value for key,value in os.environ.items() if not key.startswith(('STRIPE_','SMTP_','COMMERCIAL_','OTEL_'))};parsed=urllib.parse.urlsplit(env['DATABASE_URL']);env.update(DATABASE_URL=urllib.parse.urlunsplit(parsed._replace(path='/'+name)),CLICKHOUSE_DATABASE=name,APP_ENV='development',ALLOW_LOCALHOST='true',REQUIRE_GEOIP='false',PUBLIC_URL=base,BIND='127.0.0.1:'+str(args.port),RUN_WORKERS='false',DISTRIBUTED_LIMITS='true',ABUSE_HASH_KEY=secrets.token_hex(32),METRICS_BEARER_TOKEN=secrets.token_urlsafe(32),OTEL_EXPORTER_OTLP_ENDPOINT='http://127.0.0.1:'+str(args.port+2),OTEL_TRACES_SAMPLER_ARG='1',RUST_LOG='click_server=warn')
        env.pop('GEOIP_PATH',None);password=secrets.token_urlsafe(32);envfile=folder/'configuration.env';envfile.write_text('\n'.join(key+'='+shlex.quote(value) for key,value in env.items() if key in ['DATABASE_URL','CLICKHOUSE_URL','CLICKHOUSE_USER','CLICKHOUSE_PASSWORD','CLICKHOUSE_DATABASE','APP_ENV','PUBLIC_URL','ALLOW_LOCALHOST','ABUSE_HASH_KEY','METRICS_BEARER_TOKEN'])+'\n');os.chmod(envfile,0o600);env['BACKUP_SECRETS_FILE']=str(envfile)
        repository=folder/'encrypted-repository';keyfile=folder/'restic-password';keyfile.write_text(secrets.token_urlsafe(48));os.chmod(keyfile,0o600);env.update(RESTIC_REPOSITORY=str(repository),RESTIC_PASSWORD_FILE=str(keyfile),RESTIC_BIN=str(ROOT/'.local/ops-tools/restic'))
        collector=ThreadingHTTPServer(('127.0.0.1',args.port+2),Collector);threading.Thread(target=collector.serve_forever,daemon=True).start()
        for offset in [0,1]:
            child_env=dict(env,BIND='127.0.0.1:'+str(args.port+offset),PUBLIC_URL=base)
            with (folder/('server-'+str(offset)+'.log')).open('wb') as log:process=subprocess.Popen([str(BINARY),'serve'],env=child_env,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            owned.append(process)
            for _ in range(150):
                if process.poll() is not None:raise RuntimeError('Isolated operational app failed to start; private log retained')
                try:
                    with urllib.request.urlopen('http://localhost:'+str(args.port+offset)+'/readyz',timeout=1) as response:
                        if response.status==200:break
                except OSError:pass
                time.sleep(.2)
            else:raise RuntimeError('Isolated readiness timed out')
        client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()));api(client,base,'/v1/auth/register',{'email':'recovery@operations.invalid','password':password});org=api(client,base,'/v1/organizations')[0]['id'];site=api(client,base,'/v1/websites',{'organization_id':org,'name':'Operational recovery fixture','origin':base})
        events=[{'event_id':str(uuid.uuid4()),'session_id':str(uuid.uuid4()),'visitor_id':None,'name':'pageview','url':base+'/recovery'} for _ in range(3)];api(client,base,'/v1/collect/'+site['collection_id'],{'events':events})
        with connect(name) as db:
            rows=db.execute("SELECT event_id,payload FROM event_outbox ORDER BY event_id").fetchall()
            for index,(event_id,payload) in enumerate(rows[:2]):
                ch('INSERT INTO events FORMAT JSONEachRow',name,payload=json.dumps(payload).encode())
                if index==0:db.execute("UPDATE event_outbox SET state='delivered',delivered_at=now(),payload=NULL WHERE event_id=%s",(event_id,))
        # Freeze rejects mutations rather than accepting work into a torn snapshot.
        with connect(name) as freeze:
            freeze.execute('SELECT pg_advisory_xact_lock(%s)',(BARRIER,))
            try:api(client,base,'/v1/organizations',{'name':'Must be refused'});raise AssertionError('Write was admitted during snapshot barrier')
            except urllib.error.HTTPError as error:assert error.code==503
        evidence['maintenance_refused_writes']=True
        def backup(*command,custom=None,allow_failure=False):
            result=subprocess.run([sys.executable,str(ROOT/'scripts/backup.py'),'--allow-local',*command],env=custom or env,capture_output=True,timeout=900)
            if result.returncode and not allow_failure:raise RuntimeError('Backup/restore drill failed; command='+command[0])
            return result
        backup('init');snapshot=json.loads(backup('snapshot').stdout)['snapshot_id'];restoration=json.loads(backup('restore-drill',snapshot,'--keep-isolated').stdout);restored=restoration['isolated_target'];evidence['restore_seconds']=restoration['restore_seconds'];assert restored.startswith('click_restore_')
        restored_env=dict(env,DATABASE_URL=urllib.parse.urlunsplit(parsed._replace(path='/'+restored)),CLICKHOUSE_DATABASE=restored)
        with connect(restored) as db:before=db.execute('SELECT sum(accepted_events) FROM usage_records').fetchone()[0]
        drain(restored_env)
        with connect(restored) as db:after=db.execute('SELECT sum(accepted_events) FROM usage_records').fetchone()[0];pending=db.execute("SELECT count(*) FROM event_outbox WHERE state='pending'").fetchone()[0]
        raw=int(ch('SELECT uniqExact(event_id) FROM events',restored));views=int(ch('SELECT uniqExactMerge(views) FROM pageviews_hourly',restored));assert before==after==3 and raw==views==3 and pending==0
        evidence['consistent_recovery_and_replay']={'control_usage_unchanged':True,'logical_events_after_replay':raw,'logical_pageviews_after_replay':views,'pending_after_replay':pending,'secrets_and_both_stores_verified':True}
        assert backup('check',custom=dict(env,RESTIC_PASSWORD='invalid-password-fixture',RESTIC_PASSWORD_FILE=''),allow_failure=True).returncode!=0;evidence['wrong_key_rejected']=True
        # Corrupt only an owned local pack, prove authentication failure, restore it.
        pack=next((repository/'data').glob('*/*'));original=pack.read_bytes();original_mode=pack.stat().st_mode&0o777;os.chmod(pack,original_mode|0o200);damaged=bytearray(original);damaged[len(damaged)//2]^=1;pack.write_bytes(damaged)
        try:assert backup('check',allow_failure=True).returncode!=0;evidence['encrypted_pack_tamper_rejected']=True
        finally:pack.write_bytes(original);os.chmod(pack,original_mode)
        backup('check');drain(env)
        # A separate marked synthetic site measures resident cardinality, never billing.
        volume_id=str(uuid.uuid4())
        with connect(name) as db:db.execute("INSERT INTO websites(id,organization_id,name,origin,collection_id,verification_token,verified_at,is_demo) VALUES(%s,%s,'Synthetic capacity fixture','https://capacity.invalid',%s,%s,now(),true)",(volume_id,org,'ops_'+secrets.token_hex(24),secrets.token_hex(32)))
        if args.volume_rows:
            query="INSERT INTO events(organization_id,website_id,event_id,timestamp,session_id,visitor_id,name,path,country,referrer,utm_source,utm_medium,utm_campaign,device,browser) SELECT '"+org+"','"+volume_id+"',concat('volume-',toString(number)),now()-toIntervalSecond(number%86400),concat('session-',toString(intDiv(number,5))),concat('visitor-',toString(intDiv(number,10))),'pageview',concat('/page/',toString(number%100)),'ZZ','','','','','desktop','other' FROM numbers("+str(args.volume_rows)+")"
            start=time.monotonic()
            for offset in range(0,args.volume_rows,100000):ch(query.replace('numbers('+str(args.volume_rows)+')','numbers('+str(offset)+','+str(min(100000,args.volume_rows-offset))+')'),name)
            evidence['resident_seed_seconds']=time.monotonic()-start
        report_times=[]
        for _ in range(5):
            start=time.monotonic();report=api(client,base,'/v1/websites/'+volume_id+'/overview');report_times.append(time.monotonic()-start);assert report['pageviews']==args.volume_rows
        evidence['resident_overview_seconds']={'samples':len(report_times),'max':max(report_times),'median':sorted(report_times)[len(report_times)//2]}
        try:api(client,base,'/unknown-private-path?token=must-never-appear')
        except urllib.error.HTTPError:pass
        total_generated=0
        for monthly in [1_000_000,10_000_000,100_000_000]:
            rate=monthly/(30*86400)*5;batch=max(1,min(50,math.ceil(rate/5)));requests=max(5,int(args.duration*rate/batch));latencies=[];accepted=0;start=time.monotonic()
            def send_batch(index):
                payload=[{'event_id':str(uuid.uuid4()),'session_id':str(uuid.uuid4()),'visitor_id':None,'name':'pageview','url':base+'/arrival-profile'} for _ in range(batch)]
                target='http://localhost:'+str(args.port+index%2);request_start=time.monotonic()
                response=api(urllib.request.build_opener(),target,'/v1/collect/'+site['collection_id'],{'events':payload},origin=base)
                return response['accepted'],time.monotonic()-request_start
            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
                futures=[]
                for index in range(requests):
                    scheduled=start+index*batch/rate
                    if scheduled>time.monotonic():time.sleep(scheduled-time.monotonic())
                    futures.append(executor.submit(send_batch,index))
                for future in futures:
                    count,latency=future.result();accepted+=count;latencies.append(latency)
            total_generated+=accepted;elapsed=time.monotonic()-start;private_json(folder/'last-profile.json',{'monthly':monthly,'elapsed':elapsed,'latencies':latencies,'accepted':accepted});expected_window=(requests-1)*batch/rate;assert elapsed<=expected_window+max(2,expected_window*.1),'Arrival profile fell behind its offered rate';assert sorted(latencies)[int((len(latencies)-1)*.95)]<1,'Request p95 exceeded one second';delivery_started=time.monotonic();drain(env);assert time.monotonic()-delivery_started<30,'Delivery exceeded 30 seconds'
            evidence['monthly_profiles'].append({'monthly_arrival_equivalent':monthly,'burst_multiplier':5,'batch_size':batch,'requests':requests,'events_accepted':accepted,'elapsed_seconds':elapsed,'request_p95_seconds':sorted(latencies)[int((len(latencies)-1)*.95)],'resident_dataset_rows':args.volume_rows})
        with connect(name) as db:usage=int(db.execute('SELECT sum(accepted_events) FROM usage_records').fetchone()[0]);pending=db.execute("SELECT count(*) FROM event_outbox WHERE state='pending'").fetchone()[0]
        assert usage==3+total_generated and pending==0;evidence['metering_and_delivery_exact']=True
        # Graceful shutdown flushes the actual OTLP exporter.
        for process in owned:process.terminate();process.wait(timeout=45)
        owned=[];serialized=json.dumps(Collector.spans);assert Collector.spans and 'must-never-appear' not in serialized and 'unknown-private-path' not in serialized and password not in serialized
        evidence['real_otlp_spans_received']=sum(len(scope.get('spans',[])) for item in Collector.spans for resource in item.get('resourceSpans',[]) for scope in resource.get('scopeSpans',[]));evidence['trace_privacy_verified']=True;evidence['observed_at_epoch']=int(time.time());private_json(ROOT/'.local/ops/validation-evidence.json',evidence);print(json.dumps(evidence,indent=2))
    finally:
        for process in owned:
            if process.poll() is None:process.terminate();process.wait(timeout=45)
        if collector:collector.shutdown();collector.server_close()
        if restored:
            ch('DROP DATABASE IF EXISTS '+restored)
            with connect(autocommit=True) as db:db.execute(sql.SQL('DROP DATABASE IF EXISTS {} WITH (FORCE)').format(sql.Identifier(restored)))
            (ROOT/'.local/ops'/(restored+'.json')).unlink(missing_ok=True)
        if created_ch:ch('DROP DATABASE IF EXISTS '+name)
        if created_pg:
            with connect(autocommit=True) as db:db.execute(sql.SQL('DROP DATABASE IF EXISTS {} WITH (FORCE)').format(sql.Identifier(name)))
        for private in ['configuration.env','restic-password']:(folder/private).unlink(missing_ok=True)
if __name__=='__main__':
    try:main()
    except Exception:
        print('Isolated operational validation failed. No credentials or raw data were printed; inspect the private fixture logs and rerun after resolving the failed gate.',file=sys.stderr);sys.exit(1)
