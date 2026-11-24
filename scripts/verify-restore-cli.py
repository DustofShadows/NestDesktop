import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

exe = Path(sys.argv[1]).resolve()
base = Path(__file__).parent
with tempfile.TemporaryDirectory(prefix='restore-cli-', dir=base) as tmp:
    data = Path(tmp)
    desktop = data / 'SyntheticDesktop'
    storage = data / 'shortcuts'
    desktop.mkdir()
    storage.mkdir()
    original = desktop / 'example.lnk'
    stored = storage / 'example.lnk'
    payload = b'NestDesktop synthetic shortcut bytes'
    stored.write_bytes(payload)
    entry = {'original': str(original), 'stored': str(stored)}
    ledger = storage / 'collection.json'
    ledger.write_text(json.dumps({'version': 1, 'shortcuts': [entry]}), encoding='utf-8')
    layout = data / 'layout.json'
    layout.write_text(json.dumps({'version': 1, 'groups': [{'id': 'g', 'name': 'Apps', 'color': '#A6E9D1', 'x': 48, 'y': 64,
       'items': [{'id': 'a', 'name': 'Example', 'target': str(stored), 'arguments': '', 'children': []}]}]}), encoding='utf-8')
    env = dict(os.environ, NESTDESKTOP_DATA_DIR=str(data))
    result = subprocess.run([str(exe), '--restore-shortcuts', '--background'], env=env, timeout=15)
    assert result.returncode == 0, result.returncode
    assert original.read_bytes() == payload
    assert not stored.exists()
    assert json.loads(ledger.read_text(encoding='utf-8'))['shortcuts'] == []
    assert json.loads(layout.read_text(encoding='utf-8'))['groups'][0]['items'][0]['target'] == str(original)
    stored.write_bytes(payload)
    ledger.write_text('{invalid recovery data', encoding='utf-8')
    result = subprocess.run([str(exe), '--restore-shortcuts', '--background'], env=env, timeout=15)
    assert result.returncode == 2, result.returncode
    assert stored.read_bytes() == payload
    assert original.read_bytes() == payload
    assert (data / 'restore-error.txt').exists()
print('Restore CLI: successful restoration and corrupt-ledger failure both verified with synthetic files.')
