#!/usr/bin/env python3
"""Loopback-only, bounded local Alertmanager delivery evidence; no external messages."""
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
import os
from pathlib import Path
import threading
ROOT=Path(__file__).resolve().parent.parent
FILE=ROOT/'.local/ops/local-alerts.json'
LOCK=threading.Lock()
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_GET(self):
        if self.path!='/alerts':self.send_error(404);return
        with LOCK:body=FILE.read_bytes() if FILE.exists() else b'[]'
        self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(body)
    def do_POST(self):
        if self.path!='/alerts':self.send_error(404);return
        size=int(self.headers.get('Content-Length','0'))
        if size>128*1024:self.send_error(413);return
        try:data=json.loads(self.rfile.read(size))
        except ValueError:self.send_error(400);return
        records=[{'name':str(a.get('labels',{}).get('alertname',''))[:100],'severity':str(a.get('labels',{}).get('severity',''))[:20],'status':str(a.get('status',''))[:20],'starts_at':str(a.get('startsAt',''))[:64]} for a in data.get('alerts',[])[:100]]
        with LOCK:
            existing=json.loads(FILE.read_text()) if FILE.exists() else []
            fd=os.open(FILE,os.O_CREAT|os.O_TRUNC|os.O_WRONLY,0o600)
            with os.fdopen(fd,'w') as stream:json.dump((existing+records)[-100:],stream)
        self.send_response(200);self.end_headers()
if __name__=='__main__':
    os.umask(0o077);FILE.parent.mkdir(parents=True,exist_ok=True,mode=0o700);ThreadingHTTPServer(('127.0.0.1',9099),Handler).serve_forever()
