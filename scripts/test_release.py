"""Meaningful failure checks for signed release verification."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('release',Path(__file__).with_name('release.py'))
release=importlib.util.module_from_spec(spec);spec.loader.exec_module(release)

class ReleaseVerification(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.folder=self.root/'bundle';self.folder.mkdir()
        self.key=self.root/'key.pem'
        subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(self.key)],check=True,capture_output=True)
        artifact=self.folder/'sdk.tgz';artifact.write_bytes(b'verified sdk bytes')
        entry={'name':artifact.name,'bytes':artifact.stat().st_size,'sha256':release.sha(artifact)}
        (self.folder/'release-manifest.json').write_text(json.dumps({'format':1,'artifacts':[entry]}))
        (self.folder/'SHA256SUMS').write_text(entry['sha256']+'  '+entry['name']+'\n')
        release.sign(self.folder,self.key)
        self.public=self.folder/'release-public.pem'
    def test_intact_release_passes(self):
        self.assertEqual(release.verify(self.folder,self.public),1)
    def test_modified_artifact_is_rejected(self):
        (self.folder/'sdk.tgz').write_bytes(b'tampered sdk bytes')
        with self.assertRaises(AssertionError):release.verify(self.folder,self.public)
    def test_manifest_and_checksum_rewrite_is_rejected(self):
        manifest=self.folder/'release-manifest.json';data=json.loads(manifest.read_text());data['artifacts'][0]['name']='../outside'
        manifest.write_text(json.dumps(data))
        with self.assertRaises(subprocess.CalledProcessError):release.verify(self.folder,self.public)
    def test_substituted_public_key_is_rejected(self):
        other=self.root/'other.pem';public=self.root/'other-public.pem'
        subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(other)],check=True,capture_output=True)
        subprocess.run(['openssl','pkey','-in',str(other),'-pubout','-out',str(public)],check=True,capture_output=True)
        with self.assertRaises(subprocess.CalledProcessError):release.verify(self.folder,public)
    def test_signed_path_escape_is_rejected(self):
        manifest=self.folder/'release-manifest.json';data=json.loads(manifest.read_text());data['artifacts'][0]['name']='../outside'
        manifest.write_text(json.dumps(data));release.sign(self.folder,self.key)
        with self.assertRaises(AssertionError):release.verify(self.folder,self.public)

if __name__=='__main__':unittest.main()
