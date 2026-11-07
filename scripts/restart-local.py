#!/usr/bin/env python3
"""Restart the already-built development binary without changing database volumes."""
import os
from pathlib import Path
import signal
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
if os.environ.get('APP_ENV') != 'development':
    raise SystemExit('This helper starts development only; load the trusted .env first.')
PORT = int(os.environ.get('BIND', '127.0.0.1:8082').rsplit(':', 1)[1])
BASE = os.environ.get('PUBLIC_URL', 'http://localhost:8082').rstrip('/')
result = subprocess.run(['lsof', f'-tiTCP:{PORT}', '-sTCP:LISTEN'], capture_output=True, text=True)
for raw in result.stdout.split():
    pid = int(raw)
    command = subprocess.check_output(['ps', '-p', str(pid), '-o', 'command='], text=True)
    if 'target/debug/click-server serve' not in command:
        raise SystemExit('Unexpected listener; refusing to stop it.')
    os.kill(pid, signal.SIGTERM)
for _ in range(100):
    if subprocess.run(['lsof', f'-tiTCP:{PORT}', '-sTCP:LISTEN'], capture_output=True).returncode:
        break
    time.sleep(0.1)
else:
    raise SystemExit('Previous listener did not exit.')
(ROOT / '.local').mkdir(mode=0o700, exist_ok=True)
fd = os.open(ROOT / '.local/server.log', os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o600)
with os.fdopen(fd, 'ab', buffering=0) as log:
    process = subprocess.Popen([str(ROOT / 'target/debug/click-server'), 'serve'], cwd=ROOT,
                               stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
(ROOT / '.local/server.pid').write_text(str(process.pid) + '\n')
for _ in range(100):
    if process.poll() is not None:
        raise SystemExit('Server failed; inspect .local/server.log.')
    try:
        with urllib.request.urlopen(BASE + '/readyz', timeout=1) as response:
            if response.status == 200:
                print('Ready: ' + BASE)
                break
    except OSError:
        pass
    time.sleep(0.1)
else:
    raise SystemExit('Readiness did not succeed.')
