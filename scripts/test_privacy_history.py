#!/usr/bin/env python3
"""Ensure history-scanner examples do not suppress real-looking credentials."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('history', Path(__file__).with_name('privacy-history-audit.py'))
history = importlib.util.module_from_spec(spec)
spec.loader.exec_module(history)


class HistoryRulesTests(unittest.TestCase):
    def test_only_documented_example_origins_are_exempt(self):
        self.assertEqual(list(history.matches('https://device.example.ts.net', 'fixture')), [])
        self.assertEqual(list(history.matches('https://' + 'private-host.tail123.ts.net', 'fixture')),
                         [(1, 'private-tailnet')])

    def test_example_does_not_hide_second_real_origin(self):
        value = 'https://device.example.ts.net ' + 'https://' + 'private-host.tail123.ts.net'
        self.assertEqual(list(history.matches(value, 'fixture')), [(1, 'private-tailnet')])

    def test_secret_and_pairing_rules(self):
        # Construct synthetic values so the source scanner does not flag its own fixture.
        value = 'sk-' + 'or-v1-' + 'x' * 48 + '\n#pair=' + 'a' * 64
        self.assertEqual(list(history.matches(value, 'fixture')),
                         [(1, 'provider-token'), (2, 'pairing-value')])


if __name__ == '__main__':
    unittest.main()
