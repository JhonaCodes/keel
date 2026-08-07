// SPDX-License-Identifier: Apache-2.0
//! keel-shim — governed command interposition (spec section 5.3 inner ring).
//!
//! Invoked by a generated session shim as:
//!   keel-shim --socket <path> --name <cmd> --real <path> -- [args...]
//!
//! It sends `{name, argv, cwd}` to the session broker and applies the
//! decision: allow → exec the REAL binary (this process BECOMES the command);
//! block → the ContextPacket goes to stderr and the exit code is 2 — the
//! command never exists as a process.
//!
//! FAIL-CLOSED: a dead/unreachable broker is exit 2, never a silent allow. A
//! containment layer that fails open is not a containment layer.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::ExitCode;

#[derive(Serialize)]
struct ShimRequest {
    name: String,
    argv: Vec<String>,
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct ShimResponse {
    allow: bool,
    #[serde(default)]
    packet: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            // Fail-closed contract: any shim-side failure blocks.
            eprintln!("[keel] shim: {message} — command blocked (fail-closed)");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut name = None;
    let mut real = None;
    let mut rest: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next(),
            "--name" => name = args.next(),
            "--real" => real = args.next(),
            "--" => {
                rest = args.collect();
                break;
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    let socket = socket.ok_or("missing --socket")?;
    let name = name.ok_or("missing --name")?;
    let real = real.ok_or("missing --real")?;

    let mut argv = vec![name.clone()];
    argv.extend(rest.iter().cloned());

    let request = ShimRequest {
        name,
        argv,
        cwd: std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string()),
    };

    let mut stream =
        UnixStream::connect(&socket).map_err(|e| format!("broker unreachable at {socket}: {e}"))?;
    let payload = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    stream
        .write_all(payload.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| format!("could not send the request: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("no decision from the broker: {e}"))?;
    let response: ShimResponse =
        serde_json::from_str(&line).map_err(|e| format!("malformed decision: {e}"))?;

    if let Some(packet) = &response.packet {
        eprintln!("{packet}");
    }

    if response.allow {
        // exec: this process becomes the real command — exit code, signals
        // and stdio belong to it, exactly as if the shim never existed.
        let err = std::process::Command::new(&real).args(&rest).exec();
        Err(format!("could not exec `{real}`: {err}"))
    } else {
        Ok(ExitCode::from(2))
    }
}
