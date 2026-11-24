//! A sleeping companion restores collected shortcuts after the organizer exits.
//!
//! The main process owns a per-profile mutex throughout startup, collection and
//! shutdown. The helper must open the exact parent and signal readiness before
//! collection is allowed. It then waits on that process handle, without polling
//! or initializing the desktop UI. A replacement main process can take over the
//! profile before this helper runs; in that case its startup recovery is in
//! charge and the obsolete helper does nothing.

use std::{
    ffi::{c_void, OsStr},
    marker::PhantomData,
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use windows::{
    core::{HRESULT, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        System::Threading::{
            GetCurrentProcess, GetProcessTimes, OpenEventW, OpenProcess, ReleaseMutex, SetEvent,
            WaitForSingleObject, CREATE_NO_WINDOW, EVENT_MODIFY_STATE, INFINITE,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
};

// Null security attributes use the current user's default ACL and avoid adding
// the broad Win32_Security feature just for these two constructors.
#[link(name = "kernel32")]
extern "system" {
    #[link_name = "CreateMutexW"]
    fn create_mutex(attributes: *const c_void, owner: i32, name: *const u16) -> *mut c_void;
    #[link_name = "CreateEventW"]
    fn create_event(
        attributes: *const c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> *mut c_void;
}

const READY_TIMEOUT_MS: u32 = 5_000;
static READY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The guard must stay on its acquiring thread: Win32 mutex ownership belongs
/// to a thread rather than to a HANDLE or Rust value.
pub(crate) struct LifetimeGuard {
    handle: OwnedHandle,
    _same_thread: PhantomData<Rc<()>>,
}

impl LifetimeGuard {
    /// Hold this on the main/UI thread until after normal shortcut restoration.
    /// Acquire it before startup restoration, layout reads and managed recapture.
    pub(crate) fn acquire(data: &Path) -> Result<Self, String> {
        Self::acquire_with_timeout(data, READY_TIMEOUT_MS)?.ok_or_else(|| {
            "Another NestDesktop process or shortcut recovery is using this profile.".into()
        })
    }

    fn acquire_with_timeout(data: &Path, timeout_ms: u32) -> Result<Option<Self>, String> {
        let name = wide(&profile_mutex_name(data)?);
        unsafe {
            let raw = create_mutex(std::ptr::null(), 0, name.as_ptr());
            if raw.is_null() {
                return Err(format!(
                    "Could not create the shortcut recovery lock: {}",
                    windows::core::Error::from_win32()
                ));
            }
            let handle = OwnedHandle(HANDLE(raw));
            let status = WaitForSingleObject(handle.0, timeout_ms);
            if status == WAIT_OBJECT_0 || status == WAIT_ABANDONED {
                Ok(Some(Self {
                    handle,
                    _same_thread: PhantomData,
                }))
            } else if status == WAIT_TIMEOUT {
                Ok(None)
            } else {
                Err(format!(
                    "Could not acquire the shortcut recovery lock: {}",
                    windows::core::Error::from_win32()
                ))
            }
        }
    }
}

impl Drop for LifetimeGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle.0);
        }
    }
}

/// A creation time distinguishes the exact parent from a reused Windows PID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParentIdentity {
    pub pid: u32,
    pub created: u64,
}

impl ParentIdentity {
    fn argument(self) -> String {
        format!("--watch-parent={}:{:016x}", self.pid, self.created)
    }

    fn parse(value: &str) -> Result<Self, String> {
        let (pid, created) = value
            .split_once(':')
            .ok_or("The shortcut recovery parent identity is invalid.")?;
        let pid = pid
            .parse::<u32>()
            .map_err(|_| "The shortcut recovery parent PID is invalid.")?;
        let created = u64::from_str_radix(created, 16)
            .map_err(|_| "The shortcut recovery parent creation time is invalid.")?;
        if pid == 0 || created == 0 {
            return Err("The shortcut recovery parent identity is empty.".into());
        }
        Ok(Self { pid, created })
    }
}

unsafe fn process_creation(process: HANDLE) -> Result<u64, String> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)
        .map_err(|error| format!("Could not inspect the recovery parent: {error}"))?;
    Ok(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
}

pub(crate) fn current_identity() -> Result<ParentIdentity, String> {
    unsafe {
        Ok(ParentIdentity {
            pid: std::process::id(),
            created: process_creation(GetCurrentProcess())?,
        })
    }
}

