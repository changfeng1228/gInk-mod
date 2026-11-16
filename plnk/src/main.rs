#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use plnk::app::{
    App, AppError, INSTANCE_MUTEX_NAME, InstanceDecision, InstanceGuard, RunOptions,
    decide_instance, report_already_running,
};
use plnk::settings_store::SettingsStore;

fn main() -> ExitCode {
    // First statement of the process: the release build is a GUI-subsystem
    // executable with no visible stderr, so a startup panic must reach the
    // crash log installed here or it would leave no trace at all.
    plnk::telemetry::install_panic_hook();
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
    // so a second instance performs no settings/filesystem work and cannot race
    // a first launch's import/save.
    let acquired = InstanceGuard::acquire(INSTANCE_MUTEX_NAME).map_err(AppError::Instance)?;
    match decide_instance(acquired) {
        InstanceDecision::Primary(instance) => run_primary(instance, options),
        InstanceDecision::AlreadyRunning => {
            report_already_running();
            Ok(())
        }
    }
}

fn run_primary(instance: InstanceGuard, options: RunOptions) -> Result<(), AppError> {
    let mut store = SettingsStore::default();
    let import = store.load_or_import()?;
    for warning in &import.warnings {
        eprintln!("settings warning: {warning}");
    }
    App::run_with_instance(import.settings, options, instance)
}
