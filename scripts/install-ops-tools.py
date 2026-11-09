#!/usr/bin/env python3
"""Install pinned, SHA-256 verified operator tools without administrator access."""
import argparse
import bz2
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import tarfile
import urllib.request
ROOT=Path(__file__).resolve().parent.parent

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('tools',nargs='*',choices=['restic','trivy','cargo-deny']);args=parser.parse_args();pins=json.loads((ROOT/'deploy/tool-versions.json').read_text());machine={'aarch64':'arm64','arm64':'arm64','x86_64':'amd64'}.get(platform.machine());system=platform.system().lower()+'-'+str(machine);target=ROOT/'.local/ops-tools';target.mkdir(mode=0o700,parents=True,exist_ok=True)
    for name in args.tools or list(pins):
        asset=pins[name]['assets'].get(system)
        if not asset:raise SystemExit('Unsupported platform for '+name)
        with urllib.request.urlopen(asset['url'],timeout=120) as response:data=response.read()
        if hashlib.sha256(data).hexdigest()!=asset['sha256']:raise SystemExit('Tool download integrity mismatch')
        if asset['url'].endswith('.bz2'):binary=bz2.decompress(data)
        else:
            with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as archive:
                members=[m for m in archive if m.isfile() and Path(m.name).name==name]
                if len(members)!=1:raise SystemExit('Ambiguous tool archive')
                binary=archive.extractfile(members[0]).read()
        temporary=target/(name+'.tmp');fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o700)
        with os.fdopen(fd,'wb') as stream:stream.write(binary)
        os.replace(temporary,target/name);print(name+' '+pins[name]['version']+' installed; SHA-256 verified')
if __name__=='__main__':main()
