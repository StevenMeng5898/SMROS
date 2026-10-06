use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const HOST_SHARE_DIR: &str = "host_shared";
const HOST_SHARE_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const FALLBACK_LOGICAL_CPUS: usize = 8;
const MAX_LOGICAL_CPUS: usize = 64;

#[derive(Debug)]
struct SnapshotFile {
    relative: String,
    absolute: PathBuf,
    len: u64,
}

#[derive(Debug)]
struct SkippedFile {
    relative: String,
    reason: &'static str,
    len: u64,
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn relative_string(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if text.is_empty() || text.starts_with('/') || text.contains("//") {
        return None;
    }
    for part in text.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
    }
    Some(text.replace('\\', "/"))
}

fn logical_cpu_count() -> usize {
    println!("cargo:rerun-if-env-changed=SMROS_LOGICAL_CPUS");
    let Ok(value) = env::var("SMROS_LOGICAL_CPUS") else {
        return FALLBACK_LOGICAL_CPUS;
    };
    let trimmed = value.trim();
    match trimmed.parse::<usize>() {
        Ok(parsed) if (1..=MAX_LOGICAL_CPUS).contains(&parsed) => parsed,
        _ => {
            println!(
                "cargo:warning=invalid SMROS_LOGICAL_CPUS={value:?}; using {FALLBACK_LOGICAL_CPUS}"
            );
            FALLBACK_LOGICAL_CPUS
        }
    }
}

fn collect_snapshot(
    root: &Path,
    dir: &Path,
    files: &mut Vec<SnapshotFile>,
    dirs: &mut Vec<String>,
    skipped: &mut Vec<SkippedFile>,
    excluded: Option<&Path>,
) {
    println!("cargo:rerun-if-changed={}", dir.display());

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if excluded.is_some_and(|excluded| excluded == path) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(relative_path) = path.strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative_string(relative_path) else {
            continue;
        };

        if metadata.is_dir() {
            dirs.push(relative);
            collect_snapshot(root, &path, files, dirs, skipped, excluded);
        } else if metadata.is_file() {
            if metadata.len() > HOST_SHARE_MAX_FILE_BYTES {
                skipped.push(SkippedFile {
                    relative,
                    reason: "too-large",
                    len: metadata.len(),
                });
            } else {
                files.push(SnapshotFile {
                    relative,
                    absolute: path,
                    len: metadata.len(),
                });
            }
        }
    }
}

fn target_architecture(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-unknown-none" => Some("aarch64"),
        "riscv64gc-unknown-none-elf" => Some("riscv64"),
        "x86_64-unknown-none" => Some("x86_64"),
        _ => None,
    }
}

fn selected_posix_stage(manifest_dir: &Path, target: &str) -> Result<Option<PathBuf>, String> {
    println!("cargo:rerun-if-env-changed=SMROS_POSIX_STAGE");
    println!("cargo:rerun-if-env-changed=TARGET");
    let Some(architecture) = target_architecture(target) else {
        return Ok(None);
    };
    let host_share_root = manifest_dir.join(HOST_SHARE_DIR);
    let override_path = env::var_os("SMROS_POSIX_STAGE");
    let stage = match &override_path {
        Some(value) => {
            if value.is_empty() {
                return Err("POSIX stage override SMROS_POSIX_STAGE must not be empty".into());
            }
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                manifest_dir.join(path)
            }
        }
        _ if architecture == "aarch64" => host_share_root.join("posixtest"),
        _ => manifest_dir
            .join("target")
            .join("posix")
            .join(architecture)
            .join("stage"),
    };
    // Watch an absent stage as well: publishing it must invalidate Cargo's
    // previous stage-less build. The selection is independent of OUT_DIR.
    println!("cargo:rerun-if-changed={}", stage.display());
    if override_path.is_none() && !stage.exists() && !host_share_root.join("posixtest").exists() {
        return Ok(None);
    }
    if !stage.is_dir() {
        return Err(format!(
            "POSIX stage for {architecture} is unavailable at {}; run `python3 -m scripts.posix.cli build --arch {architecture}` or set SMROS_POSIX_STAGE to a valid stage",
            stage.display()
        ));
    }

    let manifest = stage.join("manifest.tsv");
    let contents = fs::read_to_string(&manifest).map_err(|error| {
        format!(
            "POSIX stage for {architecture} has no readable manifest.tsv at {}: {error}",
            manifest.display()
        )
    })?;
    if contents.lines().next() != Some("SMROS_POSIX_MANIFEST\t1") {
        return Err(format!(
            "POSIX stage has invalid manifest schema: {}",
            manifest.display()
        ));
    }
    let architectures: Vec<&str> = contents
        .lines()
        .filter_map(|line| line.strip_prefix("meta\tarchitecture\t"))
        .collect();
    if architectures.len() != 1 || architectures[0] != architecture {
        return Err(format!(
            "POSIX stage architecture mismatch for target {target}: expected {architecture}, found {:?} in {}",
            architectures,
            manifest.display()
        ));
    }
    Ok(Some(stage))
}

