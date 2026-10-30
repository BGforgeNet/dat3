//! Output to a reader that has gone. Rust ignores SIGPIPE, so a std print to a closed pipe panics; dat3 drops the
//! output instead and finishes the work. Each pipe's reader is closed before the child starts, so every write fails,
//! where a shell `| head -1` races the writer.
#![expect(
    clippy::unwrap_used,
    reason = "fixture helpers outside #[test] fns: a panic is the failure report"
)]

use std::process::{Command, ExitStatus, Stdio};

use dat3_core::test_support::ScratchPath;

fn dat3() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dat3"))
}

/// Runs `cmd` with stdout and stderr both on pipes whose readers are already closed.
fn run_with_closed_pipes(cmd: &mut Command) -> ExitStatus {
    let (out_reader, out_writer) = std::io::pipe().unwrap();
    let (err_reader, err_writer) = std::io::pipe().unwrap();
    drop((out_reader, err_reader));
    cmd.stdout(out_writer).stderr(err_writer).status().unwrap()
}

/// A scratch directory holding `src/data/a.txt`, and an archive of it at `test.dat`.
fn fixture() -> ScratchPath {
    let dir = ScratchPath::dir("closed_pipe");
    std::fs::create_dir_all(dir.join("src/data")).unwrap();
    std::fs::write(dir.join("src/data/a.txt"), b"a").unwrap();
    let status = dat3()
        .arg("a")
        .arg(dir.join("test.dat"))
        .arg("-C")
        .arg(dir.join("src"))
        .arg("data")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

#[test]
fn a_missing_name_fails_by_its_exit_status_not_a_signal() {
    let dir = fixture();
    let status = run_with_closed_pipes(
        dat3()
            .arg("l")
            .arg(dir.join("test.dat"))
            .arg("no/such/file"),
    );
    assert_eq!(status.code(), Some(1), "{status}");
}

/// `a` reports to both streams: each added file on stdout, and on Unix the skipped symlink on stderr.
#[test]
fn add_finishes_its_archive() {
    let dir = fixture();
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.txt", dir.join("src/data/link.txt")).unwrap();
    let archive = dir.join("new.dat");
    let status = run_with_closed_pipes(
        dat3()
            .arg("a")
            .arg(&archive)
            .arg("-C")
            .arg(dir.join("src"))
            .arg("data"),
    );
    assert_eq!(status.code(), Some(0), "{status}");
    let listing = dat3()
        .arg("l")
        .arg("--json")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(listing.status.success());
    assert!(
        String::from_utf8(listing.stdout)
            .unwrap()
            .contains("data/a.txt")
    );
}

/// A write error other than a closed pipe means the output is incomplete, which the exit status reports.
#[cfg(target_os = "linux")]
#[test]
fn a_full_disk_fails_the_command() {
    let dir = fixture();
    let output = dat3()
        .arg("l")
        .arg(dir.join("test.dat"))
        .stdout(std::fs::File::create("/dev/full").unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("dat3: writing output:"), "{stderr}");
}
