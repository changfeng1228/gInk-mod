use std::process::ExitCode;

use plnk::app::{App, AppError, RunOptions};
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
    let mut store = SettingsStore::default();
    let import = store.load_or_import()?;
    for warning in &import.warnings {
        eprintln!("settings warning: {warning}");
    }
    App::run_with(import.settings, options)
}
