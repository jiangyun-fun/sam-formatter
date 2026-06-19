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

/// A command-line tool to convert SAM/BAM alignments into a uniform table format.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
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
