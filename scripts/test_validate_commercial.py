"""Safety checks for the explicitly configured external validation helper."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('commercial_validator', Path(__file__).with_name('validate-commercial.py'))
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


class ValidationGuardTests(unittest.TestCase):
    def test_missing_and_live_keys_never_touch_stores(self):
        for values in [{}, {'APP_ENV': 'development', 'STRIPE_SECRET_KEY': 'sk_live_fixture'}, {'APP_ENV': 'production', 'STRIPE_SECRET_KEY': 'sk_test_fixture'}]:
            for owner_only in [False, True]:
                with self.subTest(values=values, owner_only=owner_only), patch.dict(os.environ, values, clear=True), patch.object(validator, 'pg') as pg, patch.object(validator, 'ch') as ch:
                    with self.assertRaises(RuntimeError):
                        validator.setup(8084, owner_only=owner_only)
                    pg.assert_not_called()
                    ch.assert_not_called()

    def test_cleanup_rejects_unowned_or_injected_database_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(validator, 'RUNS', root):
                folder = root / ('click_commercial_' + 'a' * 32)
                folder.mkdir(parents=True)
                manifest = folder / 'run.json'
                manifest.write_text(json.dumps({'database': 'click; DROP DATABASE click;'}))
                with patch.object(validator, 'pg') as pg, patch.object(validator, 'ch') as ch:
                    with self.assertRaises(RuntimeError):
                        validator.cleanup(manifest)
                    pg.assert_not_called()
                    ch.assert_not_called()

    def test_private_files_are_exclusive_and_private(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'credentials.json'
            validator.private_json(path, {'password': 'fixture'})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                validator.private_json(path, {'password': 'replacement'})


if __name__ == '__main__':
    unittest.main()
