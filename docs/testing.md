# Testing and limitations

NestDesktop is an early alpha for the primary Windows desktop. It uses native
Windows desktop integration, so behaviour can vary with Windows, Explorer and
display settings.

The current test suite covers layout persistence, nested groups, ordering,
colour presets, shortcut ownership, no-overwrite restoration and recovery
records. File-operation tests use synthetic directories rather than a user's
Desktop. Native checks have exercised desktop tiles, popups, the colour panel,
keyboard dismissal and quiet startup. A second-PC user trial also reported
that the app worked well, and identified installer text clipping addressed in
0.2.11; this is not a claim of exhaustive compatibility testing.

## Shortcut behaviour

Selected personal Desktop `.lnk` and `.url` files can be collected. Other
files are references; applications and documents are not physically moved.
Shared Public Desktop shortcuts require an explicit per-app choice before
becoming personal, which also removes those icons from other Windows accounts.
Windows may request administrator approval for that shared-shortcut conversion.

Normal quit returns collected shortcuts. A sleeping helper attempts recovery
after an unexpected app exit, and startup replays recovery records. Existing
files are never overwritten. Conflicts remain recoverable in local storage.
Power loss and Windows shutdown can delay recovery until the next start.
Windows Auto arrange can override saved icon positions.

Layouts and recovery data live in `%LOCALAPPDATA%\NestDesktop`. Back up that
whole folder, including its shortcut files, before testing development builds.
The app does not include a network service, automatic updates or telemetry.

For a bug report, include the app and Windows versions, display scaling, steps
to reproduce, and what you expected. Remove personal paths and other private
information from screenshots or exported layouts before sharing them.
