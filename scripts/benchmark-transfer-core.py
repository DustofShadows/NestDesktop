"""Benchmark durable transfers in a disposable clone, using synthetic shortcuts.

No desktop/UI automation or application targets are launched. This isolates
filesystem/model cost: it excludes Explorer refresh, icon extraction and the
watchdog handshake, so these numbers are not end-to-end user-visible latency.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


TEST = r'''
use crate::{desktop_transfer, model::{Config, Item}};
use std::{fs, io::Write, path::PathBuf, time::Instant};

#[test]
fn synthetic_transfer_core_benchmark() {
    let root = PathBuf::from(std::env::var_os("NEST_TRANSFER_BENCH_DIR").unwrap());
    let mut results = Vec::new();
    for (count, existing) in [(1usize, 0usize), (1, 200), (8, 200), (24, 200)] {
        let mut collections = Vec::new();
        let mut restores = Vec::new();
        for repeat in 0..3 {
            let case = root.join(format!("case-{count}-{existing}-{repeat}"));
            let desktop = case.join("SyntheticDesktop");
            let data = case.join("Data");
            fs::create_dir_all(&desktop).unwrap();
            fs::create_dir_all(&data).unwrap();
            if std::env::var("NEST_TRANSFER_BENCH_WARM").ok().as_deref() == Some("1") {
                let mut existing_registry = fs::File::create(data.join("managed-shortcuts.json")).unwrap();
                existing_registry.write_all(br#"{"version":1,"shortcuts":[]}"#).unwrap();
                existing_registry.sync_all().unwrap();
            }
            let layout = data.join("layout.json");
            let mut config = Config::default();
            for index in 0..existing {
                config.groups[0].items.push(Item::shortcut(
                    format!("Existing {index}"),
                    format!(r"C:\SyntheticApps\existing-{index}.exe"),
                ));
            }
            config.save(&layout).unwrap();
            let originals: Vec<_> = (0..count).map(|index| {
                let path = desktop.join(format!("Example {index}.lnk"));
                fs::write(&path, b"Synthetic shortcut bytes; never launched").unwrap();
                path
            }).collect();
            let started = Instant::now();
            __COLLECT_BLOCK__
            collections.push(started.elapsed().as_secs_f64() * 1000.);
            assert!(originals.iter().all(|path| !path.exists()));
            let started = Instant::now();
            let report = desktop_transfer::restore_all(&mut config, &layout, false).unwrap();
            restores.push(started.elapsed().as_secs_f64() * 1000.);
            assert_eq!(report.restored, count);
            assert_eq!(report.remaining, 0);
            assert_eq!(report.failures, 0);
            for path in &originals {
                assert_eq!(fs::read(path).unwrap(), b"Synthetic shortcut bytes; never launched");
            }
            assert_eq!(config.groups[0].items.len(), existing + count);
        }
        let mut collect_sorted = collections.clone();
        let mut restore_sorted = restores.clone();
        collect_sorted.sort_by(f64::total_cmp);
        restore_sorted.sort_by(f64::total_cmp);
        results.push(serde_json::json!({
            "shortcuts": count,
            "existing_items": existing,
            "collect_ms": collections,
            "restore_ms": restores,
            "collect_median_ms": collect_sorted[1],
            "restore_median_ms": restore_sorted[1],
        }));
    }
    println!("NEST_TRANSFER_BENCH {}", serde_json::json!({
        "cases": results,
        "limitations": "Synthetic same-volume filesystem/model cost only; excludes Explorer refresh, watchdog startup and icon extraction."
    }));
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--source', type=Path,
                        help='Read a stable source tree, such as the previous release.')
    parser.add_argument('--warm-registry', action='store_true',
                        help='Prepare an already-durable empty ownership registry before timing.')
    args = parser.parse_args()
    work_root = Path(__file__).resolve().parents[1]
    root = args.source.resolve() if args.source else work_root
    local_tools = work_root.parent / 'tools'
    cargo = (args.cargo or shutil.which('cargo')
             or local_tools / 'cargo' / 'bin' / 'cargo.exe')
    env = os.environ.copy()
    if (local_tools / 'cargo').is_dir():
        env.setdefault('CARGO_HOME', str(local_tools / 'cargo'))
    if (local_tools / 'rustup').is_dir():
        env.setdefault('RUSTUP_HOME', str(local_tools / 'rustup'))
    env['CARGO_TARGET_DIR'] = str(work_root / 'target' / 'transfer-core-benchmark-build')
    env['NEST_TRANSFER_BENCH_WARM'] = '1' if args.warm_registry else '0'
    scratch_parent = work_root / 'target'
    scratch_parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='transfer-core-benchmark-', dir=scratch_parent) as tmp:
        clone = Path(tmp)
        for name in ('Cargo.toml', 'Cargo.lock', 'build.rs'):
            shutil.copy2(root / name, clone / name)
        for name in ('src', 'assets'):
            shutil.copytree(root / name, clone / name)
        with (clone / 'src' / 'main.rs').open('a', encoding='utf-8') as stream:
            stream.write('\n#[cfg(test)]\nmod transfer_core_benchmark;\n')
        batch_api = 'pub fn collect_many_into_with_positions(' in (
            clone / 'src' / 'desktop_transfer.rs').read_text(encoding='utf-8')
        if batch_api:
            collect_block = '''
            let paths: Vec<_> = originals.iter().cloned().map(|path| (path, None)).collect();
            assert_eq!(desktop_transfer::collect_many_into_with_positions(
                &mut config, &layout, &desktop, &paths, 0, None, |_| {},
            ).unwrap(), count);'''
        else:
            collect_block = '''
            for path in &originals {
                assert!(desktop_transfer::collect_into(
                    &mut config, &layout, &desktop, path, 0, None,
                ).unwrap());
            }'''
        (clone / 'src' / 'transfer_core_benchmark.rs').write_text(
            TEST.replace('__COLLECT_BLOCK__', collect_block), encoding='utf-8')
        env['NEST_TRANSFER_BENCH_DIR'] = str(clone / 'SyntheticCases')
        result = subprocess.run(
            [str(cargo), 'test', '--offline', '--locked',
             'transfer_core_benchmark::synthetic_transfer_core_benchmark',
             '--', '--exact', '--nocapture'],
            cwd=clone, env=env, capture_output=True, text=True, timeout=180,
        )
        if result.returncode:
            raise SystemExit((result.stdout + '\n' + result.stderr)[-8000:])
        line = next((line for line in result.stdout.splitlines()
                     if line.startswith('NEST_TRANSFER_BENCH ')), None)
        if line is None:
            raise SystemExit('Benchmark output marker missing.')
        report = json.loads(line.split(' ', 1)[1])
        report['collect_api'] = ('collect_many_into_with_positions' if batch_api
                                 else 'collect_into, one call per selected shortcut')
        report['shell_notifications'] = 'Excluded: callbacks are no-ops.'
        report['registry_state'] = ('existing durable empty ownership registry' if args.warm_registry
                                    else 'new profile with no ownership registry')
        report['version'] = next(line.split('"')[1] for line in
                                 (clone / 'Cargo.toml').read_text().splitlines()
                                 if line.startswith('version ='))
        encoded = json.dumps(report, indent=2)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(encoded + '\n', encoding='utf-8')
        print(encoded)


if __name__ == '__main__':
    main()
