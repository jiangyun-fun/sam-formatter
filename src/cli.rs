//! Command-line interface: argument parsing and the shared schema constants.

use arrow::datatypes::DataType;
use clap::{Parser, ValueEnum};

/// The 11 mandatory SAM columns in canonical order, paired with the Arrow type
/// used for them when writing Parquet output.
pub const MANDATORY_HEADERS: [(&str, DataType); 11] = [
    ("QNAME", DataType::Utf8),
    ("FLAG", DataType::Int32),
    ("RNAME", DataType::Utf8),
    ("POS", DataType::Int32),
    ("MAPQ", DataType::Int32),
    ("CIGAR", DataType::Utf8),
    ("RNEXT", DataType::Utf8),
    ("PNEXT", DataType::Int32),
    ("TLEN", DataType::Int32),
    ("SEQ", DataType::Utf8),
    ("QUAL", DataType::Utf8),
];

/// Output format options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Comma-separated values (default).
    Csv,
    /// Tab-separated values.
    Tsv,
    /// Pipe-separated values.
    Psv,
    /// Apache Parquet format.
    Parquet,
    /// Custom delimiter (specify with `--delimiter`).
    Custom,
}

/// Input (SAM/BAM) format selection. `Auto` sniffs the magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InputFormat {
    /// Detect SAM vs BAM from the input's magic bytes (default).
    Auto,
    /// Force SAM (text) parsing.
    Sam,
    /// Force BAM (binary/BGZF) parsing.
    Bam,
}

/// How to downsample the records, derived from the `--downsample` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sampler {
    /// Keep each record independently with the given probability (a fraction in (0, 1)).
    Bernoulli(f64),
    /// Keep exactly this many uniformly random records (reservoir sampling).
    Reservoir(usize),
}

/// A command-line tool to convert SAM/BAM alignments into a uniform table format.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, allow_negative_numbers = true)]
pub struct Args {
    /// Input file (`.sam`, `.bam`, or `-` for standard input). Format is auto-detected
    /// unless `--input-format` is set.
    #[arg(default_value = "-")]
    pub input: String,

    /// Output file. If not provided, writes to standard output.
    #[arg(short = 'o', long)]
    pub output: Option<String>,

    /// Maximum number of records to process in total.
    #[arg(short = 'n', long, default_value_t = 1_000_000)]
    pub limit: usize,

    /// Number of records to scan to discover all optional tags before writing.
    /// A larger value is more likely to find every tag in a diverse file, at the cost
    /// of buffering more records in memory up front.
    #[arg(long, default_value_t = 100_000)]
    pub detect_limit: usize,

    /// Output format. If not provided, it is auto-detected from the output file
    /// extension; defaults to CSV for standard output.
    #[arg(short = 'f', long, value_enum)]
    pub format: Option<OutputFormat>,

    /// Input format override (`auto`, `sam`, or `bam`).
    #[arg(long, value_enum, default_value_t = InputFormat::Auto)]
    pub input_format: InputFormat,

    /// Custom delimiter string (only used with `--format custom`).
    #[arg(short = 'd', long, default_value = ",")]
    pub delimiter: String,

    /// Keep the full tag with type and value (e.g. `cs:Z::70`).
    /// By default the prefix is removed and only the value is kept (e.g. `:70` or `140`).
    #[arg(long)]
    pub keep_tag_prefix: bool,

    /// Don't quote fields in the output (only applies to CSV format).
    #[arg(long)]
    pub no_quotes: bool,

    /// Downsample the output. A fraction in (0, 1) (e.g. `0.1` ≈ 10%) keeps each record
    /// independently; an integer ≥ 1 (e.g. `1000`) keeps exactly that many uniformly
    /// random records via reservoir sampling.
    #[arg(long)]
    pub downsample: Option<f64>,

    /// Seed for `--downsample` reproducibility. Omit for a different random subset each
    /// run; provide a `u64` for a reproducible subset.
    #[arg(long)]
    pub seed: Option<u64>,
}

impl Args {
    /// Resolves `--downsample` into a [`Sampler`], validating the value:
    /// - `(0, 1)` → `Bernoulli(fraction)`
    /// - integer `≥ 1` → `Reservoir(count)`
    /// - `≤ 0`, non-finite, or a non-integer `≥ 1` → error.
    pub fn sampler(&self) -> Result<Option<Sampler>, &'static str> {
        let Some(value) = self.downsample else {
            return Ok(None);
        };
        if !value.is_finite() || value <= 0.0 {
            return Err("--downsample must be a positive number");
        }
        if value < 1.0 {
            Ok(Some(Sampler::Bernoulli(value)))
        } else if value.fract() == 0.0 {
            Ok(Some(Sampler::Reservoir(value as usize)))
        } else {
            Err("--downsample >= 1 must be an integer count; use a fraction < 1 for a percent")
        }
    }
}

/// Resolves the effective output format, honoring an explicit `--format` or, failing
/// that, the output file extension (defaulting to CSV for stdout).
pub fn resolve_output_format(args: &Args) -> OutputFormat {
    if let Some(format) = args.format {
        return format;
    }

    let extension = args
        .output
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::extension)
        .and_then(std::ffi::OsStr::to_str);

    match extension {
        Some("tsv") => OutputFormat::Tsv,
        Some("psv") => OutputFormat::Psv,
        Some("parquet") | Some("parq") => OutputFormat::Parquet,
        // CSV (and any other/unknown extension) defaults to CSV.
        _ => OutputFormat::Csv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(downsample: Option<f64>) -> Args {
        Args {
            input: "-".into(),
            output: None,
            limit: 1_000_000,
            detect_limit: 100_000,
            format: None,
            input_format: InputFormat::Auto,
            delimiter: ",".into(),
            keep_tag_prefix: false,
            no_quotes: false,
            downsample,
            seed: None,
        }
    }

    #[test]
    fn sampler_classifies_fraction_and_count() {
        assert_eq!(
            args(Some(0.1)).sampler().unwrap(),
            Some(Sampler::Bernoulli(0.1))
        );
        assert_eq!(
            args(Some(0.5)).sampler().unwrap(),
            Some(Sampler::Bernoulli(0.5))
        );
        assert_eq!(
            args(Some(100.0)).sampler().unwrap(),
            Some(Sampler::Reservoir(100))
        );
        // `1` is ≥ 1 and integer-valued → one record (not 100%).
        assert_eq!(
            args(Some(1.0)).sampler().unwrap(),
            Some(Sampler::Reservoir(1))
        );
        assert_eq!(args(None).sampler().unwrap(), None);
    }

    #[test]
    fn sampler_rejects_invalid_values() {
        assert!(args(Some(0.0)).sampler().is_err());
        assert!(args(Some(-0.5)).sampler().is_err());
        // Non-integer ≥ 1 is ambiguous → error.
        assert!(args(Some(2.5)).sampler().is_err());
    }
}
