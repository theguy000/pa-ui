use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    #[cfg(windows)]
    window_icon();
}

#[cfg(windows)]
fn window_icon() {
    println!("cargo:rerun-if-changed=assets/icon.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let Some(rc) = windows_sdk_rc() else {
        println!("cargo:warning=Windows SDK rc.exe not found - pa-gpui.exe will have no icon");
        return;
    };

    let res = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("pa-gpui-icon.res");
    let assets_dir = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("assets");
    let built = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("icon.rc")
        .current_dir(&assets_dir)
        .status();

    match built {
        Ok(status) if status.success() => println!("cargo:rustc-link-arg-bins={}", res.display()),
        _ => println!("cargo:warning=rc.exe failed on icon.rc - pa-gpui.exe will have no icon"),
    }
}

#[cfg(windows)]
fn windows_sdk_rc() -> Option<PathBuf> {
    let drive = env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let root = PathBuf::from(format!("{}\\", drive.trim_end_matches('\\')));
    let bin = env::var("WindowsSdkDir")
        .or_else(|_| env::var("WINDOWSSDKDIR"))
        .map(|dir| Path::new(&dir).join("bin"))
        .ok()
        .into_iter()
        .chain(
            ["ProgramFiles(x86)", "ProgramFiles"]
                .iter()
                .filter_map(|var| env::var(var).ok())
                .map(|dir| Path::new(&dir).join("Windows Kits").join("10").join("bin")),
        )
        .chain(std::iter::once(
            root.join("Program Files (x86)")
                .join("Windows Kits")
                .join("10")
                .join("bin"),
        ))
        .find(|dir| dir.is_dir())?;

    let mut versions: Vec<PathBuf> = std::fs::read_dir(&bin)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();

    versions
        .iter()
        .rev()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|rc| rc.is_file())
}
