# Building and packaging

Build on Windows x64 with a current stable Rust toolchain. Both the GNU and
MSVC Windows x64 targets are supported by the packaging script. The MSVC
toolchain needs its usual C++ build tools. Python 3 is used for release checks.

```powershell
cargo test --locked
cargo fmt --all --check
cargo build --release --locked
python scripts/check-pe-imports.py target/release/nest-desktop.exe
python scripts/check-manifest.py target/release/nest-desktop.exe
python scripts/check-icon-resources.py target/release/nest-desktop.exe
.\scripts\package.ps1 -SkipBuild
```

The application is `target\release\nest-desktop.exe`. Packaging produces a ZIP
and SHA-256 checksum in `dist`. It builds the graphical installer using the
C# compiler in Windows' .NET Framework; a separate .NET SDK is not required.
Extract the complete ZIP before opening the installer.

For an explicit target, pass `--target x86_64-pc-windows-gnu` (or
`x86_64-pc-windows-msvc`) to Cargo and the same value to the packaging script's
`-Target` parameter. Omitting `-SkipBuild` runs the tests and release build.

Keep [third-party notices](../THIRD_PARTY_NOTICES.md) with distributed binaries.
Regenerate them from the locked dependencies and the selected Rust toolchain
when updating dependencies. See the generator's `--help` for its inputs.

Development builds use the same profile as the installed app unless
`NESTDESKTOP_DATA_DIR` points to an isolated directory. Back up the complete
`%LOCALAPPDATA%\NestDesktop` folder before trying changes. Native UI fixtures
also share the controller singleton: do not assume an isolated data directory
allows a fixture to run beside the normal app.