fn linker_script_for_target(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-unknown-none" => Some("linker/kernel.ld"),
        "riscv64gc-unknown-none-elf" => Some("linker/kernel-riscv64.ld"),
        "x86_64-unknown-none" => Some("linker/kernel-x86_64.ld"),
        _ => None,
    }
}

fn linker_arg_is_placement_option(argument: &str) -> bool {
    [
        "-Ttext",
        "-Ttext-segment",
        "-Trodata-segment",
        "-Tldata-segment",
        "-Tdata",
        "-Tbss",
    ]
    .iter()
    .any(|option| argument == *option || argument.starts_with(&format!("{option}=")))
}

fn linker_arg_selects_script(argument: &str) -> bool {
    if argument == "-T" || argument == "--script" {
        return true;
    }
    if linker_arg_is_placement_option(argument) {
        return false;
    }
    if argument
        .strip_prefix("-T")
        .is_some_and(|path| !path.is_empty())
        || argument
            .strip_prefix("--script=")
            .is_some_and(|path| !path.is_empty())
    {
        return true;
    }

    argument
        .strip_prefix("-Wl,")
        .is_some_and(|arguments| arguments.split(',').any(linker_arg_selects_script))
}

fn rustflags_select_linker_script(encoded_rustflags: &str) -> bool {
    let mut rustflags = encoded_rustflags.split('\x1f');
    while let Some(flag) = rustflags.next() {
        let codegen_option = if flag == "-C" {
            rustflags.next()
        } else {
            flag.strip_prefix("-C")
        };
        let Some(codegen_option) = codegen_option else {
            continue;
        };

        if let Some(argument) = codegen_option.strip_prefix("link-arg=") {
            if linker_arg_selects_script(argument) {
                return true;
            }
        } else if let Some(arguments) = codegen_option.strip_prefix("link-args=") {
            if arguments.split_whitespace().any(linker_arg_selects_script) {
                return true;
            }
        }
    }
    false
}

fn configure_linker() {
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");

    let target = env::var("TARGET").unwrap();
    let Some(script) = linker_script_for_target(&target) else {
        return;
    };
    println!("cargo:rerun-if-changed={script}");

    let rustflags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if !rustflags_select_linker_script(&rustflags) {
        println!("cargo:rustc-link-arg=-T{script}");
    }
}

fn main() {
    configure_linker();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let host_share_root = manifest_dir.join(HOST_SHARE_DIR);
    println!("cargo:rerun-if-changed={}", host_share_root.display());

    let selected_stage =
        selected_posix_stage(&manifest_dir, &target).unwrap_or_else(|error| panic!("{error}"));

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let legacy_stage = host_share_root.join("posixtest");
    if host_share_root.is_dir() {
        collect_snapshot(
            &host_share_root,
            &host_share_root,
            &mut files,
            &mut dirs,
            &mut skipped,
            selected_stage.as_ref().map(|_| legacy_stage.as_path()),
        );
    }

    if let Some(stage) = selected_stage {
        dirs.push("posixtest".into());
        let mut stage_dirs = Vec::new();
        let mut stage_files = Vec::new();
        let mut stage_skipped = Vec::new();
        collect_snapshot(
            &stage,
            &stage,
            &mut stage_files,
            &mut stage_dirs,
            &mut stage_skipped,
            None,
        );
        for dir in stage_dirs {
            dirs.push(format!("posixtest/{dir}"));
        }
        for mut file in stage_files {
            file.relative = format!("posixtest/{}", file.relative);
            files.push(file);
        }
        for mut file in stage_skipped {
            file.relative = format!("posixtest/{}", file.relative);
            skipped.push(file);
        }
    }

    dirs.sort();
    dirs.dedup();
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    skipped.sort_by(|a, b| a.relative.cmp(&b.relative));

    let mut generated = String::new();
    generated.push_str(
        "// Generated by build.rs from host_shared/ and the target POSIX stage. Do not edit.\n",
    );
    generated.push_str("#[derive(Clone, Copy, Debug)]\n");
    generated.push_str("pub struct HostShareFile {\n");
    generated.push_str("    pub path: &'static str,\n");
    generated.push_str("    pub data: &'static [u8],\n");
    generated.push_str("}\n\n");
    generated.push_str("#[derive(Clone, Copy, Debug)]\n");
    generated.push_str("pub struct HostShareSkipped {\n");
    generated.push_str("    pub path: &'static str,\n");
    generated.push_str("    pub reason: &'static str,\n");
    generated.push_str("    pub size: usize,\n");
    generated.push_str("}\n\n");
    generated.push_str("pub const HOST_SHARE_ROOT: &str = \"host_shared\";\n");
    generated.push_str(&format!(
        "pub const HOST_SHARE_MAX_FILE_BYTES: usize = {};\n",
        HOST_SHARE_MAX_FILE_BYTES
    ));
    generated.push_str("pub static HOST_SHARE_DIRS: &[&str] = &[\n");
    for dir in &dirs {
        generated.push_str("    ");
        generated.push_str(&rust_string_literal(dir));
        generated.push_str(",\n");
    }
    generated.push_str("];\n\n");
    generated.push_str("pub static HOST_SHARE_FILES: &[HostShareFile] = &[\n");
    for file in &files {
        let absolute = file.absolute.to_string_lossy();
        generated.push_str("    HostShareFile { path: ");
        generated.push_str(&rust_string_literal(&file.relative));
        generated.push_str(", data: include_bytes!(");
        generated.push_str(&rust_string_literal(&absolute));
        generated.push_str(") },\n");
        println!(
            "cargo:metadata=host_shared include {} ({} bytes)",
            file.relative, file.len
        );
    }
    generated.push_str("];\n\n");
    generated.push_str("pub static HOST_SHARE_SKIPPED: &[HostShareSkipped] = &[\n");
    for file in &skipped {
        generated.push_str("    HostShareSkipped { path: ");
        generated.push_str(&rust_string_literal(&file.relative));
        generated.push_str(", reason: ");
        generated.push_str(&rust_string_literal(file.reason));
        generated.push_str(", size: ");
        generated.push_str(&file.len.to_string());
        generated.push_str(" },\n");
        println!(
            "cargo:metadata=host_shared skip {}: {} ({} bytes)",
            file.relative, file.reason, file.len
        );
    }
    generated.push_str("];\n");

    fs::write(out_dir.join("host_share.rs"), generated).unwrap();

    let logical_cpus = logical_cpu_count();
    println!("cargo:metadata=SMROS logical CPUs configured: {logical_cpus}");
    let build_config = format!("// Generated by build.rs. Do not edit.\n{logical_cpus}usize\n");
    fs::write(out_dir.join("build_config.rs"), build_config).unwrap();

    compile_linuxcompat(&manifest_dir, &target);
}

