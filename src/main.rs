//! `sam-formatter`: convert SAM/BAM alignments into a uniform table format.
//!
//! Input may be a SAM (text) or BAM (binary/BGZF) file, or `-` for standard input. The
//! format is auto-detected from magic bytes (override with `--input-format`), and the
//! output can be CSV, TSV, PSV, Parquet, or a custom-delimited text format.

mod cli;
mod pipeline;
mod reader;
mod record;
mod writer;

use std::error::Error;
use std::io;

use clap::Parser;

use noodles_bam as bam;
use noodles_sam as sam;

use cli::{Args, InputFormat, OutputFormat, resolve_output_format};
use pipeline::{Cfg, run};

/// Application entry point.
fn main() {
    if let Err(err) = run_app() {
        // A closed downstream pipe (e.g. `| head`) is not an error.
        if let Some(io_err) = err.downcast_ref::<io::Error>()
            && io_err.kind() == io::ErrorKind::BrokenPipe
        {
            std::process::exit(0);
        }
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

/// Main application logic.
fn run_app() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let format = resolve_output_format(&args);

    if format == OutputFormat::Parquet && args.output.is_none() {
        return Err(
            "Parquet format cannot be written to standard output. Please specify an output file with -o."
                .into(),
        );
    }
    if matches!(format, OutputFormat::Custom) && args.delimiter.is_empty() {
        return Err("Custom delimiter cannot be empty.".into());
    }
    if args.detect_limit == 0 {
        return Err("--detect-limit must be greater than 0.".into());
    }

    let sampler = args.sampler()?;

    let mut input = reader::open_input(&args.input)?;
    let input_format = reader::detect_format(&mut input, args.input_format)?;
    eprintln!("Detected input format: {input_format:?}");

    let cfg = Cfg {
        output: args.output.clone(),
        format,
        delimiter: args.delimiter.clone(),
        no_quotes: args.no_quotes,
        limit: args.limit,
        detect_limit: args.detect_limit,
        keep_tag_prefix: args.keep_tag_prefix,
        sampler,
        seed: args.seed,
    };

    let count = match input_format {
        InputFormat::Sam => {
            let mut reader = sam::io::Reader::new(input);
            let header = reader.read_header()?;
            let mut records = reader.records();
            run(&mut records, &header, &cfg)?
        }
        InputFormat::Bam => {
            let mut reader = bam::io::Reader::new(input);
            let header = reader.read_header()?;
            let mut records = reader.records();
            run(&mut records, &header, &cfg)?
        }
        // `detect_format` always resolves `Auto` to `Sam` or `Bam`.
        InputFormat::Auto => unreachable!("input format should be resolved by detect_format"),
    };

    eprintln!("Successfully processed a total of {count} records.");
    Ok(())
}
