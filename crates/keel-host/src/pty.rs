// SPDX-License-Identifier: Apache-2.0
//! PTY passthrough — the child CLI runs interactive and unmodified.
//!
//! keel is a transparent parent: bytes flow master↔terminal untouched (no
//! parsing, no filtering — the operator sees everything the model sees).
//! Governance does not live here; it lives in the environment the child was
//! born into (shims, sandbox). The PTY exists so ANY interactive CLI — TUIs
//! included — runs under keel exactly as it would alone.

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// SIGWINCH observed flag (async-signal-safe: the handler only stores).
static WINCH: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_: libc::c_int) {
    WINCH.store(true, Ordering::SeqCst);
}

/// Runs `argv` under a fresh PTY with `env_overrides` applied on top of the
/// parent environment. Blocks until the child exits; returns its exit code.
pub fn run(argv: &[String], env_overrides: &BTreeMap<String, String>, cwd: &Path) -> Result<i32> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(current_size())
        .context("pty: could not allocate a pseudo-terminal")?;

    let mut cmd = CommandBuilder::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.cwd(cwd);
    for (k, v) in env_overrides {
        cmd.env(k, v);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("pty: could not spawn `{}`", argv[0]))?;
    drop(pair.slave);

    // Terminal in raw mode only when we actually own one: piped stdin (tests,
    // scripts) flows through unchanged.
    let raw_guard = RawModeGuard::enable_if_tty();
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            on_winch as extern "C" fn(libc::c_int) as usize as libc::sighandler_t,
        );
    }

    // master → our stdout.
    let mut reader = pair.master.try_clone_reader()?;
    let out_thread = std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let mut buf = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if stdout
                .write_all(&buf[..n])
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    });

    // our stdin → master. The thread parks on read; it ends with the process
    // (it cannot be joined portably once the child is gone — detached).
    let mut writer = pair.master.take_writer()?;
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
            if writer
                .write_all(&buf[..n])
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });

    // Wait loop: child exit + window resizes.
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if WINCH.swap(false, Ordering::SeqCst) {
            let _ = pair.master.resize(current_size());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };

    drop(pair.master);
    let _ = out_thread.join();
    drop(raw_guard);

    Ok(status.exit_code() as i32)
}

/// Current terminal size, or the 80x24 default when stdout is not a tty
/// (CI, pipes).
fn current_size() -> PtySize {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    if ok && ws.ws_row > 0 && ws.ws_col > 0 {
        PtySize {
            rows: ws.ws_row,
            cols: ws.ws_col,
            pixel_width: ws.ws_xpixel,
            pixel_height: ws.ws_ypixel,
        }
    } else {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Puts the controlling terminal in raw mode for the child's lifetime and
/// restores it on drop. No-op when stdin is not a tty.
struct RawModeGuard {
    original: Option<libc::termios>,
}

impl RawModeGuard {
    fn enable_if_tty() -> Self {
        let is_tty = unsafe { libc::isatty(libc::STDIN_FILENO) } == 1;
        if !is_tty {
            return RawModeGuard { original: None };
        }
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut term) != 0 {
                return RawModeGuard { original: None };
            }
            let original = term;
            libc::cfmakeraw(&mut term);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &term) != 0 {
                return RawModeGuard { original: None };
            }
            RawModeGuard {
                original: Some(original),
            }
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if let Some(original) = self.original {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &original);
            }
        }
    }
}
