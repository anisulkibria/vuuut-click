#!/usr/bin/env python3
"""Build offline-installable SDK/operator releases and verify pinned-key signatures."""
import argparse
import base64
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT=Path(__file__).resolve().parent.parent

def run(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()

def archive(output, files):
    with output.open('wb') as raw, gzip.GzipFile(fileobj=raw,mode='wb',mtime=0,filename='') as compressed, tarfile.open(fileobj=compressed,mode='w') as tar:
        for path in sorted(files):
            assert path.is_file() and not path.is_symlink(), 'Release input must be a regular file'
            name=path.relative_to(ROOT).as_posix()
            info=tarfile.TarInfo(name);info.size=path.stat().st_size;info.mode=0o644
            tar.addfile(info,io.BytesIO(path.read_bytes()))

def sign(folder, key):
    public=run(['openssl','pkey','-in',str(key),'-pubout'],capture_output=True).stdout
    manifest=folder/'release-manifest.json';data=json.loads(manifest.read_text())
    data['signing_public_key_sha256']=hashlib.sha256(public).hexdigest()
    manifest.write_text(json.dumps(data,indent=2,sort_keys=True)+'\n')
    (folder/'release-public.pem').write_bytes(public)
    run(['openssl','pkeyutl','-sign','-rawin','-inkey',str(key),'-in',str(manifest),'-out',str(folder/'release-manifest.sig')],capture_output=True)

def verify(folder, public_key):
    folder=folder.resolve();public_key=public_key.resolve()
    manifest=folder/'release-manifest.json'
    # Verify before trusting any paths or claims from the manifest.
    run(['openssl','pkeyutl','-verify','-pubin','-rawin','-inkey',str(public_key),'-in',str(manifest),'-sigfile',str(folder/'release-manifest.sig')],capture_output=True)
    data=json.loads(manifest.read_text())
    assert sha(public_key)==data['signing_public_key_sha256'], 'Signing identity mismatch'
    assert data['format']==1 and data['artifacts'], 'Invalid release manifest'
    sums=[];names=set()
    for item in data['artifacts']:
        name=item['name']
        assert re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]+',name) and name not in names, 'Unsafe or duplicate artifact name'
        names.add(name);path=folder/name
        assert path.is_file() and not path.is_symlink() and path.stat().st_size==item['bytes'] and sha(path)==item['sha256'], 'Artifact integrity failure'
        sums.append(item['sha256']+'  '+name+'\n')
    assert (folder/'SHA256SUMS').read_text()==''.join(sums), 'Checksum list differs from signed manifest'
    return len(names)