/// Keep this in App. Drop closes only our process handle; it deliberately does
/// not terminate the companion that is waiting to restore after our exit.
pub(crate) struct Watchdog {
    child: Child,
}

impl Watchdog {
    /// Check at collection events, not on an idle timer. If this returns false,
    /// restore current collections or establish a new ready helper before moves.
    pub(crate) fn is_running(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| format!("Could not inspect shortcut recovery: {error}"))
    }
}

/// Call while holding LifetimeGuard and before moving the first shortcut.
/// Successful return means the helper opened and verified this exact process.
pub(crate) fn spawn_watchdog(data: &Path) -> Result<Watchdog, String> {
    let parent = current_identity()?;
    let sequence = READY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let ready_name = format!(
        "Local\\NestDesktop.RecoveryReady.{}.{:016x}.{sequence}",
        parent.pid, parent.created
    );
    let name = wide(&ready_name);
    let ready = unsafe {
        let raw = create_event(std::ptr::null(), 1, 0, name.as_ptr());
        if raw.is_null() {
            return Err(format!(
                "Could not create the shortcut recovery ready signal: {}",
                windows::core::Error::from_win32()
            ));
        }
        OwnedHandle(HANDLE(raw))
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the shortcut recovery program: {error}"))?;
    let mut child = Command::new(executable)
        .arg(parent.argument())
        .arg(format!("--watch-ready={ready_name}"))
        .env("NESTDESKTOP_DATA_DIR", resolved_profile(data)?)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|error| format!("Could not start shortcut recovery: {error}"))?;
    let status = unsafe { WaitForSingleObject(ready.0, READY_TIMEOUT_MS) };
    let exited = child
        .try_wait()
        .map_err(|error| format!("Could not confirm shortcut recovery readiness: {error}"))?;
    if status != WAIT_OBJECT_0 || exited.is_some() {
        // The child is ours and has not acknowledged readiness. Stop just this
        // failed helper; collection has not yet been permitted by this API.
        if child.kill().is_ok() {
            // Do not hang the organizer if process termination itself stalls.
            if unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle()), 1_000) } == WAIT_OBJECT_0
            {
                let _ = child.wait();
            }
        }
        return Err(
            "Shortcut recovery did not become ready. No new Desktop shortcuts were moved.".into(),
        );
    }
    Ok(Watchdog { child })
}

/// Run at the very start of main, before COM, window classes or singleton logic.
/// The helper profile must be explicit; malformed helper invocations never open
/// the ordinary app or fall back to another profile.
pub(crate) fn run_if_requested() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args
        .iter()
        .any(|arg| arg.to_string_lossy().starts_with("--watch-parent"))
    {
        return None;
    }
    let data = std::env::var_os("NESTDESKTOP_DATA_DIR").map(PathBuf::from);
    let result = (|| {
        let data = data
            .as_ref()
            .ok_or("Shortcut recovery requires an explicit data directory.")?;
        if data.as_os_str().is_empty() {
            return Err("Shortcut recovery data directory is empty.".into());
        }
        let parents: Vec<_> = args
            .iter()
            .filter_map(|arg| {
                arg.to_str()
                    .and_then(|arg| arg.strip_prefix("--watch-parent="))
            })
            .collect();
        if parents.len() != 1 {
            return Err("Shortcut recovery requires exactly one parent identity.".into());
        }
        let parent = ParentIdentity::parse(parents[0])?;
        let ready = args.iter().find_map(|arg| {
            arg.to_str()
                .and_then(|arg| arg.strip_prefix("--watch-ready="))
        });
        watch_parent(parent, data, ready)
    })();
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            if let Some(data) = data.filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(&data);
                let _ = std::fs::write(data.join("recovery-error.txt"), error);
            }
            Some(2)
        }
    }
}

