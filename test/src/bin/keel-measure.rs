// SPDX-License-Identifier: Apache-2.0
//! `keel-measure` — the Phase 0c enforcement-measurement harness entry point
//! (spec section 15.1). A thin wrapper: it parses arguments, runs the
//! experiment (all logic lives in `keel_tests::measure`), writes `report.json`
//! and `report.md`, and prints the primary delta.
//!
//! Usage:
//!   keel-measure --dataset <dir> [--out <dir>] [--min-delta-rate <f>] [--keel-bin <path>]

use anyhow::{bail, Context, Result};
use keel_tests::measure::{run, Dataset, Options};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut dataset: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut opts = Options::default();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dataset" => dataset = Some(next(&mut args, "--dataset")?.into()),
            "--out" => out = Some(next(&mut args, "--out")?.into()),
            "--keel-bin" => opts.keel_bin = next(&mut args, "--keel-bin")?.into(),
            "--min-delta-rate" => {
                opts.min_delta_rate = next(&mut args, "--min-delta-rate")?
                    .parse()
                    .context("--min-delta-rate must be a number")?
            }
            "-h" | "--help" => {
                println!(
                    "keel-measure --dataset <dir> [--out <dir>] [--min-delta-rate <f>] [--keel-bin <path>]"
                );
                return Ok(());
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let dataset_dir = dataset.context("--dataset <dir> is required")?;
    let out_dir = out.unwrap_or_else(|| {
        PathBuf::from("target/phase0c").join(
            dataset_dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "run".into()),
        )
    });

    let ds = Dataset::load(&dataset_dir)?;
    let report = run(&ds, &out_dir, &opts)?;
    report.write_to(&out_dir)?;

    let p = &report.primary;
    println!(
        "dataset {} · violations {} · delta {} ({:.1}%) · verdict {}",
        report.dataset_id,
        p.violations,
        p.delta,
        p.delta_rate * 100.0,
        report.verdict
    );
    println!("report → {}", out_dir.join("report.md").display());
    Ok(())
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next().with_context(|| format!("{flag} needs a value"))
}
