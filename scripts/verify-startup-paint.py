"""Verify first-frame desktop painting without pointer input or real shortcuts.

Run only while the normal organizer is stopped. The executable must support the
guarded --render-fixture mode; every file used here belongs to a temporary test
profile. No hover, focus, reveal-desktop command, or application launch is sent.
"""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


GROUP_COUNT = 12
TIMEOUT = 5.0
CONTROLLER_CLASS = 'NestDesktopWindow'
CONTROLLER_TITLE = 'NestDesktopController'
TH32CS_SNAPPROCESS = 0x00000002
SYNCHRONIZE = 0x00100000
WAIT_OBJECT_0 = 0


class ProcessEntry(ctypes.Structure):
    _fields_ = [
        ('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD),
        ('th32ProcessID', wintypes.DWORD), ('th32DefaultHeapID', ctypes.c_size_t),
        ('th32ModuleID', wintypes.DWORD), ('cntThreads', wintypes.DWORD),
        ('th32ParentProcessID', wintypes.DWORD), ('pcPriClassBase', wintypes.LONG),
        ('dwFlags', wintypes.DWORD), ('szExeFile', wintypes.WCHAR * 260),
    ]


def windows_api():
    user = ctypes.WinDLL('user32', use_last_error=True)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    user.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
    user.FindWindowW.restype = wintypes.HWND
    user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user.GetWindowThreadProcessId.restype = wintypes.DWORD
    kernel.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
    kernel.Process32FirstW.restype = wintypes.BOOL
    kernel.Process32NextW.argtypes = kernel.Process32FirstW.argtypes
    kernel.Process32NextW.restype = wintypes.BOOL
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.WaitForSingleObject.restype = wintypes.DWORD
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    return user, kernel


def controller_pid(user):
    window = user.FindWindowW(CONTROLLER_CLASS, CONTROLLER_TITLE)
    if not window:
        return None
    owner = wintypes.DWORD()
    user.GetWindowThreadProcessId(window, ctypes.byref(owner))
    return owner.value


def wait_helpers(kernel, parent_pid, executable_name):
    """Wait for only this owned parent incarnation's recovery children."""
    snapshot = kernel.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    handles = []
    try:
        entry = ProcessEntry()
        entry.dwSize = ctypes.sizeof(entry)
        more = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            if (entry.th32ParentProcessID == parent_pid
                    and entry.szExeFile.casefold() == executable_name.casefold()):
                handle = kernel.OpenProcess(SYNCHRONIZE, False, entry.th32ProcessID)
                if handle:
                    handles.append(handle)
                elif ctypes.get_last_error() != 87:  # Already exited is harmless.
                    raise ctypes.WinError(ctypes.get_last_error())
            more = kernel.Process32NextW(snapshot, ctypes.byref(entry))
        deadline = time.monotonic() + TIMEOUT
        for handle in handles:
            remaining = max(0, int((deadline - time.monotonic()) * 1000))
            if kernel.WaitForSingleObject(handle, remaining) != WAIT_OBJECT_0:
                raise AssertionError('The owned recovery helper did not finish after fixture exit')
    finally:
        kernel.CloseHandle(snapshot)
        for handle in handles:
            kernel.CloseHandle(handle)


def startup_options():
    info = subprocess.STARTUPINFO()
    info.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 0  # SW_HIDE: exercise the real quiet-startup path.
    return {'startupinfo': info, 'creationflags': subprocess.CREATE_NO_WINDOW}


def read_report(path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (FileNotFoundError, json.JSONDecodeError):
        # A diagnostic file can briefly be observed while its writer is open.
        return None


def wait_report(path, process, deadline=None):
    if deadline is None:
        deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        report = read_report(path)
        if report is not None:
            return report
        if process.poll() is not None:
            raise AssertionError(f'Fixture exited before writing {path.name}: {process.returncode}')
        time.sleep(0.02)
    raise AssertionError(f'No valid {path.name} within {TIMEOUT:.0f} seconds')


def request(exe, arguments, env, process, user, deadline=None):
    # Refuse to forward a command to another organizer if our fixture exited.
    if controller_pid(user) != process.pid:
        raise AssertionError('The controller is not owned by this fixture; no command was sent')
    timeout = TIMEOUT if deadline is None else max(0.001, deadline - time.monotonic())
    result = subprocess.run(
        [str(exe), *arguments, '--render-fixture'], env=env,
        stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout,
        **startup_options(),
    )
    if result.returncode != 0:
        raise AssertionError(f'Fixture command {arguments} returned {result.returncode}')


def check_report(report, data, ready_count, injected, pending, timer):
    tiles = report.get('tiles', [])
    assert len(tiles) == GROUP_COUNT, f'Expected {GROUP_COUNT} tiles: {report}'
    ready = [tile for tile in tiles if tile.get('surface_ready') is True]
    assert len(ready) == ready_count, f'Unexpected ready tile count: {report}'
    assert all(tile.get('presentations', 0) >= 1 for tile in ready), report
    if ready_count == GROUP_COUNT:
        assert all(tile.get('visible') is True for tile in tiles), report
    assert report.get('tile_mouse_events') == 0, (
        'Pointer traffic occurred; this run cannot establish startup without hover', report)
    assert report.get('injected_paint_failures') == injected, report
    assert report.get('paint_retry_pending') == pending, report
    assert report.get('paint_retry_timer_active') is timer, report
    assert Path(report['layout_path']).resolve() == (data / 'layout.json').resolve(), report


def semantic_layout(config):
    """Allow display-position normalization and the local startup preference."""
    result = json.loads(json.dumps(config))
    for group in result['groups']:
        group.pop('x', None)
        group.pop('y', None)
    result.get('settings', {}).pop('start_with_windows', None)
    return result


def fixture_layout():
    colors = ['#8CA9F8', '#C69AF5', '#90D5BA', '#FF4757', '#E86DB0', '#36C789']
    return {
        'version': 1,
        'groups': [
            {'id': f'startup-group-{index:02}', 'name': f'Startup fixture {index + 1:02}',
             'color': colors[index % len(colors)], 'x': 48 + (index % 6) * 116,
             'y': 64 + (index // 6) * 184, 'items': []}
            for index in range(GROUP_COUNT)
        ],
        'settings': {
            'start_with_windows': False, 'theme': 'system', 'tile_size': 112,
            'show_item_counts': False, 'snap_to_grid': True, 'hover_open': False,
            'hover_delay_ms': 450, 'custom_colors': [],
        },
    }


def run_case(exe, data, env, user, kernel, case, inject):
    if controller_pid(user) is not None:
        raise AssertionError('An organizer controller already exists; fixture launch refused')
    initial_path = data / 'startup-paint.json'
    diagnostic_path = data / 'diagnostics.json'
    initial_path.unlink(missing_ok=True)
    diagnostic_path.unlink(missing_ok=True)
    arguments = [str(exe), '--background', '--render-fixture']
    if inject:
        arguments.append('--fail-first-tile-paint')
    started = time.monotonic()
    process = subprocess.Popen(
        arguments, env=env, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **startup_options(),
    )
    try:
        initial = wait_report(initial_path, process)
        check_report(initial, data, GROUP_COUNT - int(inject), int(inject), int(inject), inject)
        initial_elapsed = time.monotonic() - started
        deadline = time.monotonic() + TIMEOUT
        latest = None
        while time.monotonic() < deadline:
            diagnostic_path.unlink(missing_ok=True)
            request(exe, ['--diagnostics'], env, process, user, deadline)
            latest = wait_report(diagnostic_path, process, deadline)
            if time.monotonic() > deadline:
                raise AssertionError(f'Eventual paint diagnostics exceeded {TIMEOUT:.0f} seconds')
            if (sum(tile.get('surface_ready') is True for tile in latest.get('tiles', [])) == GROUP_COUNT
                    and latest.get('paint_retry_pending') == 0
                    and latest.get('paint_retry_timer_active') is False):
                check_report(latest, data, GROUP_COUNT, int(inject), 0, False)
                break
            time.sleep(0.02)
        else:
            raise AssertionError(f'All tiles did not become ready within {TIMEOUT:.0f}s: {latest}')
        return {
            'case': case, 'initial_ready': GROUP_COUNT - int(inject),
            'eventual_ready': GROUP_COUNT, 'pointer_events': latest['tile_mouse_events'],
            'initial_snapshot_seconds': round(initial_elapsed, 3),
            'injected_paint_failures': int(inject), 'retry_timer_stopped': True,
        }
    finally:
        try:
            if process.poll() is None and controller_pid(user) == process.pid:
                request(exe, ['--quit'], env, process, user)
            process.wait(timeout=TIMEOUT)
        except (AssertionError, subprocess.TimeoutExpired):
            # Failure cleanup is confined to the child created in this case.
            # No real profile or Desktop files exist in this fixture.
            if process.poll() is None:
                process.kill()
                process.wait(timeout=TIMEOUT)
            raise
        finally:
            # Holding Popen's process handle also prevents parent PID reuse here.
            wait_helpers(kernel, process.pid, exe.name)
        assert process.returncode == 0, f'Fixture exit was not graceful: {process.returncode}'
        assert controller_pid(user) != process.pid, 'Fixture controller remained after exit'


def main():
    if os.name != 'nt':
        raise SystemExit('This native startup verification requires Windows.')
    if len(sys.argv) != 2:
        raise SystemExit('Usage: python scripts/verify-startup-paint.py path/to/nest-desktop.exe')
    exe = Path(sys.argv[1]).resolve(strict=True)
    user, kernel = windows_api()
    if controller_pid(user) is not None:
        raise SystemExit('Quit the running organizer before this synthetic startup check; no commands were sent.')
    with tempfile.TemporaryDirectory(prefix='startup-paint-', dir=Path(__file__).parent) as tmp:
        data = Path(tmp).resolve()
        (data / '.render-fixture').write_text('Synthetic startup paint verification only', encoding='utf-8')
        layout = fixture_layout()
        (data / 'layout.json').write_text(json.dumps(layout), encoding='utf-8')
        expected = semantic_layout(layout)
        env = dict(os.environ, NESTDESKTOP_DATA_DIR=str(data))
        results = []
        for index in range(3):
            results.append(run_case(exe, data, env, user, kernel, f'normal-restart-{index + 1}', False))
            saved = json.loads((data / 'layout.json').read_text(encoding='utf-8-sig'))
            assert semantic_layout(saved) == expected, 'Group identities or semantic layout changed across restart'
        results.append(run_case(exe, data, env, user, kernel, 'injected-first-paint-failure', True))
        saved = json.loads((data / 'layout.json').read_text(encoding='utf-8-sig'))
        assert semantic_layout(saved) == expected, 'Recovery changed group identities or semantic layout'
    print(json.dumps({'passed': True, 'groups': GROUP_COUNT, 'cases': results}, indent=2))
    print('Startup paint: three quiet restarts and one injected initial failure passed without pointer input.')


if __name__ == '__main__':
    main()
