# sam-formatter

Convert **SAM/BAM** alignment files into a uniform tabular format — CSV, TSV, PSV,
Parquet, or any custom delimiter — with optional SAM tags promoted to their own sorted
columns.

## Features

- Reads **SAM** (text) and **BAM** (binary/BGZF) from a file or standard input.
- Auto-detects format from magic bytes (`--input-format` to override).
- Outputs CSV (default), TSV, PSV, Parquet, or a custom delimiter.
- Optional tags (`NM`, `RG`, `MD`, …) become their own columns, sorted alphabetically.
- Two-phase discover-then-stream; tag discovery is capped at `min(--detect-limit, --limit)`
  so memory stays bounded.
- Parquet output is typed: `FLAG/POS/MAPQ/PNEXT/TLEN` as `int32`, text and tags as `utf8`,
  missing tags as `NULL`.
- **Downsample**: `--downsample 0.1` keeps ~10% of records (Bernoulli); `--downsample 1000`
  keeps exactly 1000 uniformly random records (reservoir sampling). `--seed` makes a sample
  reproducible.

## Install

```bash
cargo install --path .
# or build locally:
cargo build --release   # binary at target/release/sam-formatter
```

Requires Rust ≥ 1.89 (noodles).

## Usage

```bash
sam-formatter in.sam                        # SAM file -> CSV on stdout
sam-formatter in.bam                        # BAM file -> CSV (auto-detected)
cat in.sam | sam-formatter -                # SAM from stdin
samtools view -b in.bam | sam-formatter -   # BAM from stdin
```

Output format:

```bash
sam-formatter in.bam -o out.tsv             # extension -> TSV
sam-formatter in.bam -o out.parquet         # Parquet
sam-formatter in.bam -f psv                 # pipe-separated
sam-formatter in.bam -f custom -d ';'       # any custom delimiter
```

Tags:

```bash
sam-formatter in.sam                        # NM:i:1 -> "1"        (value only)
sam-formatter in.sam --keep-tag-prefix      # NM:i:1 -> "NM:i:1"   (full tag)
```

Downsampling:

```bash
sam-formatter in.bam --downsample 0.1            # keep ~10% of records (Bernoulli)
sam-formatter in.bam --downsample 1000           # keep exactly 1000 random records (reservoir)
sam-formatter in.bam --downsample 1000 --seed 7  # reproducible sample
```

A fraction in `(0, 1)` keeps each record independently (≈ that share of the input); an
integer `≥ 1` keeps exactly that many records via reservoir sampling (held in memory; the
whole stream is read before any output). `--limit` caps the input before sampling.

## Options

| Flag                 | Default   | Purpose                                              |
| -------------------- | --------- | ---------------------------------------------------- |
| `[INPUT]`            | `-`       | SAM/BAM file, or `-` for standard input              |
| `-o, --output`       | stdout    | output file (extension picks format)                 |
| `-f, --format`       | auto      | `csv\|tsv\|psv\|parquet\|custom`                      |
| `--input-format`     | `auto`    | `auto\|sam\|bam`                                     |
| `-d, --delimiter`    | `,`       | delimiter for `custom`                               |
| `-n, --limit`        | `1000000` | max records read from input (capped before sampling) |
| `--detect-limit`     | `100000`  | records scanned to discover tags                     |
| `--keep-tag-prefix`  | off       | keep `TAG:TYPE:VALUE` instead of value only          |
| `--no-quotes`        | off       | disable CSV quoting                                  |
| `--downsample`       | off       | fraction `(0,1)` or integer `≥1`; downsample output  |
| `--seed`             | off       | `u64` seed for a reproducible `--downsample`         |

## Notes

- When piping **SAM text**, include the header (`samtools view -h`): records whose
  `RNAME` is not declared in an `@SQ` line are rejected. **BAM** carries its header
  internally, so no special handling is needed.
- A gzip-compressed SAM is **not** read directly (auto-detect misreads it as BAM, and the
  tool does not decompress). Decompress first: `zcat in.sam.gz | sam-formatter`.
- A mate reference equal to `RNAME` is written as `=` (matches `samtools view`).

## How it works

Parsing uses [noodles](https://github.com/zaeleus/noodles) for both SAM and BAM via the
shared `sam::alignment::Record` trait. Each record is serialized to a canonical SAM line
with noodles' own writer and then split into columns — so CIGAR, SEQ/QUAL, and typed tag
formatting are always spec-correct, identically for SAM and BAM.

## License

None yet — all rights reserved. Add a `LICENSE` before redistributing.
