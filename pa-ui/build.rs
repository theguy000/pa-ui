use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    slint_build::compile("ui/app.slint").unwrap();
    #[cfg(windows)]
    window_icon();
}

/// pa-ui.exe's taskbar / Explorer / Alt-Tab icon comes from a Win32 resource, because
/// Slint 1.18 exposes no `Window::set_icon` (only `SystemTray::set_icon`), so there is no
/// runtime route to it. `assets/icon.rc` names `icon.ico` beside it.
///
/// Hand-run rc.exe rather than pulling `embed-resource`/`winres`: both drag in build
/// dependencies that need `cl.exe` or re-resolve the workspace lockfile, and rc.exe —
/// which already ships with the Windows SDK — is all this needs. No SDK means no icon,
/// not a broken build: the icon is cosmetic.
#[cfg(windows)]
fn window_icon() {
    println!("cargo:rerun-if-changed=assets/icon.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let Some(rc) = windows_sdk_rc() else {
        println!("cargo:warning=Windows SDK rc.exe not found - pa-ui.exe will have no icon");
        return;
    };

    let res = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("pa-ui-icon.res");
    let built = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("assets/icon.rc")
        .current_dir(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .status();

    match built {
        Ok(status) if status.success() => println!("cargo:rustc-link-arg-bins={}", res.display()),
        _ => println!("cargo:warning=rc.exe failed on pa-ui/assets/icon.rc - pa-ui.exe will have no icon"),
    }
}

/// `<kits>\bin\<version>\x64\rc.exe`, newest SDK first.
///
/// `WindowsSdkDir` is set inside a VS developer prompt; `ProgramFiles(x86)` outside one.
/// `SystemDrive` is the fallback of last resort so this still works from a bare
/// environment (build servers, stripped shells) instead of silently skipping the icon.
#[cfg(windows)]
fn windows_sdk_rc() -> Option<PathBuf> {
    // "C:" alone is a *drive-relative* path in Windows; the trailing separator is what
    // makes it an absolute root.
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
            root.join("Program Files (x86)").join("Windows Kits").join("10").join("bin"),
        ))
        .find(|dir| dir.is_dir())?;

    let mut versions: Vec<PathBuf> = std::fs::read_dir(&bin)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    // "10.0.26100.0" sorts after "10.0.19041.0", so the newest SDK is last.
    versions.sort();

    versions
        .iter()
        .rev()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|rc| rc.is_file())
}
