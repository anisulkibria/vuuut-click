#!/usr/bin/env python3
"""Start private local monitoring; optional proof alert only to our loopback inbox."""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
import urllib.request
ROOT=Path(__file__).resolve().parent.parent
PROJECT='vuuut-click-ops-local'

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--proof-alert',action='store_true');parser.add_argument('--no-grafana',action='store_true');parser.add_argument('--stop',action='store_true');args=parser.parse_args();directory=ROOT/'.local/ops';directory.mkdir(mode=0o700,parents=True,exist_ok=True);os.chmod(directory,0o700);secret_dir=directory/'secrets';secret_dir.mkdir(mode=0o700,exist_ok=True);os.chmod(secret_dir,0o700);override=directory/'compose.local.json';pidfile=directory/'alert-inbox-process.json'
    env=dict(os.environ,OPS_SECRETS_DIR=str(secret_dir));command=['docker','compose','-p',PROJECT,'-f',str(ROOT/'deploy/compose.observability.yml'),'-f',str(override)]
    if args.stop:
        if override.exists():subprocess.run([*command,'stop'],env=env,check=True)
        if pidfile.exists():
            process=json.loads(pidfile.read_text());result=subprocess.run(['ps','-p',str(process['pid']),'-o','command='],capture_output=True,text=True)
            if result.returncode==0 and 'scripts/alert-inbox.py' in result.stdout and subprocess.check_output(['ps','-p',str(process['pid']),'-o','lstart='],text=True).strip()==process['started']:os.kill(process['pid'],signal.SIGTERM)
            pidfile.unlink()
        return
    if os.environ.get('APP_ENV')!='development' or not os.environ.get('METRICS_BEARER_TOKEN'):raise SystemExit('Source the private development .env after setup-operations.py')
    # Files must be readable by container UIDs; their host parent remains0700.
    for name,value in [('metrics_token',env['METRICS_BEARER_TOKEN']),('grafana_password',secrets.token_urlsafe(48))]:
        path=secret_dir/name
        if not path.exists():path.write_text(value);os.chmod(path,0o444)
    for existing in ['prometheus.local.yml','alerts.local.yml','alertmanager.local.yml']:
        path=directory/existing
        if path.exists():os.chmod(path,0o600)
    base=(ROOT/'deploy/observability/prometheus.yml').read_text().replace('app:8082','host.docker.internal:8082');(directory/'prometheus.local.yml').write_text(base);os.chmod(directory/'prometheus.local.yml',0o444)
    rules=(ROOT/'deploy/observability/alerts.yml').read_text()
    if args.proof_alert:rules+='\n  - name: local-delivery-proof\n    rules:\n      - alert: ClickLocalValidationProof\n        expr: vector(1)\n        labels: {severity: info}\n'
    (directory/'alerts.local.yml').write_text(rules);os.chmod(directory/'alerts.local.yml',0o444)
    receiver={'route':{'receiver':'local-inbox','group_by':['alertname'],'group_wait':'1s','group_interval':'5s','repeat_interval':'4h'},'receivers':[{'name':'local-inbox','webhook_configs':[{'url':'http://host.docker.internal:9099/alerts','send_resolved':True}]}]};(directory/'alertmanager.local.yml').write_text(json.dumps(receiver));os.chmod(directory/'alertmanager.local.yml',0o444)
    configuration={'services':{'prometheus':{'volumes':[str(directory/'prometheus.local.yml')+':/etc/prometheus/prometheus.yml:ro',str(directory/'alerts.local.yml')+':/etc/prometheus/alerts.yml:ro']},'alertmanager':{'volumes':[str(directory/'alertmanager.local.yml')+':/etc/alertmanager/alertmanager.yml:ro']},'otel':{'ports':['127.0.0.1:4318:4318']}}};override.write_text(json.dumps(configuration))
    if not pidfile.exists():
        fd=os.open(directory/'alert-inbox.log',os.O_CREAT|os.O_APPEND|os.O_WRONLY,0o600)
        with os.fdopen(fd,'ab') as log:process=subprocess.Popen([sys.executable,str(ROOT/'scripts/alert-inbox.py')],cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
        record={'pid':process.pid,'started':subprocess.check_output(['ps','-p',str(process.pid),'-o','lstart='],text=True).strip()};pidfile.write_text(json.dumps(record));os.chmod(pidfile,0o600)
    selected=['prometheus','alertmanager','otel']+([] if args.no_grafana else ['grafana']);subprocess.run([*command,'up','-d',*selected],env=env,check=True)
    for port in [9090,9093]:
        for _ in range(120):
            try:
                with urllib.request.urlopen('http://localhost:'+str(port)+'/-/ready',timeout=1) as response:
                    if response.status==200:break
            except OSError:pass
            time.sleep(.25)
        else:raise RuntimeError('Local monitoring readiness timed out')
    # Reload changed rules/config even when Compose can reuse the containers.
    for service in ['prometheus','alertmanager']:
        result=subprocess.run([*command,'ps','-q',service],env=env,capture_output=True,text=True,check=True)
        if result.stdout.strip():subprocess.run(['docker','kill','--signal','HUP',result.stdout.strip()],capture_output=True,check=True)
    print('Local monitoring started. Prometheus:9090, Alertmanager:9093, bounded alert inbox:9099/alerts; Grafana:3001 when enabled. All published ports use loopback.')
if __name__=='__main__':main()
