import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import backup

class RecoverySafetyTests(unittest.TestCase):
    def test_unsafe_snapshot_argument_never_touches_a_database(self):
        with patch.object(backup,'connect') as connection:
            with self.assertRaises(RuntimeError):backup.restore('--repository=untrusted',True,False)
            connection.assert_not_called()
    def test_archive_traversal_is_rejected_before_creating_stores(self):
        with tempfile.TemporaryDirectory() as directory:
            def dump(*args,**kwargs):
                with tarfile.open(fileobj=kwargs['stdout'],mode='w') as archive:
                    member=tarfile.TarInfo('../escape');member.size=1;archive.addfile(member,io.BytesIO(b'x'))
            with patch.object(backup,'require_repository',return_value=Path(directory)),patch.object(backup,'record'),patch.object(backup,'restic',side_effect=dump),patch.object(backup,'connect') as connection:
                with self.assertRaises(RuntimeError):backup.restore('latest',True,False)
                connection.assert_not_called()
                self.assertFalse((Path(directory).parent/'escape').exists())
    def test_configuration_and_database_integrity_are_both_required(self):
        with tempfile.TemporaryDirectory() as directory:
            folder=Path(directory);files=['postgres.dump','events.native','pageviews_hourly.native','configuration.env']
            for name in files:(folder/name).write_bytes(b'fixture')
            manifest={'format':1,'geoip_included':False,'sha256':{name:backup.file_hash(folder/name) for name in files}};(folder/'manifest.json').write_text(json.dumps(manifest));backup.verify_files(folder)
            (folder/'configuration.env').write_bytes(b'changed')
            with self.assertRaises(RuntimeError):backup.verify_files(folder)
    def test_dashboard_key_is_required_and_integrity_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            folder=Path(directory);files=['postgres.dump','events.native','pageviews_hourly.native','configuration.env','connections.key']
            for name in files:(folder/name).write_bytes(b'fixture')
            manifest={'format':1,'geoip_included':False,'connections_key_included':True,'sha256':{name:backup.file_hash(folder/name) for name in files}}
            (folder/'manifest.json').write_text(json.dumps(manifest));backup.verify_files(folder)
            (folder/'connections.key').write_bytes(b'wrong key')
            with self.assertRaises(RuntimeError):backup.verify_files(folder)
if __name__=='__main__':unittest.main()
