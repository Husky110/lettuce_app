use std::env;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;

const ORT_VERSION: &str = "1.22.0";

fn main() {
    println!("cargo:rerun-if-env-changed=ORT_LIB_LOCATION");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "android" => {
            println!("cargo:warning=Detected Android build, checking ONNX Runtime libraries...");
            setup_android_libs().expect("Failed to setup Android libraries");
        }
        "ios" => {
            println!("cargo:warning=Detected iOS build.");
            if env::var("ORT_LIB_LOCATION").is_err() {
                println!(
                    "cargo:warning=ORT_LIB_LOCATION is not set. iOS builds require a CoreML-enabled ONNX Runtime library location."
                );
            }
        }
        "macos" => {
            println!("cargo:warning=Detected macOS build, preparing ONNX Runtime library...");
            setup_macos_libs().expect("Failed to setup macOS ONNX Runtime library");
        }
        "linux" => {
            println!(
                "cargo:warning=Detected Linux desktop build, preparing ONNX Runtime library..."
            );
            setup_linux_libs().expect("Failed to setup Linux ONNX Runtime library");
        }
        _ => {
            println!(
                "cargo:warning=Detected desktop build for target OS '{}'; ensuring ONNX Runtime resource directory exists.",
                target_os
            );
            ensure_resource_dir().expect("Failed to create ONNX Runtime resource directory");
        }
    }

    tauri_build::build();
}

fn ensure_resource_dir() -> anyhow::Result<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let resource_dir = manifest_dir.join("onnxruntime");
    fs::create_dir_all(&resource_dir)?;
    Ok(resource_dir)
}

fn setup_android_libs() -> anyhow::Result<()> {
    let resource_dir = ensure_resource_dir()?;
    println!(
        "cargo:warning=Ensured ONNX Runtime resource dir at {:?} for Android build",
        resource_dir
    );

    let jni_libs_path = PathBuf::from("gen/android/app/src/main/jniLibs");

    let targets = vec![
        ("arm64-v8a", "jni/arm64-v8a/libonnxruntime.so"),
        ("x86_64", "jni/x86_64/libonnxruntime.so"),
    ];

    let mut missing = false;
    for (arch, _) in &targets {
        let lib_path = jni_libs_path.join(arch).join("libonnxruntime.so");
        if !lib_path.exists() {
            missing = true;
            break;
        }
    }

    if !missing {
        println!("cargo:warning=ONNX Runtime libs already present.");
        return Ok(());
    }

    println!(
        "cargo:warning=Downloading ONNX Runtime Android v{}...",
        ORT_VERSION
    );
    let url = format!(
        "https://repo1.maven.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/{0}/onnxruntime-android-{0}.aar",
        ORT_VERSION
    );

    let response = reqwest::blocking::get(&url)?.bytes()?;
    let reader = Cursor::new(response);
    let mut zip = zip::ZipArchive::new(reader)?;

    for (arch, internal_path) in targets {
        let dest_dir = jni_libs_path.join(arch);
        fs::create_dir_all(&dest_dir)?;

        let dest_file = dest_dir.join("libonnxruntime.so");

        match zip.by_name(internal_path) {
            Ok(mut file) => {
                let mut outfile = fs::File::create(&dest_file)?;
                io::copy(&mut file, &mut outfile)?;
                println!("cargo:warning=Extracted: {:?}", dest_file);
            }
            Err(_) => {
                println!(
                    "cargo:warning=Could not find {} in AAR, skipping...",
                    internal_path
                );
            }
        }
    }

    Ok(())
}

fn setup_linux_libs() -> anyhow::Result<()> {
    let resource_dir = ensure_resource_dir()?;

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch != "x86_64" {
        println!(
            "cargo:warning=Unsupported Linux architecture '{}' for bundled ONNX Runtime; runtime fetch fallback will be used.",
            target_arch
        );
        return Ok(());
    }

    let target_path = resource_dir.join("libonnxruntime.so");

    if let Ok(path) = env::var("ORT_LIB_LOCATION") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let src_dir = Path::new(trimmed);
            copy_linux_so_from_dir(src_dir, &target_path)?;
            println!(
                "cargo:warning=ORT_LIB_LOCATION is set for Linux build ({}); copied ONNX Runtime library into {:?}.",
                trimmed, target_path
            );
            return Ok(());
        }
    }

    if target_path.exists() {
        println!(
            "cargo:warning=Linux ONNX Runtime already present at {:?}",
            target_path
        );
        return Ok(());
    }

    let archive_url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{0}/onnxruntime-linux-x64-{0}.tgz",
        ORT_VERSION
    );
    let lib_path_in_archive = format!(
        "onnxruntime-linux-x64-{}/lib/libonnxruntime.so",
        ORT_VERSION
    );

    println!(
        "cargo:warning=Downloading ONNX Runtime Linux v{}...",
        ORT_VERSION
    );
    let response = reqwest::blocking::get(&archive_url)?.bytes()?;
    extract_tgz_single_file(&response, &lib_path_in_archive, &target_path)?;

    if !target_path.exists() {
        anyhow::bail!(
            "ONNX Runtime library not found after Linux download: {}",
            target_path.display()
        );
    }

    println!("cargo:warning=Extracted: {:?}", target_path);
    Ok(())
}