fn watch_parent(parent: ParentIdentity, data: &Path, ready: Option<&str>) -> Result<(), String> {
    unsafe {
        match OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            parent.pid,
        ) {
            Ok(process) => {
                let process = OwnedHandle(process);
                if process_creation(process.0)? != parent.created {
                    return Err(
                        "The recovery parent identity changed. No shortcuts were moved.".into(),
                    );
                }
                if let Some(name) = ready {
                    if !name.starts_with("Local\\NestDesktop.RecoveryReady.") {
                        return Err("Shortcut recovery ready signal is invalid.".into());
                    }
                    let name = wide(name);
                    let event = OwnedHandle(
                        OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())).map_err(
                            |error| {
                                format!(
                                    "Could not open the shortcut recovery ready signal: {error}"
                                )
                            },
                        )?,
                    );
                    SetEvent(event.0).map_err(|error| {
                        format!("Could not acknowledge shortcut recovery: {error}")
                    })?;
                }
                if WaitForSingleObject(process.0, INFINITE) != WAIT_OBJECT_0 {
                    return Err(
                        "Could not wait for the organizer to finish. No shortcuts were moved."
                            .into(),
                    );
                }
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
                // The parent ended before opening its handle. A successful
                // spawn handshake could not have returned in that parent.
            }
            Err(error) => {
                return Err(format!(
                    "Could not monitor the shortcut recovery parent: {error}"
                ))
            }
        }
    }
    let Some(_guard) = LifetimeGuard::acquire_with_timeout(data, 0)? else {
        // A replacement organizer owns startup recovery and any new moves.
        return Ok(());
    };
    crate::quiet_restore(data)
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

fn resolved_profile(data: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(data)
        .map_err(|error| format!("Could not resolve the recovery profile: {error}"))?;
    let mut probe = absolute.clone();
    let mut missing = Vec::new();
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(&probe) {
            for component in missing.iter().rev() {
                resolved.push(component);
            }
            return Ok(resolved);
        }
        let Some(name) = probe.file_name().map(|name| name.to_owned()) else {
            return Ok(absolute);
        };
        missing.push(name);
        if !probe.pop() {
            return Ok(absolute);
        }
    }
}

fn profile_mutex_name(data: &Path) -> Result<String, String> {
    let path = resolved_profile(data)?;
    let mut key = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = key.strip_prefix(r"\\?\UNC\") {
        key = format!(r"\\{rest}");
    } else if let Some(rest) = key.strip_prefix(r"\\?\") {
        key = rest.to_string();
    }
    let key = key.trim_end_matches('\\').to_lowercase();
    // Stable across compiler versions/processes; collisions can only serialize
    // unrelated profiles, never change which paths quiet_restore receives.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in key.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    Ok(format!("Local\\NestDesktop.Profile.v1.{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_identity_roundtrips_and_rejects_empty_or_ambiguous_values() {
        let parent = ParentIdentity {
            pid: 42,
            created: 0x01db123456789abc,
        };
        assert_eq!(
            ParentIdentity::parse(parent.argument().strip_prefix("--watch-parent=").unwrap())
                .unwrap(),
            parent
        );
        for bad in [
            "",
            "42",
            "0:1",
            "42:0",
            "42:nothex",
            "42:1:2",
            "-1:1",
            "4294967296:1",
        ] {
            assert!(ParentIdentity::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn profile_lock_key_is_stable_across_case_and_separators_for_missing_directories() {
        let first = Path::new(r"C:\NestDesktop-recovery-test\missing-profile");
        let second = Path::new("c:/nestdesktop-recovery-test/MISSING-PROFILE/");
        assert_eq!(
            profile_mutex_name(first).unwrap(),
            profile_mutex_name(second).unwrap()
        );
        assert_ne!(
            profile_mutex_name(first).unwrap(),
            profile_mutex_name(Path::new(r"C:\NestDesktop-recovery-test\other-profile")).unwrap()
        );
    }

    #[test]
    fn profile_lock_excludes_other_threads_and_releases_on_drop() {
        let data = PathBuf::from(format!(
            "C:\\NestDesktop-recovery-lock-test-{}-{}",
            std::process::id(),
            READY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let guard = LifetimeGuard::acquire(&data).unwrap();
        let other = data.clone();
        assert!(std::thread::spawn(move || {
            LifetimeGuard::acquire_with_timeout(&other, 0)
                .unwrap()
                .is_none()
        })
        .join()
        .unwrap());
        drop(guard);
        assert!(std::thread::spawn(move || {
            LifetimeGuard::acquire_with_timeout(&data, 0)
                .unwrap()
                .is_some()
        })
        .join()
        .unwrap());
    }

    #[test]
    fn current_parent_identity_has_this_pid_and_a_real_creation_time() {
        let parent = current_identity().unwrap();
        assert_eq!(parent.pid, std::process::id());
        assert!(parent.created > 0);
    }
}
