use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/icon_red.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let version = std::env::var("CARGO_PKG_VERSION")?;
    let mut numeric = version.split('.').map(|part| part.parse::<u16>());
    let major = numeric.next().ok_or("missing major version")??;
    let minor = numeric.next().ok_or("missing minor version")??;
    let patch = numeric.next().ok_or("missing patch version")??;
    if numeric.next().is_some() {
        return Err("plnk VERSIONINFO requires major.minor.patch".into());
    }
    let dotted = format!("{major}.{minor}.{patch}.0");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let icon = manifest
        .join("assets/icons/icon_red.ico")
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let rc = format!(
        r#"#include <windows.h>
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE VFT2_UNKNOWN
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "gInk / plnk contributors\0"
      VALUE "FileDescription", "plnk annotation\0"
      VALUE "FileVersion", "{dotted}\0"
      VALUE "InternalName", "plnk\0"
      VALUE "LegalCopyright", "gInk MIT License; Copyright (c) 2016-2020 Weizhi Nai\0"
      VALUE "OriginalFilename", "plnk.exe\0"
      VALUE "ProductName", "plnk\0"
      VALUE "ProductVersion", "{dotted}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc_path = out_dir.join("plnk-version.rc");
    let res_path = out_dir.join("plnk-version.res");
    fs::write(&rc_path, rc)?;
    let rc_exe = find_rc_exe().ok_or(
        "rc.exe not found; install the Windows SDK from VS 2022 Build Tools or set RC to its path",
    )?;
    let mut command = Command::new(&rc_exe);
    command.arg("/nologo");
    // Cargo does not inherit a Developer Command Prompt. Supply the include
    // directories belonging to this exact SDK, not a hard-coded SDK version.
    if let Some(version_dir) = rc_exe.parent().and_then(Path::parent)
        && let Some(bin_dir) = version_dir.parent()
        && let Some(sdk_root) = bin_dir.parent()
        && let Some(version_name) = version_dir.file_name()
    {
        let include_root = sdk_root.join("Include").join(version_name);
        for subdirectory in ["um", "shared"] {
            let directory = include_root.join(subdirectory);
            if directory.is_dir() {
                command.arg(format!("/i{}", directory.display()));
            }
        }
    }
    let status = command
        .arg(format!("/fo{}", res_path.display()))
        .arg(&rc_path)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with {status}", rc_exe.display()).into());
    }
    println!("cargo:rustc-link-arg-bins={}", res_path.display());
    Ok(())
}

fn find_rc_exe() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("RC") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    let root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64").join("rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}
