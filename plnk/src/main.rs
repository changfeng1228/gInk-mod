use std::process::ExitCode;

use plnk::settings_store::SettingsStore;

fn main() -> ExitCode {
    let mut store = SettingsStore::default();
    match store.load_or_import() {
        Ok(import) => {
            for warning in &import.warnings {
                eprintln!("warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