fn compiler_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("-dumpversion")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn compile_linuxcompat(manifest_dir: &Path, target: &str) {
    println!("cargo:rerun-if-changed=src/user_level/drivers/linuxcompat");
    println!("cargo:rerun-if-changed=include/linux");
    println!("cargo:rerun-if-changed=include/asm");
    println!("cargo:rustc-check-cfg=cfg(smros_linuxcompat)");

    let src_dir = manifest_dir.join("src/user_level/drivers/linuxcompat");
    let include_dir = manifest_dir.join("include");
    if !src_dir.join("edu.c").is_file() || !include_dir.join("linux/pci.h").is_file() {
        return;
    }

    let (compiler, extra_flags): (&str, &[&str]) = match target {
        "aarch64-unknown-none" => (
            "aarch64-linux-gnu-gcc",
            &[
                "-mgeneral-regs-only",
                "-mno-outline-atomics",
                "-march=armv8-a",
                "-mabi=lp64",
            ],
        ),
        "x86_64-unknown-none" => (
            "x86_64-linux-gnu-gcc",
            &["-mno-red-zone", "-mno-sse", "-mno-mmx", "-mno-avx"],
        ),
        _ => return,
    };

    if !compiler_exists(compiler) {
        println!("cargo:warning=linuxcompat skipped: {compiler} not found");
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let sources = ["compat.c", "pci.c", "edu.c"];
    let mut objects = Vec::new();
    for source in sources {
        let src = src_dir.join(source);
        let obj = out_dir.join(format!("{source}.o"));
        let mut cmd = std::process::Command::new(compiler);
        cmd.arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .arg("-ffreestanding")
            .arg("-fno-builtin")
            .arg("-fno-stack-protector")
            .arg("-fno-asynchronous-unwind-tables")
            .arg("-fno-unwind-tables")
            .arg("-fno-pic")
            .arg("-fno-pie")
            .arg("-nostdinc")
            .arg("-std=gnu11")
            .arg("-O2")
            .arg("-I")
            .arg(&include_dir);
        for flag in extra_flags {
            cmd.arg(flag);
        }
        let status = cmd
            .status()
            .unwrap_or_else(|error| panic!("spawn {compiler}: {error}"));
        if !status.success() {
            panic!("linuxcompat compile failed for {source}");
        }
        objects.push(obj);
    }

    let lib = out_dir.join("libsmros_linuxcompat.a");
    let archiver = compiler
        .strip_suffix("gcc")
        .map(|prefix| format!("{prefix}ar"))
        .unwrap_or_else(|| "ar".into());
    let mut ar = std::process::Command::new(&archiver);
    ar.arg("crs").arg(&lib);
    for obj in &objects {
        ar.arg(obj);
    }
    let status = ar
        .status()
        .unwrap_or_else(|error| panic!("spawn {archiver}: {error}"));
    if !status.success() {
        panic!("linuxcompat archive failed");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=smros_linuxcompat");
    println!("cargo:rustc-cfg=smros_linuxcompat");
}