def build(folder, allow_dirty, signing_key):
    folder=folder.resolve()
    assert not folder.exists(), 'Use a new output directory; existing releases are never overwritten'
    dirty=bool(run(['git','status','--porcelain'],capture_output=True,text=True).stdout)
    assert allow_dirty or not dirty, 'Release requires a clean worktree; use --allow-dirty for local candidates only'
    version=tomllib.loads((ROOT/'Cargo.toml').read_text())['workspace']['package']['version']
    assert re.fullmatch(r'\d+\.\d+\.\d+(?:-[a-zA-Z0-9.]+)?',version)
    for path in ['package.json','tracker/package.json']:
        assert json.loads((ROOT/path).read_text())['version']==version, 'Package versions differ'
    assert json.loads((ROOT/'docs/openapi.json').read_text())['info']['version']==version
    assert (ROOT/'tracker/LICENSE').read_bytes()==(ROOT/'LICENSE').read_bytes()
    assert (ROOT/'crates/click-sdk/LICENSE').read_bytes()==(ROOT/'LICENSE').read_bytes()
    assert (ROOT/'tracker/THIRD_PARTY_NOTICES').read_bytes()==(ROOT/'docs/licenses/WEB-VITALS-LICENSE').read_bytes()
    run(['npm','run','check']);run(['npm','run','build']);run(['npm','run','test:sdk'])
    run(['cargo','package','--locked','--offline','-p','click-sdk']+(['--allow-dirty'] if allow_dirty else []))
    folder.mkdir(parents=True)
    try:
        package=run(['npm','pack','./tracker','--ignore-scripts','--json','--pack-destination',str(folder)],capture_output=True,text=True)
        packed=json.loads(package.stdout)[0]; assert packed['name']=='@vuuut/click' and packed['version']==version
        with tempfile.TemporaryDirectory(prefix='click-sdk-consumer-') as name:
            consumer=Path(name)
            (consumer/'package.json').write_text('{"private":true,"type":"module"}')
            run(['npm','install','--prefix',name,'--ignore-scripts','--no-audit','--no-fund','--package-lock=false',str(folder/packed['filename'])],capture_output=True)
            shutil.copyfile(ROOT/'tests/sdk/consumer.ts',consumer/'consumer.ts')
            run([str(ROOT/'node_modules/.bin/tsc'),'--noEmit','--strict','--target','ES2022','--module','NodeNext','--moduleResolution','NodeNext',str(consumer/'consumer.ts')])
            env=os.environ.copy();env['SDK_PACKAGE_URL']=(consumer/'node_modules/@vuuut/click/dist/api.js').as_uri()
            run(['node','--test','tests/sdk/api.test.mjs'],env=env)
        shutil.copyfile(ROOT/f'target/package/click-sdk-{version}.crate',folder/f'click-sdk-{version}.crate')
        scripts=[ROOT/'tracker/dist'/name for name in ['tracker.js','index.js','api.js','vitals.js','index.d.ts','api.d.ts','vitals.d.ts']]
        archive(folder/f'vuuut-click-browser-{version}.tar.gz',scripts+[ROOT/'tracker/LICENSE',ROOT/'tracker/THIRD_PARTY_NOTICES',ROOT/'tracker/README.md'])
        operator=[ROOT/n for n in ['README.md','LICENSE','CHANGELOG.md','.env.example','Cargo.lock','Dockerfile']]
        reviewable=set(run(['git','ls-files','--cached','--others','--exclude-standard','-z'],capture_output=True).stdout.decode().split(chr(0)))-{''}
        for name in sorted(reviewable):
            p=ROOT/name
            if Path(name).parts[0] in ['docs','deploy','migrations','scripts']:
                assert p.suffix not in ['.pem','.key','.env','.dump','.mmdb'], 'Private release input rejected'
                operator.append(p)
        archive(folder/f'vuuut-click-operator-{version}.tar.gz',operator)
        files=sorted(p for p in folder.iterdir() if p.is_file())
        source=hashlib.sha256()
        names=run(['git','ls-files','--cached','--others','--exclude-standard','-z'],capture_output=True).stdout.decode().split('\0')
        for name in sorted(set(names)-{''}):
            p=ROOT/name
            if p.is_file() and not p.is_symlink():source.update(name.encode()+b'\0'+bytes.fromhex(sha(p)))
        data={'format':1,'version':version,'git_commit':run(['git','rev-parse','HEAD'],capture_output=True,text=True).stdout.strip(),'dirty':dirty,'source_tree_sha256':source.hexdigest(),'artifacts':[{'name':p.name,'bytes':p.stat().st_size,'sha256':sha(p)} for p in files],'scripts':{p.name:{'sha256':sha(p),'sri':'sha384-'+base64.b64encode(hashlib.sha384(p.read_bytes()).digest()).decode()} for p in scripts if p.suffix=='.js'},'migrations':{p.relative_to(ROOT).as_posix():sha(p) for p in sorted((ROOT/'migrations').rglob('*.sql'))},'production_accepted':False}
        (folder/'release-manifest.json').write_text(json.dumps(data,indent=2,sort_keys=True)+'\n')
        (folder/'SHA256SUMS').write_text(''.join(x['sha256']+'  '+x['name']+'\n' for x in data['artifacts']))
        if signing_key:
            sign(folder,signing_key.resolve());verify(folder,folder/'release-public.pem')
        print(json.dumps({'version':version,'artifacts':len(files),'signed':bool(signing_key),'dirty':dirty,'output':str(folder)}))
    except BaseException:
        # Do not leave a partially produced release that looks complete.
        shutil.rmtree(folder)
        raise

def main():
    parser=argparse.ArgumentParser(description=__doc__);commands=parser.add_subparsers(dest='command',required=True)
    b=commands.add_parser('build');b.add_argument('--output',type=Path,required=True);b.add_argument('--allow-dirty',action='store_true');b.add_argument('--signing-key',type=Path)
    v=commands.add_parser('verify');v.add_argument('directory',type=Path);v.add_argument('--public-key',type=Path,required=True)
    args=parser.parse_args()
    if args.command=='build':build(args.output,args.allow_dirty,args.signing_key)
    else:print(json.dumps({'verified_artifacts':verify(args.directory,args.public_key)}))
if __name__=='__main__':main()