fn copy_linux_so_from_dir(src_dir: &Path, dest_path: &Path) -> anyhow::Result<()> {
    if !src_dir.exists() {
        anyhow::bail!("ORT_LIB_LOCATION does not exist: {}", src_dir.display());
    }
    if !src_dir.is_dir() {
        anyhow::bail!("ORT_LIB_LOCATION is not a directory: {}", src_dir.display());
    }

    let exact = src_dir.join("libonnxruntime.so");
    if exact.exists() {
        fs::copy(&exact, dest_path)?;
        return Ok(());
    }

    let mut fallback: Option<PathBuf> = None;
    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("libonnxruntime.so") {
            fallback = Some(path);
            break;
        }
    }

    if let Some(source) = fallback {
        fs::copy(source, dest_path)?;
        return Ok(());
    }

    anyhow::bail!(
        "ORT_LIB_LOCATION is missing libonnxruntime.so (or versioned variant): {}",
        src_dir.display()
    )
}

fn setup_macos_libs() -> anyhow::Result<()> {
    let resource_dir = ensure_resource_dir()?;

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let archive_arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => {
            println!(
                "cargo:warning=Unsupported macOS architecture '{}' for bundled ONNX Runtime; runtime fetch fallback will be used.",
                target_arch
            );
            return Ok(());
        }
    };

    if let Ok(path) = env::var("ORT_LIB_LOCATION") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            copy_macos_dylibs_from_dir(Path::new(trimmed), &resource_dir)?;
            validate_macos_ort_dylibs(&resource_dir, archive_arch, false)?;
            println!(
                "cargo:warning=ORT_LIB_LOCATION is set for macOS build ({}); copied ONNX Runtime dylibs into {:?}.",
                trimmed, resource_dir
            );
            return Ok(());
        }
    }

    let dylib_path = resource_dir.join("libonnxruntime.dylib");
    let shared_path = resource_dir.join("libonnxruntime_providers_shared.dylib");
    let coreml_path = resource_dir.join("libonnxruntime_providers_coreml.dylib");
    if dylib_path.exists() {
        match validate_macos_ort_dylibs(&resource_dir, archive_arch, false) {
            Ok(()) => {
                if !shared_path.exists() {
                    println!(
                        "cargo:warning=macOS ONNX Runtime provider shared dylib is missing; embeddings may rely on runtime-fetched providers."
                    );
                }
                if !coreml_path.exists() {
                    println!(
                        "cargo:warning=macOS ONNX Runtime CoreML provider dylib is missing; runtime will use CPU fallback for embeddings."
                    );
                }
                println!(
                    "cargo:warning=macOS ONNX Runtime already present at {:?}",
                    dylib_path
                );
                return Ok(());
            }
            Err(err) => {
                println!(
                    "cargo:warning=Existing macOS ONNX Runtime dylibs are invalid for target arch '{}': {}. Refreshing download.",
                    archive_arch, err
                );
            }
        }
    }

    let archive_url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{0}/onnxruntime-osx-{1}-{0}.tgz",
        ORT_VERSION, archive_arch
    );
    let lib_dir_in_archive = format!("onnxruntime-osx-{}-{}/lib/", archive_arch, ORT_VERSION);

    println!(
        "cargo:warning=Downloading ONNX Runtime macOS v{} ({})...",
        ORT_VERSION, archive_arch
    );
    let response = reqwest::blocking::get(&archive_url)?.bytes()?;
    extract_tgz_dylibs_from_dir(&response, &lib_dir_in_archive, &resource_dir)?;
    validate_macos_ort_dylibs(&resource_dir, archive_arch, false)?;
    if !shared_path.exists() {
        println!(
            "cargo:warning=Downloaded macOS ONNX Runtime does not include libonnxruntime_providers_shared.dylib; embeddings may rely on runtime-fetched providers."
        );
    }
    if !coreml_path.exists() {
        println!(
            "cargo:warning=Downloaded macOS ONNX Runtime does not include libonnxruntime_providers_coreml.dylib; runtime will fall back to CPU for embeddings."
        );
    }
    if dylib_path.exists() {
        println!("cargo:warning=Extracted: {:?}", dylib_path);
    }

    Ok(())
}

