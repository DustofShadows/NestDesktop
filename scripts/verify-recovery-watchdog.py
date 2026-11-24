"""Exercise the real sleeping companion with synthetic files and forced exit."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

exe = Path(sys.argv[1]).resolve()


def wait_until(check, description):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.05)
    raise AssertionError(description)


def fixture(data):
    desktop = data / 'SyntheticDesktop'
    storage = data / 'shortcuts'
    desktop.mkdir()
    storage.mkdir()
    original = desktop / 'Example.lnk'
    stored = storage / 'Example.lnk'
    payload = b'Synthetic shortcut payload; never launched'
    stored.write_bytes(payload)
    record = {'original': str(original), 'stored': str(stored)}
    ledger = storage / 'collection.json'
    ledger.write_text(json.dumps({'version': 1, 'shortcuts': [record]}), encoding='utf-8')
    layout = data / 'layout.json'
    layout.write_text(json.dumps({'version': 1, 'groups': [
        {'id': 'group', 'name': 'Test', 'color': '#36C789', 'x': 40, 'y': 60,
         'items': [{'id': 'app', 'name': 'Example', 'target': str(stored),
                    'arguments': '', 'children': []}]}]}), encoding='utf-8')
    return original, stored, payload, ledger, layout


with tempfile.TemporaryDirectory(prefix='watchdog-', dir=Path(__file__).parent) as tmp:
    data = Path(tmp)
    original, stored, payload, ledger, layout = fixture(data)
    (data / '.recovery-fixture').write_text('synthetic only', encoding='utf-8')
    env = dict(os.environ, NESTDESKTOP_DATA_DIR=str(data))
    parent = subprocess.Popen([str(exe), '--recovery-fixture'], env=env)
    try:
        wait_until(lambda: (data / 'fixture-ready').exists(), 'Helper did not acknowledge readiness')
        assert not original.exists() and stored.read_bytes() == payload
        parent.kill()
        parent.wait(timeout=5)
        wait_until(lambda: original.exists() and not stored.exists(), 'Crash recovery did not restore the shortcut')
        wait_until(lambda: json.loads(ledger.read_text(encoding='utf-8'))['shortcuts'] == [], 'Recovery ledger not committed')
        assert original.read_bytes() == payload
        assert json.loads(layout.read_text(encoding='utf-8'))['groups'][0]['items'][0]['target'] == str(original)
        assert not (data / 'recovery-error.txt').exists()
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=5)

with tempfile.TemporaryDirectory(prefix='malformed-layout-', dir=Path(__file__).parent) as tmp:
    data = Path(tmp)
    original, stored, payload, ledger, layout = fixture(data)
    layout.write_text('{invalid saved layout', encoding='utf-8')
    before = ledger.read_bytes()
    result = subprocess.run([str(exe), '--restore-shortcuts', '--background'],
                            env=dict(os.environ, NESTDESKTOP_DATA_DIR=str(data)), timeout=15)
    assert result.returncode == 2
    assert original.read_bytes() == payload and not stored.exists()
    assert ledger.read_bytes() == before, 'Keep records until the damaged layout can be remapped'
    assert layout.read_text(encoding='utf-8') == '{invalid saved layout'

print('Recovery: ready handshake, forced parent termination, exact-byte restoration, layout remapping, and malformed-layout recovery passed.')
