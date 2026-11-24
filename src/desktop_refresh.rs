//! Publish completed file returns promptly; placement is advisory and separate
//! from the durable no-overwrite recovery transaction.
use crate::{desktop_positions, desktop_transfer, model::Config, shortcuts::DesktopPosition};
use std::path::{Path, PathBuf};

pub type Positions = Vec<(PathBuf, DesktopPosition)>;
pub struct RestoreOutcome {
    pub result: Result<desktop_transfer::Report, String>,
    pub positions: Positions,
}

pub fn restore_all(config: &mut Config, layout: &Path, forget: bool) -> RestoreOutcome {
    let mut positions = Vec::new();
    let result = desktop_transfer::restore_all_with_callback(config, layout, forget, |entry| {
        let path = PathBuf::from(&entry.original);
        desktop_positions::notify_created(std::slice::from_ref(&path));
        if let Some(position) = entry.position {
            positions.push((path, position));
        }
    });
    RestoreOutcome { result, positions }
}

pub fn restore_orphans(config: &Config, layout: &Path) -> RestoreOutcome {
    let mut positions = Vec::new();
    let result = desktop_transfer::restore_orphans_with_callback(config, layout, |entry| {
        let path = PathBuf::from(&entry.original);
        desktop_positions::notify_created(std::slice::from_ref(&path));
        if let Some(position) = entry.position {
            positions.push((path, position));
        }
    });
    RestoreOutcome { result, positions }
}

/// Headless recovery/restore has no UI timer. Retry only this finite returned
/// batch while Explorer incorporates its notifications; never scan the desktop.
pub fn finish_positions_blocking(mut pending: Positions) -> Vec<String> {
    if pending.is_empty() {
        return Vec::new();
    }
    let session = match desktop_positions::DesktopSession::connect() {
        Ok(session) => session,
        Err(error) => return vec![error],
    };
    let mut warnings = Vec::new();
    for attempt in 0..=3 {
        match session.restore_positions(&pending) {
            Ok(report) => {
                pending = report.pending;
                warnings = report.warnings;
                if pending.is_empty() {
                    return warnings;
                }
            }
            Err(error) => {
                warnings = vec![error];
            }
        }
        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    warnings.push(format!(
        "Explorer has not restored {} saved icon positions yet.",
        pending.len()
    ));
    warnings
}