fn copy_macos_dylibs_from_dir(src_dir: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    if !src_dir.exists() {
        anyhow::bail!("ORT_LIB_LOCATION does not exist: {}", src_dir.display());
    }
    if !src_dir.is_dir() {
        anyhow::bail!("ORT_LIB_LOCATION is not a directory: {}", src_dir.display());
    }

    let mut copied_count = 0usize;
    let mut has_main_dylib = false;
    let mut has_shared_provider = false;
    let mut has_coreml_provider = false;

    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("dylib") {
            continue;
        }

        let Some(file_name) = path.file_name() else {
            continue;
        };
        let dest_path = dest_dir.join(file_name);
        fs::copy(&path, &dest_path)?;
        copied_count += 1;

        match file_name.to_string_lossy().as_ref() {
            "libonnxruntime.dylib" => has_main_dylib = true,
            "libonnxruntime_providers_shared.dylib" => has_shared_provider = true,
            "libonnxruntime_providers_coreml.dylib" => has_coreml_provider = true,
            _ => {}
        }
    }

    if copied_count == 0 {
        anyhow::bail!(
            "No .dylib files found in ORT_LIB_LOCATION: {}",
            src_dir.display()
        );
    }

    if !has_main_dylib {
        anyhow::bail!(
            "ORT_LIB_LOCATION is missing libonnxruntime.dylib: {}",
            src_dir.display()
        );
    }

    if !has_shared_provider {
        println!(
            "cargo:warning=ORT_LIB_LOCATION does not include libonnxruntime_providers_shared.dylib; embeddings may rely on runtime-fetched providers."
        );
    }

    if !has_coreml_provider {
        println!(
            "cargo:warning=ORT_LIB_LOCATION does not include libonnxruntime_providers_coreml.dylib; runtime will use CPU fallback for embeddings."
        );
    }

    Ok(())
}

fn dylib_supports_arch(path: &Path, expected_arch: &str) -> anyhow::Result<bool> {
    let output = match Command::new("lipo").arg("-archs").arg(path).output() {
        Ok(out) => out,
        Err(err) => {
            println!(
                "cargo:warning=Unable to execute lipo for '{}': {}. Skipping arch validation.",
                path.display(),
                err
            );
            return Ok(true);
        }
    };

    if !output.status.success() {
        println!(
            "cargo:warning=lipo -archs failed for '{}'; skipping arch validation.",
            path.display()
        );
        return Ok(true);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.split_whitespace().any(|arch| arch == expected_arch))
}

fn validate_macos_ort_dylibs(
    dylib_dir: &Path,
    expected_arch: &str,
    require_coreml: bool,
) -> anyhow::Result<()> {
    let main = dylib_dir.join("libonnxruntime.dylib");
    let shared = dylib_dir.join("libonnxruntime_providers_shared.dylib");
    let coreml = dylib_dir.join("libonnxruntime_providers_coreml.dylib");

    if !main.exists() {
        anyhow::bail!("Missing libonnxruntime.dylib in {}", dylib_dir.display());
    }
    if require_coreml && !coreml.exists() {
        anyhow::bail!(
            "Missing libonnxruntime_providers_coreml.dylib in {}",
            dylib_dir.display()
        );
    }

    if !dylib_supports_arch(&main, expected_arch)? {
        anyhow::bail!(
            "Dylib '{}' does not include target arch '{}'",
            main.display(),
            expected_arch
        );
    }

    if shared.exists() && !dylib_supports_arch(&shared, expected_arch)? {
        anyhow::bail!(
            "Dylib '{}' does not include target arch '{}'",
            shared.display(),
            expected_arch
        );
    }

    if coreml.exists() && !dylib_supports_arch(&coreml, expected_arch)? {
        anyhow::bail!(
            "Dylib '{}' does not include target arch '{}'",
            coreml.display(),
            expected_arch
        );
    }

    Ok(())
}

fn extract_tgz_single_file(bytes: &[u8], entry_path: &str, dest_path: &Path) -> anyhow::Result<()> {
    let reader = Cursor::new(bytes);
    let tar = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(tar);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().replace('\\', "/");
        if path != entry_path {
            continue;
        }
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = fs::File::create(dest_path)?;
        io::copy(&mut entry, &mut outfile)?;
        return Ok(());
    }

    anyhow::bail!("Could not find '{}' in ONNX Runtime archive", entry_path)
}

fn extract_tgz_dylibs_from_dir(
    bytes: &[u8],
    entry_dir: &str,
    dest_dir: &Path,
) -> anyhow::Result<()> {
    let reader = Cursor::new(bytes);
    let tar = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(tar);
    let mut extracted_count = 0usize;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().replace('\\', "/");
        if !path.starts_with(entry_dir) || !path.ends_with(".dylib") {
            continue;
        }
        let Some(filename) = Path::new(&path).file_name() else {
            continue;
        };
        fs::create_dir_all(dest_dir)?;
        let out_path = dest_dir.join(filename);
        let mut outfile = fs::File::create(&out_path)?;
        io::copy(&mut entry, &mut outfile)?;
        extracted_count += 1;
    }

    if extracted_count == 0 {
        anyhow::bail!(
            "No .dylib entries found under '{}' in ONNX Runtime archive",
            entry_dir
        );
    }

    Ok(())
}
