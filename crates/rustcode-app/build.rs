use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" || env::var_os("CARGO_FEATURE_DESKTOP").is_none() {
        return;
    }

    let arch_dir = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => return,
    };

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };

    // Locate `<target>/<profile>/build` regardless of whether Cargo uses flat
    // (`build/rustcode-app-<hash>/out`) or nested (`build/rustcode-app/<hash>/out`)
    // artifact directories.
    let Some(build_dir) = out_dir
        .ancestors()
        .find(|p| p.file_name() == Some(OsStr::new("build")))
    else {
        return;
    };
    let Some(profile_dir) = build_dir.parent() else {
        return;
    };

    let Some(src_dll) = find_webview2_loader(build_dir, arch_dir) else {
        // Re-check if webview2-com-sys had not finished running yet on a cold build.
        println!("cargo:rerun-if-changed={}", build_dir.display());
        return;
    };

    println!("cargo:rerun-if-changed={}", src_dll.display());

    let dest_dll = profile_dir.join("WebView2Loader.dll");
    if should_copy(&src_dll, &dest_dll) {
        if let Err(err) = fs::copy(&src_dll, &dest_dll) {
            println!(
                "cargo:warning=Failed to copy WebView2Loader.dll from '{}' to '{}': {err}",
                src_dll.display(),
                dest_dll.display()
            );
        }
    }
}

fn find_webview2_loader(build_dir: &Path, arch_dir: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(build_dir).ok()?;
    let mut candidates: Vec<(SystemTime, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if name.starts_with("webview2-com-sys-") {
            let dll = path.join("out").join(arch_dir).join("WebView2Loader.dll");
            if dll.is_file() {
                let mtime = fs::metadata(&dll)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                candidates.push((mtime, dll));
            }
        } else if name == "webview2-com-sys" {
            if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let dll = sub.path().join("out").join(arch_dir).join("WebView2Loader.dll");
                    if dll.is_file() {
                        let mtime = fs::metadata(&dll)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        candidates.push((mtime, dll));
                    }
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().map(|(_, p)| p).next()
}

fn should_copy(src: &Path, dest: &Path) -> bool {
    let Ok(src_meta) = fs::metadata(src) else {
        return false;
    };
    let Ok(dest_meta) = fs::metadata(dest) else {
        return true;
    };
    if src_meta.len() != dest_meta.len() {
        return true;
    }
    match (src_meta.modified(), dest_meta.modified()) {
        (Ok(s), Ok(d)) => s > d,
        _ => true,
    }
}
