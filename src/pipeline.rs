//! The format-agnostic discover-then-stream pipeline, with optional downsampling.

use std::collections::HashSet;
use std::error::Error;

use noodles_sam as sam;
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

use crate::cli::{OutputFormat, Sampler};
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
    pub sampler: Option<Sampler>,
    pub seed: Option<u64>,
}

/// Builds the RNG: seeded for reproducibility, or system-random when `--seed` is omitted.
fn make_rng(seed: Option<u64>) -> SmallRng {
    match seed {
        Some(s) => SmallRng::seed_from_u64(s),
        None => {
            // rand 0.10 removed `from_os_rng`; seed from the system RNG instead.
            let mut sys = rand::rngs::SysRng;
            SmallRng::try_from_rng(&mut sys).expect("system RNG unavailable")
        }
    }
}

/// Processes an iterator of noodles alignment records into the configured output.
///
/// Dispatches on the optional sampler:
/// - none / `Bernoulli` — the streaming discover-then-stream path (Bernoulli gates each
///   emitted record independently with probability `f`).
/// - `Reservoir(n)` — a single full pass keeping exactly `n` uniformly random records
///   (Algorithm R); reads the whole stream before writing, holds `n` records in memory,
///   and (since it reads everything) discovers the complete tag schema.
///
/// Works for both SAM (`sam::Record`) and BAM (`bam::Record`) because both implement the
/// shared `sam::alignment::Record` trait. Returns the number of records written.
pub fn run<Rec>(
    records: &mut dyn Iterator<Item = std::io::Result<Rec>>,
    header: &sam::Header,
    cfg: &Cfg,
) -> Result<usize, Box<dyn Error>>
where
    Rec: sam::alignment::Record,
{
    match cfg.sampler {
        Some(Sampler::Reservoir(n)) => run_reservoir(records, header, cfg, n),
        Some(Sampler::Bernoulli(f)) => {
            run_stream(records, header, cfg, Some((f, make_rng(cfg.seed))))
        }
        None => run_stream(records, header, cfg, None),
    }
}

/// Streaming path: discover tags from a prefix, then emit records (optionally Bernoulli-
/// gated). `--limit` caps the number of records *read*.
fn run_stream<Rec>(
    records: &mut dyn Iterator<Item = std::io::Result<Rec>>,
    header: &sam::Header,
    cfg: &Cfg,
    mut bernoulli: Option<(f64, SmallRng)>,
) -> Result<usize, Box<dyn Error>>
where
    Rec: sam::alignment::Record,
{
    // --- Phase 1: discover tags from the initial part of the stream. ---
    // Records beyond `limit` are never read, so their tags can't affect the output schema
    // — cap the scan at `min(detect_limit, limit)` to avoid needless work/memory.
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

    // --- Phase 2: write buffered records, then stream the rest (gated if sampling). ---
    let mut written = 0usize;
    let mut read = buffered.len();

    for record in &buffered {
        if let Some((fraction, rng)) = bernoulli.as_mut()
            && !rng.random_bool(*fraction)
        {
            continue;
        }
        writer.write_record(record)?;
        written += 1;
    }

    eprintln!("Processing remaining records from stream...");
    while read < cfg.limit {
        match records.next() {
            Some(Ok(record)) => {
                read += 1;
                let data = from_alignment(&record, header, cfg.keep_tag_prefix)?;
                if let Some((fraction, rng)) = bernoulli.as_mut()
                    && !rng.random_bool(*fraction)
                {
                    continue;
                }
                writer.write_record(&data)?;
                written += 1;
            }
            Some(Err(err)) => return Err(err.into()),
            None => break,
        }
    }

    writer.flush()?;
    Ok(written)
}

/// Reservoir path: read the whole stream (up to `limit`), keep exactly `n` uniformly
/// random records, then write them in original stream order.
fn run_reservoir<Rec>(
    records: &mut dyn Iterator<Item = std::io::Result<Rec>>,
    header: &sam::Header,
    cfg: &Cfg,
    n: usize,
) -> Result<usize, Box<dyn Error>>
where
    Rec: sam::alignment::Record,
{
    eprintln!("Reservoir-sampling {n} records (reading entire stream before writing)...");

    let mut rng = make_rng(cfg.seed);
    let mut all_tags: HashSet<String> = HashSet::new();
    // (stream position, record) so the sample can be emitted in original order.
    let mut reservoir: Vec<(usize, DataRecord)> = Vec::with_capacity(n);
    let mut seen: usize = 0;

    loop {
        if cfg.limit != 0 && seen >= cfg.limit {
            break; // `--limit` caps the input stream.
        }
        let record = match records.next() {
            Some(Ok(record)) => record,
            Some(Err(err)) => return Err(err.into()),
            None => break,
        };
        seen += 1;

        let data = from_alignment(&record, header, cfg.keep_tag_prefix)?;
        for tag in data.optional_fields.keys() {
            all_tags.insert(tag.clone());
        }

        if reservoir.len() < n {
            reservoir.push((seen, data));
        } else {
            // Algorithm R: replace a random slot with probability n/seen.
            let j = rng.random_range(0..seen);
            if j < n {
                reservoir[j] = (seen, data);
            }
        }
    }

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

    // Emit the sample in original stream order.
    reservoir.sort_unstable_by_key(|(position, _)| *position);
    for (_, data) in &reservoir {
        writer.write_record(data)?;
    }

    let written = reservoir.len();
    writer.flush()?;
    Ok(written)
}
