use std::process::ExitCode;

use plnk::app::{App, AppError, INSTANCE_MUTEX_NAME, InstanceGuard, RunOptions};
use plnk::settings_store::SettingsStore;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error.report();
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), AppError> {
    let options = RunOptions::from_args(std::env::args_os().skip(1))?;
    // Acquire the singleton before touching the settings or data directories,
    // so a second instance performs no filesystem work and cannot race a first
    // launch's import/save.
    let Some(instance) = InstanceGuard::acquire(INSTANCE_MUTEX_NAME).map_err(AppError::Instance)?
    else {
        eprintln!("plnk: another instance is already running");
        return Ok(());
    };

    let mut store = SettingsStore::default();
    let import = store.load_or_import()?;
    for warning in &import.warnings {
        eprintln!("settings warning: {warning}");
    }
    App::run_with_instance(import.settings, options, instance)
}
