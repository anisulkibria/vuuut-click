#!/usr/bin/env python3
"""Scan reviewable source and optional runtime image, emitting redacted evidence."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from datetime import datetime,timezone
ROOT=Path(__file__).resolve().parent.parent

def sanitize(report):
    findings=[]
    for result in report.get('Results',[]):
        target=result.get('Target','')
        for item in result.get('Vulnerabilities',[]) or []:
            if item.get('Severity') in ['HIGH','CRITICAL']:findings.append({'kind':'vulnerability','target':target,'id':item.get('VulnerabilityID'),'package':item.get('PkgName'),'installed':item.get('InstalledVersion'),'fixed':item.get('FixedVersion'),'severity':item.get('Severity')})
        for item in result.get('Secrets',[]) or []:findings.append({'kind':'secret','target':target,'rule':item.get('RuleID'),'severity':item.get('Severity'),'line':item.get('StartLine')})
    return findings

def main():
    os.umask(0o077);parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--image');args=parser.parse_args();folder=ROOT/'.local/ops';folder.mkdir(mode=0o700,exist_ok=True);trivy=str(ROOT/'.local/ops-tools/trivy');deny=str(ROOT/'.local/ops-tools/cargo-deny');evidence={'observed_at':datetime.now(timezone.utc).isoformat(),'source_findings':[],'image_findings':None};failed=False
    with tempfile.TemporaryDirectory(prefix='click-scan-') as name:
        stage=Path(name)
        files=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=ROOT).split(b'\0')
        for raw in files:
            if not raw:continue
            relative=Path(os.fsdecode(raw));source=ROOT/relative
            if relative.is_absolute() or '..' in relative.parts or source.is_symlink():continue
            if source.is_file():destination=stage/relative;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,destination)
        result=subprocess.run([trivy,'fs','--quiet','--scanners','vuln,secret','--format','json',str(stage)],capture_output=True,timeout=900)
        if result.returncode:evidence['source_scanner_error']=True;failed=True
        else:
            report=json.loads(result.stdout);evidence['source_findings']=sanitize(report);failed|=bool(evidence['source_findings'])
            # Generate an SBOM separately; it contains dependencies, not source matches.
            sbom=subprocess.run([trivy,'fs','--quiet','--scanners','vuln','--format','cyclonedx',str(stage)],capture_output=True,timeout=900)
            if sbom.returncode:failed=True;evidence['sbom_error']=True
            else:(folder/'source-sbom.cdx.json').write_bytes(sbom.stdout)
    deny_result=subprocess.run([deny,'--format','json','check','advisories','licenses','sources','bans'],cwd=ROOT,capture_output=True,timeout=900)
    evidence['cargo_policy_passed']=deny_result.returncode==0;failed|=deny_result.returncode!=0
    # Cargo diagnostics have no source text/secret matches; retain privately.
    (folder/'cargo-policy.jsonl').write_bytes(deny_result.stderr)
    if args.image:
        result=subprocess.run([trivy,'image','--quiet','--scanners','vuln','--format','json',args.image],capture_output=True,timeout=900)
        if result.returncode:evidence['image_scanner_error']=True;failed=True
        else:evidence['image_findings']=sanitize(json.loads(result.stdout));failed|=bool(evidence['image_findings'])
    (folder/'security-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps(evidence,indent=2));raise SystemExit(1 if failed else 0)
if __name__=='__main__':main()
