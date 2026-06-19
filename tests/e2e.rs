//! End-to-end test: a SAM file and its BAM equivalent must produce identical tabular
//! output. The BAM is produced with noodles (no samtools dependency).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use noodles_bam as bam;
use noodles_sam as sam;
use sam::alignment::io::{Read as SamRead, Write as SamWrite};

// Header + two records with NM and RG tags (RG present on r1 only, to exercise a
// sparse optional column). RNEXT uses an explicit name rather than `=` so the SAM and
// BAM serializations match exactly (BAM resolves `=` to the reference name).
const SAM_FIXTURE: &str = "@HD\tVN:1.6\tSO:unsorted\n\
@SQ\tSN:chr1\tLN:1000\n\
r1\t0\tchr1\t1\t60\t10M\tchr1\t10\t0\tACGTACGTAC\tIIIIIIIIII\tNM:i:1\tRG:Z:grp1\n\
r2\t16\tchr1\t5\t30\t5M\t*\t0\t0\tTTTTT\tABCDE\tNM:i:0\n";

fn tmp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("sam-formatter-test-{}-{name}", std::process::id()));
    path
}

/// Locates the built binary. Prefers cargo's `CARGO_BIN_EXE_*` (set for the test
/// process), falling back to `target/debug/sam-formatter` relative to the manifest dir
/// (works under `cargo test`, which builds the bin before running tests).
fn binary_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_sam_formatter") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("sam-formatter");
    path
}

/// Converts the SAM fixture into a BAM file using noodles (samtools-free).
fn write_bam_from_sam(sam_path: &PathBuf, bam_path: &PathBuf) {
    let sam_reader = sam::io::Reader::new(std::io::BufReader::new(
        fs::File::open(sam_path).expect("open SAM fixture"),
    ));
    let mut sam_reader = sam_reader;
    let header = sam_reader.read_alignment_header().expect("read SAM header");

    let bam_file = fs::File::create(bam_path).expect("create BAM file");
    let mut bam_writer = bam::io::Writer::new(bam_file);
    bam_writer
        .write_alignment_header(&header)
        .expect("write BAM header");

    for result in sam_reader.alignment_records(&header) {
        let record = result.expect("read SAM record");
        bam_writer
            .write_alignment_record(&header, &record)
            .expect("write BAM record");
    }
    bam_writer.finish(&header).expect("finish BAM");
}

#[test]
fn sam_and_bam_produce_equivalent_csv() {
    let sam_path = tmp_path("in.sam");
    let bam_path = tmp_path("in.bam");
    let csv_from_sam = tmp_path("from_sam.csv");
    let csv_from_bam = tmp_path("from_bam.csv");

    fs::write(&sam_path, SAM_FIXTURE).expect("write SAM fixture");
    write_bam_from_sam(&sam_path, &bam_path);

    let bin = binary_path();

    let sam_run = Command::new(&bin)
        .arg(&sam_path)
        .arg("-o")
        .arg(&csv_from_sam)
        .output()
        .expect("run binary on SAM");
    assert!(
        sam_run.status.success(),
        "SAM run failed: {}",
        String::from_utf8_lossy(&sam_run.stderr)
    );

    let bam_run = Command::new(&bin)
        .arg(&bam_path)
        .arg("-o")
        .arg(&csv_from_bam)
        .output()
        .expect("run binary on BAM");
    assert!(
        bam_run.status.success(),
        "BAM run failed: {}",
        String::from_utf8_lossy(&bam_run.stderr)
    );

    let out_sam = fs::read_to_string(&csv_from_sam).expect("read SAM csv");
    let out_bam = fs::read_to_string(&csv_from_bam).expect("read BAM csv");

    // Header must include the mandatory columns plus the sorted tags NM and RG.
    let header = out_sam.lines().next().expect("header row");
    assert!(header.contains("QNAME"));
    assert!(header.contains("NM"));
    assert!(header.contains("RG"));
    // Tags are sorted, so RG follows NM.
    assert!(
        header
            .find("NM")
            .zip(header.find("RG"))
            .is_some_and(|(nm, rg)| nm < rg),
        "expected NM before RG in header: {header}"
    );

    // Header + 2 records = 3 rows, identical between the two paths.
    assert_eq!(out_sam.lines().count(), 3);
    assert_eq!(out_sam, out_bam, "SAM and BAM outputs must match");

    for path in [&sam_path, &bam_path, &csv_from_sam, &csv_from_bam] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn limit_smaller_than_detect_limit_omits_tags_from_non_emitted_records() {
    // r1 carries NM only; r2 carries NM plus a unique ZZ tag. With --limit 1, only r1 is
    // emitted, so the output schema must include NM but NOT ZZ (which lives only on the
    // never-emitted r2). This also guards the memory fix that caps the discovery scan at
    // min(detect_limit, limit).
    let sam = "@HD\tVN:1.6\tSO:unsorted\n\
@SQ\tSN:chr1\tLN:1000\n\
r1\t0\tchr1\t1\t60\t10M\t*\t0\t0\tACGTACGTAC\tIIIIIIIIII\tNM:i:1\n\
r2\t0\tchr1\t2\t60\t10M\t*\t0\t0\tACGTACGTAC\tIIIIIIIIII\tNM:i:0\tZZ:Z:only-on-r2\n";

    let sam_path = tmp_path("limit.sam");
    let csv_path = tmp_path("limit.csv");
    fs::write(&sam_path, sam).expect("write SAM fixture");

    let run = Command::new(binary_path())
        .arg(&sam_path)
        .arg("--limit")
        .arg("1")
        .arg("--detect-limit")
        .arg("1000")
        .arg("-o")
        .arg(&csv_path)
        .output()
        .expect("run binary");
    assert!(
        run.status.success(),
        "limit run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let out = fs::read_to_string(&csv_path).expect("read csv");
    let header = out.lines().next().expect("header row");
    assert!(header.contains("NM"), "NM (on emitted r1) must be present");
    assert!(
        !header.contains("ZZ"),
        "ZZ (only on non-emitted r2) must be absent: {header}"
    );
    // Header + exactly one emitted record.
    assert_eq!(out.lines().count(), 2);

    let _ = fs::remove_file(sam_path);
    let _ = fs::remove_file(csv_path);
}
