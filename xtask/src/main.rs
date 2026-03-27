use std::path::PathBuf;
use std::process::{Command, exit};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must have a parent dir")
        .to_owned()
}

fn cargo() -> Command {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root());
    cmd
}

fn run(mut cmd: Command) {
    let status = cmd.status().expect("failed to run command");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn task_check() {
    println!("==> cargo fmt --check");
    let mut c = cargo();
    c.args(["fmt", "--all", "--", "--check"]);
    run(c);

    println!("==> cargo clippy");
    let mut c = cargo();
    c.args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    run(c);
}

fn task_fmt() {
    println!("==> cargo fmt");
    let mut c = cargo();
    c.args(["fmt", "--all"]);
    run(c);
}

fn task_test() {
    println!("==> cargo test");
    let mut c = cargo();
    c.args(["test", "--workspace"]);
    run(c);
}

fn task_dist() {
    println!("==> cargo build --release");
    let mut c = cargo();
    c.args(["build", "--release", "--workspace", "--exclude", "xtask"]);
    run(c);

    let root = workspace_root();
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).expect("create dist/");

    for bin in ["cr2hdr", "dual-iso-gui"] {
        let src = root.join("target").join("release").join(bin);
        if src.exists() {
            let dst = dist.join(bin);
            std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {bin}: {e}"));
            println!("  dist/{bin}");
        }
    }
    println!("==> dist/ ready");
}

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "check" => task_check(),
        "fmt" => task_fmt(),
        "test" => task_test(),
        "dist" => task_dist(),
        other => {
            eprintln!("Unknown xtask: {other:?}");
            eprintln!("Available tasks: check  fmt  test  dist");
            exit(1);
        }
    }
}
