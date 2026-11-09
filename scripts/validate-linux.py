#!/usr/bin/env python3
"""Local Linux/non-root/private-CA TLS smoke test in disposable owned stores.

Uses the test MMDB solely for this fixture. It does not establish public DNS,
public certificate issuance, real SMTP or licensed production GeoIP readiness.
"""
import argparse
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from psycopg import sql
from ops_common import ROOT,ch,connect,private_json

def main():
    os.umask(0o077);parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--image',default='vuuut-click:priority5-local');args=parser.parse_args()
    parsed=urllib.parse.urlsplit(os.environ['DATABASE_URL'])
    if os.environ.get('APP_ENV')!='development' or parsed.hostname not in ['localhost','127.0.0.1']:raise RuntimeError('This smoke test targets local development stores only')
    for port in [8445,18085]:
        with socket.socket() as check:check.bind(('127.0.0.1',port))
    suffix=uuid.uuid4().hex;database='click_ops_linux_'+suffix;project='click-linux-'+suffix;folder=ROOT/'.local/ops'/project;folder.mkdir(mode=0o700,parents=True);compose=folder/'compose.yml';created_pg=created_ch=False;started=False;evidence={'scope':'local Linux and private-CA TLS only','image':args.image}
    def docker(*command,check=True):
        result=subprocess.run(['docker',*command],capture_output=True,timeout=300)
        if check and result.returncode:raise RuntimeError('Owned Linux fixture Docker operation failed')
        return result
    def dc(*command,check=True):return docker('compose','-p',project,'-f',str(compose),*command,check=check)
    try:
        with connect(autocommit=True) as db:db.execute(sql.SQL('CREATE DATABASE {}').format(sql.Identifier(database)));created_pg=True
        ch('CREATE DATABASE '+database);created_ch=True
        host_env={k:v for k,v in os.environ.items() if not k.startswith(('STRIPE_','SMTP_','COMMERCIAL_','OTEL_'))};host_env.update(DATABASE_URL=urllib.parse.urlunsplit(parsed._replace(path='/'+database)),CLICKHOUSE_DATABASE=database,APP_ENV='development',ALLOW_LOCALHOST='true',REQUIRE_GEOIP='false',PUBLIC_URL='http://localhost:18085');host_env.pop('GEOIP_PATH',None)
        migrated=subprocess.run([str(ROOT/'target/debug/click-server'),'migrate'],env=host_env,capture_output=True,timeout=120)
        if migrated.returncode:raise RuntimeError('Fixture migration failed')
        probe=docker('network','inspect','vuuut-click-dev_default','--format','{{.Name}}').stdout.decode().strip()
        if probe!='vuuut-click-dev_default':raise RuntimeError('Expected owned local database network')
        # Docker picks a free network; use only this Caddy network as a proxy trust source.
        network=project+'-edge';docker('network','create',network);subnet=docker('network','inspect',network,'--format','{{(index .IPAM.Config 0).Subnet}}').stdout.decode().strip()
        userinfo=parsed.netloc.rsplit('@',1)[0];url=urllib.parse.urlunsplit(parsed._replace(netloc=userinfo+'@postgres:5432',path='/'+database));token=secrets.token_urlsafe(40);key=secrets.token_hex(32)
        environment={'APP_ENV':'production','BIND':'0.0.0.0:8082','PUBLIC_URL':'https://click.localhost:8445','ALLOW_LOCALHOST':'false','DATABASE_URL':url,'CLICKHOUSE_URL':'http://clickhouse:8123','CLICKHOUSE_DATABASE':database,'CLICKHOUSE_USER':os.environ['CLICKHOUSE_USER'],'CLICKHOUSE_PASSWORD':os.environ['CLICKHOUSE_PASSWORD'],'GEOIP_PATH':'/fixtures/country.mmdb','TRUSTED_PROXIES':subnet,'DISTRIBUTED_LIMITS':'true','ABUSE_HASH_KEY':key,'METRICS_BEARER_TOKEN':token,'RUN_WORKERS':'false','SMTP_HOST':'mail.invalid','SMTP_PORT':'465','SMTP_FROM':'test@operations.invalid','RUST_LOG':'click_server=warn'}
        envfile=folder/'runtime.env';envfile.write_text('\n'.join(k+'='+v for k,v in environment.items())+'\n');os.chmod(envfile,0o600)
        (folder/'Caddyfile').write_text('click.localhost {\n tls internal\n @private path /readyz /healthz /internal/*\n respond @private 404\n reverse_proxy app:8082\n}\n')
        configuration={'services':{'app':{'image':args.image,'env_file':[str(envfile)],'ports':['127.0.0.1:18085:8082'],'volumes':[str(ROOT/'tests/fixtures/GeoIP2-Country-Test.mmdb')+':/fixtures/country.mmdb:ro'],'read_only':True,'tmpfs':['/tmp:size=32m'],'security_opt':['no-new-privileges:true'],'cap_drop':['ALL'],'mem_limit':'768m','networks':['default','databases']},'caddy':{'image':'caddy:2.10.2-alpine','ports':['127.0.0.1:8445:443'],'volumes':[str(folder/'Caddyfile')+':/etc/caddy/Caddyfile:ro','caddy_data:/data','caddy_config:/config'],'mem_limit':'128m','networks':['default']}},'volumes':{'caddy_data':{},'caddy_config':{}},'networks':{'default':{'external':True,'name':network},'databases':{'external':True,'name':'vuuut-click-dev_default'}}}
        # Compose accepts JSON, avoiding another operator dependency for YAML.
        compose.write_text(json.dumps(configuration));dc('up','-d');started=True
        for _ in range(150):
            try:
                with urllib.request.urlopen('http://localhost:18085/readyz',timeout=1) as response:
                    if response.status==200:break
            except OSError:pass
            time.sleep(.2)
        else:raise RuntimeError('Linux image readiness failed')
        caddy_id=dc('ps','-q','caddy').stdout.decode().strip();app_id=dc('ps','-q','app').stdout.decode().strip();certificate=folder/'local-root.crt'
        for _ in range(50):
            copied=docker('cp',caddy_id+':/data/caddy/pki/authorities/local/root.crt',str(certificate),check=False)
            if copied.returncode==0:break
            time.sleep(.2)
        else:raise RuntimeError('Local certificate authority unavailable')
        password=secrets.token_urlsafe(32);cookies=folder/'cookies';headers=folder/'headers';bodyfile=folder/'body'
        def https(path,payload=None,origin='https://click.localhost:8445',extra=None):
            command=['curl','--silent','--show-error','--noproxy','*','--resolve','click.localhost:8445:127.0.0.1','--cacert',str(certificate),'--cookie',str(cookies),'--cookie-jar',str(cookies),'--dump-header',str(headers),'--output',str(bodyfile),'--write-out','%{http_code}','--header','Origin: '+origin]
            if extra:command+=['--header',extra]
            if payload is not None:command+=['--header','Content-Type: application/json','--data-binary','@-']
            command+=['https://click.localhost:8445'+path]
            result=subprocess.run(command,input=json.dumps(payload).encode() if payload is not None else None,capture_output=True,timeout=30)
            if result.returncode:raise RuntimeError('Trusted local TLS request failed')
            return int(result.stdout)
        assert https('/signup')==200;evidence['private_ca_tls_verified']=True
        registration_status=https('/v1/auth/register',{'email':'linux@operations.invalid','password':password})
        if registration_status not in [200,201]:raise RuntimeError('TLS registration returned '+str(registration_status))
        raw=headers.read_text().lower();assert 'secure' in raw and 'httponly' in raw and 'samesite=strict' in raw and 'strict-transport-security' in raw and 'content-security-policy' in raw;evidence['secure_cookies_hsts_csp']=True
        assert https('/v1/me')==200
        assert https('/v1/organizations',{'name':'Cross-origin refused'},origin='https://untrusted.invalid')==403;evidence['csrf_rejected']=True
        for path in ['/internal/metrics','/healthz','/readyz']:assert https(path)==404
        evidence['internal_endpoints_hidden_at_edge']=True
        request=urllib.request.Request('http://localhost:18085/internal/metrics',headers={'Authorization':'Bearer '+token})
        with urllib.request.urlopen(request,timeout=8) as response:assert response.status==200 and b'click_dependency_up' in response.read()
        assert https('/signup',extra='X-Forwarded-For: 203.0.113.77')==200
        with connect(database) as db:
            bucket=db.execute('SELECT floor(extract(epoch FROM clock_timestamp())/60)::bigint').fetchone()[0]
            forged=hmac.new(bytes.fromhex(key),bucket.to_bytes(8,'big',signed=True)+b'203.0.113.77',hashlib.sha256).hexdigest();assert not db.execute("SELECT 1 FROM request_budgets WHERE scope='ip' AND subject=%s",(forged,)).fetchone()
        evidence['untrusted_forwarding_spoof_rejected']=True
        uid=docker('inspect',app_id,'--format','{{.Config.User}}').stdout.decode().strip();assert uid=='10001:10001';evidence['runtime_uid']=uid
        readonly=docker('inspect',app_id,'--format','{{.HostConfig.ReadonlyRootfs}}').stdout.decode().strip();assert readonly=='true';evidence['read_only_root']=True
        evidence['image_id']=docker('image','inspect',args.image,'--format','{{.Id}}').stdout.decode().strip();private_json(ROOT/'.local/ops/linux-evidence.json',evidence);print(json.dumps(evidence,indent=2))
    finally:
        if compose.exists():
            logs=dc('logs','--no-color',check=False);(folder/'containers.log').write_bytes(logs.stdout+logs.stderr)
            dc('down','--volumes',check=False)
        docker('network','rm',project+'-edge',check=False)
        if created_ch:ch('DROP DATABASE IF EXISTS '+database)
        if created_pg:
            with connect(autocommit=True) as db:db.execute(sql.SQL('DROP DATABASE IF EXISTS {} WITH (FORCE)').format(sql.Identifier(database)))
        for name in ['runtime.env','cookies','headers','body']:(folder/name).unlink(missing_ok=True)
if __name__=='__main__':
    try:main()
    except Exception:
        print('Owned Linux/TLS fixture failed; private container logs retained, original services unchanged.',file=sys.stderr);sys.exit(1)
