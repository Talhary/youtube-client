//! Windows: link against the libmpv import library in `third_party/mpv` and copy runtime
//! files (libmpv-2.dll, rustypipe-botguard.exe, ffmpeg.exe if present) next to the binary.
//! Linux: libmpv comes from the system (pkg-config / Flatpak runtime).

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=STREAMPIPE_MPV_DIR");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let mpv_dir = std::env::var("STREAMPIPE_MPV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("third_party").join("mpv"));
    println!("cargo:rustc-link-search=native={}", mpv_dir.display());

    // OUT_DIR is target/<profile>/build/<pkg>/out; the binary lands in target/<profile>.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let bin_dir = out.ancestors().nth(3).unwrap().to_path_buf();
    let copies = [
        mpv_dir.join("libmpv-2.dll"),
        root.join("third_party/botguard/bin/rustypipe-botguard.exe"),
        root.join("third_party/ffmpeg/ffmpeg.exe"),
        root.join("third_party/deno/deno.exe"),
        // The yt-parser resolver (build: third_party/yt-parser-src, go build -tags ytplib -buildmode=c-shared).
        root.join("third_party/ytp/ytp.dll"),
    ];
    for src in copies {
        copy_if_newer(&src, &bin_dir);
    }
    // Persistent yt-dlp workers: bundled Python + yt-dlp + worker.py.
    let worker = root.join("third_party/ytworker");
    if worker.join("worker.py").exists() {
        println!("cargo:rerun-if-changed={}", worker.join("worker.py").display());
        copy_dir_if_newer(&worker, &bin_dir.join("ytworker"));
    } else {
        println!("cargo:warning=third_party/ytworker missing; falling back to yt-dlp.exe");
        copy_if_newer(&root.join("third_party/yt-dlp/yt-dlp.exe"), &bin_dir);
    }

    #[cfg(windows)]
    {
        let icon = root.join("assets/icon.ico");
        if icon.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon.to_str().unwrap());
            res.set("ProductName", "StreamPipe");
            res.set("FileDescription", "StreamPipe");
            let _ = res.compile();
        }
    }
}

fn copy_if_newer(src: &Path, dir: &Path) {
    let Some(name) = src.file_name() else { return };
    if !src.exists() {
        println!("cargo:warning={} not found; the app will look for it at runtime", src.display());
        return;
    }
    println!("cargo:rerun-if-changed={}", src.display());
    let dst = dir.join(name);
    let newer = match (src.metadata().and_then(|m| m.modified()), dst.metadata().and_then(|m| m.modified())) {
        (Ok(a), Ok(b)) => a > b,
        _ => true,
    };
    if newer {
        let _ = std::fs::copy(src, dst);
    }
}

fn copy_dir_if_newer(src: &Path, dst: &Path) {
    let _ = std::fs::create_dir_all(dst);
    let Ok(rd) = std::fs::read_dir(src) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir_if_newer(&p, &dst.join(e.file_name()));
        } else {
            copy_if_newer_quiet(&p, &dst.join(e.file_name()));
        }
    }
}

fn copy_if_newer_quiet(src: &Path, dst: &Path) {
    let newer = match (src.metadata().and_then(|m| m.modified()), dst.metadata().and_then(|m| m.modified())) {
        (Ok(a), Ok(b)) => a > b,
        _ => true,
    };
    if newer {
        let _ = std::fs::copy(src, dst);
    }
}
