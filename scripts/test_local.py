import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import local

class NativeLifecycleSafetyTests(unittest.TestCase):
    def setUp(self):
        self.folder=tempfile.TemporaryDirectory(prefix='click-launcher-tests-');self.addCleanup(self.folder.cleanup)
        root=Path(self.folder.name);self.root=root
        for name,value in [('ROOT',root),('LOCAL',root/'.local'),('NATIVE',root/'.local/native'),('BIN',root/'.local/bin'),('APP',root/'.local/bin/click-server')]:
            item=patch.object(local,name,value);item.start();self.addCleanup(item.stop)
        env=patch.dict(os.environ,{'APP_ENV':'development','LOCAL_DB_MODE':'native','DATABASE_URL':'postgres://click:private-postgres@127.0.0.1:55432/click','CLICKHOUSE_URL':'http://127.0.0.1:58123','CLICKHOUSE_USER':'click','CLICKHOUSE_PASSWORD':'private-clickhouse','CLICKHOUSE_DATABASE':'click','BIND':'127.0.0.1:8082'},clear=True)
        env.start();self.addCleanup(env.stop)
    def test_first_setup_creates_private_distinct_credentials_once(self):
        with contextlib.redirect_stdout(io.StringIO()):local.init_env()
        path=self.root/'.env';before=path.read_bytes();local.init_env()
        self.assertEqual(before,path.read_bytes());self.assertEqual(path.stat().st_mode&0o777,0o600)
        self.assertIn(b'LOCAL_DB_MODE=native',before)
        pg=next(x for x in before.decode().splitlines() if x.startswith('DATABASE_URL='))
        ch=next(x for x in before.decode().splitlines() if x.startswith('CLICKHOUSE_PASSWORD='))
        self.assertNotIn(ch.split('=',1)[1],pg)
    def test_stop_never_signals_a_foreign_listener(self):
        with patch.object(local,'app_pid',return_value=None),patch.object(local,'busy',return_value=True),patch.object(local.os,'kill') as kill:
            with self.assertRaises(local.LocalError):local.stop_app(local.config())
            kill.assert_not_called()
    def test_database_stop_does_not_touch_unmanaged_clickhouse_or_postgres(self):
        with patch.object(local,'pid_value',return_value=None),patch.object(local,'busy',return_value=True),patch.object(local,'pg_running') as pg,patch.object(local.os,'kill') as kill:
            with self.assertRaises(local.LocalError):local.db_stop(local.config())
            pg.assert_not_called();kill.assert_not_called()
    def test_external_mode_leaves_service_lifecycle_to_operator(self):
        c=local.config();c['mode']='external'
        with patch.object(local,'pg_running') as pg,patch.object(local.os,'kill') as kill:
            local.db_stop(c);pg.assert_not_called();kill.assert_not_called()
    def test_native_config_is_loopback_only_and_does_not_store_plain_passwords(self):
        c=local.config();local.prepare_ch(c)
        config=(local.NATIVE/'clickhouse/config.xml').read_text()
        self.assertNotIn(c['ch_password'],config);self.assertNotIn(c['password'],config)
        self.assertIn('<listen_host>127.0.0.1</listen_host>',config)
        self.assertEqual((local.NATIVE/'clickhouse/config.xml').stat().st_mode&0o777,0o600)
        with patch.dict(os.environ,{'CLICKHOUSE_URL':'http://remote.example:8123'}):
            with self.assertRaises(local.LocalError):local.config()
    def test_restore_rejects_traversal_or_corruption_before_database_access(self):
        with patch.object(local,'pg') as pg:
            with self.assertRaises(local.LocalError):local.restore_snapshot(local.config(),self.root,{'files':{'../escape':'x'}})
            pg.assert_not_called()
        (self.root/'postgres.dump').write_bytes(b'corrupt')
        with patch.object(local,'pg') as pg:
            with self.assertRaises(local.LocalError):local.restore_snapshot(local.config(),self.root,{'files':{'postgres.dump':'wrong'}})
            pg.assert_not_called()
    def test_restore_refuses_nonempty_targets(self):
        with patch.object(local,'pg',return_value='1'),patch.object(local,'ch') as ch:
            with self.assertRaises(local.LocalError):local.restore_snapshot(local.config(),self.root,{'files':{}})
            ch.assert_not_called()
    def test_failed_target_shutdown_preserves_its_live_data_path(self):
        local.private_dir(local.NATIVE)
        (local.NATIVE/'keep').write_text('partial target data')
        (self.root/'.env').write_text('APP_ENV=development\n')
        bindings=json.dumps({'5432/tcp':[{'HostPort':'55432','HostIp':'127.0.0.1'}],'8123/tcp':[{'HostPort':'58123','HostIp':'127.0.0.1'}]}).encode()
        def compose(*args):return (('a' if args[-1]=='postgres' else 'b')*64).encode() if args[0]=='ps' else b''
        with patch.object(local,'docker_compose',side_effect=compose),patch.object(local,'run',return_value=bindings),patch.object(local,'install'),patch.object(local,'app_pid',return_value=None),patch.object(local,'stop_app'),patch.object(local,'migration_barrier',return_value=contextlib.nullcontext()),patch.object(local,'ch',return_value=b'0'),patch.object(local,'save_snapshot',return_value={}),patch.object(local,'db_start',side_effect=local.LocalError('restore unavailable')),patch.object(local,'db_stop',side_effect=local.LocalError('server still running')):
            with contextlib.redirect_stdout(io.StringIO()),self.assertRaises(local.LocalError):local.migrate(local.config())
        self.assertTrue((local.NATIVE/'keep').exists())
        self.assertEqual(list(local.LOCAL.glob('native-failed-*')),[])

    def test_help_has_native_commands_without_a_docker_executable(self):
        script=Path(__file__).resolve().parent.parent/'run.sh'
        result=subprocess.run(['/bin/bash',str(script),'--help'],env={'PATH':'/usr/bin:/bin'},capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        for command in ['start','stop','restart','status','setup','doctor','build','rebuild','migrate-docker']:self.assertIn(command,result.stdout)

if __name__=='__main__':unittest.main()
