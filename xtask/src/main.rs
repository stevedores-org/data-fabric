use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const WORKER_BUILD_VERSION: &str = "0.8.3";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("build-worker") => build_worker(),
        Some(other) => Err(format!("unknown command `{other}`")),
        None => Err("usage: cargo run --package xtask -- build-worker".to_string()),
    }
}

fn build_worker() -> Result<()> {
    let repo = env::current_dir().map_err(|err| format!("read current directory: {err}"))?;
    let tools_dir = repo.join(".worker-tools");
    let cargo_home = cargo_home();
    let cargo_bin = cargo_home.join("bin");
    let rustup = cargo_bin.join("rustup");
    let cargo = cargo_bin.join("cargo");
    let worker_build = tools_dir.join("bin").join(exe_name("worker-build"));

    prepend_path(&cargo_bin)?;

    ensure_rustup(&rustup)?;
    run_cmd(&rustup, ["target", "add", WASM_TARGET])?;
    run_cmd(
        &cargo,
        [
            "install",
            "worker-build",
            "--version",
            WORKER_BUILD_VERSION,
            "--locked",
            "--force",
            "--root",
            path_str(&tools_dir)?,
        ],
    )?;
    run_cmd(&worker_build, ["--release"])?;

    Ok(())
}

fn ensure_rustup(rustup: &Path) -> Result<()> {
    if rustup.exists() {
        return Ok(());
    }

    let sh = which("sh").ok_or_else(|| "cannot find `sh` to install rustup".to_string())?;
    let curl = which("curl").ok_or_else(|| "cannot find `curl` to install rustup".to_string())?;
    let installer = Command::new(curl)
        .args(["-sSf", "https://sh.rustup.rs"])
        .output()
        .map_err(|err| format!("download rustup installer: {err}"))?;
    if !installer.status.success() {
        return Err(format!(
            "download rustup installer failed with status {}",
            installer.status
        ));
    }

    let mut child = Command::new(sh)
        .args(["-s", "--", "-y", "--profile", "minimal"])
        .env("RUSTUP_INIT_SKIP_PATH_CHECK", "yes")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| format!("start rustup installer: {err}"))?;
    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "open rustup installer stdin".to_string())?;
        stdin
            .write_all(&installer.stdout)
            .map_err(|err| format!("feed rustup installer: {err}"))?;
    }
    let status = child
        .wait()
        .map_err(|err| format!("wait for rustup installer: {err}"))?;
    if !status.success() {
        return Err(format!("rustup installer failed with status {status}"));
    }

    Ok(())
}

fn run_cmd<I, S>(program: &Path, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|err| format!("run `{}`: {err}", program.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{}` failed with status {status}",
            program.display()
        ))
    }
}

fn cargo_home() -> PathBuf {
    env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

fn prepend_path(path: &Path) -> Result<()> {
    let current = env::var_os("PATH").unwrap_or_default();
    let mut paths = env::split_paths(&current).collect::<Vec<_>>();
    paths.insert(0, path.to_path_buf());
    let joined = env::join_paths(paths).map_err(|err| format!("join PATH: {err}"))?;
    env::set_var("PATH", joined);
    Ok(())
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn exe_name(name: &str) -> OsString {
    #[cfg(windows)]
    {
        OsString::from(format!("{name}.exe"))
    }
    #[cfg(not(windows))]
    {
        OsString::from(name)
    }
}

fn which(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}
