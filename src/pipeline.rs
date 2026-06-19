//! The format-agnostic discover-then-stream pipeline.

use std::collections::HashSet;
use std::error::Error;

use noodles_sam as sam;

use crate::cli::OutputFormat;
use crate::record::{DataRecord, from_alignment};
use crate::writer::OutputWriter;

/// Resolved configuration consumed by [`run`].
pub struct Cfg {
    pub output: Option<String>,
    pub format: OutputFormat,
    pub delimiter: String,
    pub no_quotes: bool,
    pub limit: usize,
    pub detect_limit: usize,
    pub keep_tag_prefix: bool,
}

/// Processes an iterator of noodles alignment records into the configured output.
///
/// Works for both SAM (`sam::Record`) and BAM (`bam::Record`) because both implement
/// the shared `sam::alignment::Record` trait. The pipeline runs in two phases:
///
/// 1. **Discover** — buffer up to `detect_limit` records, collecting every optional tag,
///    so the output schema (and header) can be built before anything is written.
/// 2. **Stream** — emit the buffered records, then keep reading the iterator up to
///    `limit` total records, writing each as it arrives.
///
/// Returns the number of records written.
pub fn run<Rec>(
    records: &mut dyn Iterator<Item = std::io::Result<Rec>>,
    header: &sam::Header,
    cfg: &Cfg,
) -> Result<usize, Box<dyn Error>>
where
    Rec: sam::alignment::Record,
{
    // --- Phase 1: discover tags from the initial part of the stream. ---
    // Records beyond `limit` are never emitted, so their tags can't affect the output
    // schema — cap the scan at `min(detect_limit, limit)` to avoid needless work/memory.
    let scan_cap = cfg.detect_limit.min(cfg.limit);
    eprintln!("Scanning up to {scan_cap} records to discover all tags...");

    let mut all_tags: HashSet<String> = HashSet::new();
    let mut buffered: Vec<DataRecord> = Vec::with_capacity(scan_cap.min(1024));

    for _ in 0..scan_cap {
        match records.next() {
            Some(Ok(record)) => {
                let data = from_alignment(&record, header, cfg.keep_tag_prefix)?;
                for tag in data.optional_fields.keys() {
                    all_tags.insert(tag.clone());
                }
                buffered.push(data);
            }
            Some(Err(err)) => return Err(err.into()),
            None => break,
        }
    }

    eprintln!(
        "Found {} unique tags in {} scanned records.",
        all_tags.len(),
        buffered.len()
    );

    let mut sorted_tags: Vec<String> = all_tags.into_iter().collect();
    sorted_tags.sort_unstable();

    let mut writer = OutputWriter::new(
        cfg.output.clone(),
        cfg.format,
        cfg.delimiter.clone(),
        cfg.no_quotes,
        &sorted_tags,
    )?;
    writer.write_header()?;

    // --- Phase 2: write buffered records, then stream the rest. ---
    let mut count = 0usize;

    for record in &buffered {
        if count >= cfg.limit {
            break;
        }
        writer.write_record(record)?;
        count += 1;
    }

    eprintln!("Processing remaining records from stream...");
    while count < cfg.limit {
        match records.next() {
            Some(Ok(record)) => {
                let data = from_alignment(&record, header, cfg.keep_tag_prefix)?;
                writer.write_record(&data)?;
                count += 1;
            }
            Some(Err(err)) => return Err(err.into()),
            None => break,
        }
    }

    writer.flush()?;
    Ok(count)
}
