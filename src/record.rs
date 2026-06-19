//! Internal record model and the bridge from noodles alignment records.

use std::collections::HashMap;
use std::io;

use noodles_sam as sam;

/// A flattened alignment record: the 11 mandatory SAM columns plus any optional
/// tags discovered on the record, keyed by tag name (e.g. `NM`, `RG`).
#[derive(Debug, Clone)]
pub struct DataRecord {
    pub mandatory_fields: Vec<String>,
    pub optional_fields: HashMap<String, String>,
}

impl DataRecord {
    /// Parses a single SAM record line (tab-separated) into a `DataRecord`.
    ///
    /// `keep_tag_prefix` controls how optional tags are stored: when `false` only the
    /// value is kept (`NM:i:1` → `1`); when `true` the full `TAG:TYPE:VALUE` is kept.
    pub fn from_line(line: &str, keep_tag_prefix: bool) -> Self {
        let fields: Vec<&str> = line.split('\t').collect();

        // The first 11 fields are the mandatory SAM columns.
        let mandatory_fields: Vec<String> = (0..11)
            .map(|i| fields.get(i).copied().unwrap_or_default().to_string())
            .collect();

        // Optional tags appear from column 12 on, formatted as `TAG:TYPE:VALUE`.
        let mut optional_fields = HashMap::new();
        for field in fields.iter().skip(11) {
            let parts: Vec<&str> = field.splitn(3, ':').collect();
            if parts.len() >= 2 && !parts[0].is_empty() {
                let tag = parts[0].to_string();
                let value = if keep_tag_prefix || parts.len() < 3 {
                    (*field).to_string()
                } else {
                    parts[2].to_string()
                };
                optional_fields.insert(tag, value);
            }
        }

        DataRecord {
            mandatory_fields,
            optional_fields,
        }
    }
}

/// Converts any noodles alignment record (`sam::Record` from text, or `bam::Record`
/// from BAM — both implement the shared `sam::alignment::Record` trait) into a
/// `DataRecord`.
///
/// Rather than hand-rolling SAM field stringification (CIGAR op packing, SEQ/QUAL
/// encoding, typed tag values), this serializes the record to a canonical SAM line
/// via noodles' own writer and feeds it through [`DataRecord::from_line`]. That keeps
/// formatting correct for both SAM and BAM and preserves the tool's existing tag
/// parsing behavior exactly.
pub fn from_alignment<Rec>(
    record: &Rec,
    header: &sam::Header,
    keep_tag_prefix: bool,
) -> io::Result<DataRecord>
where
    Rec: sam::alignment::Record,
{
    use sam::alignment::io::Write as _;

    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = sam::io::Writer::new(&mut buffer);
        writer.write_alignment_record(header, record)?;
    }

    let line = std::str::from_utf8(&buffer)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(DataRecord::from_line(
        line.trim_end_matches(['\r', '\n']),
        keep_tag_prefix,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_line_parses_mandatory_columns_and_tags() {
        let line =
            "r1\t0\tchr1\t100\t60\t10M\tchr1\t200\t100\tACGTACGTAC\tIIIIIIIIII\tNM:i:1\tRG:Z:grp1";

        let record = DataRecord::from_line(line, false);

        assert_eq!(record.mandatory_fields.len(), 11);
        assert_eq!(record.mandatory_fields[0], "r1"); // QNAME
        assert_eq!(record.mandatory_fields[1], "0"); // FLAG
        assert_eq!(record.mandatory_fields[2], "chr1"); // RNAME
        assert_eq!(record.mandatory_fields[3], "100"); // POS
        assert_eq!(record.mandatory_fields[5], "10M"); // CIGAR
        assert_eq!(record.mandatory_fields[9], "ACGTACGTAC"); // SEQ
        assert_eq!(record.mandatory_fields[10], "IIIIIIIIII"); // QUAL
        assert_eq!(
            record.optional_fields.get("NM").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            record.optional_fields.get("RG").map(String::as_str),
            Some("grp1")
        );
    }

    #[test]
    fn from_line_strips_tag_prefix_by_default_and_keeps_it_when_asked() {
        let line = "r1\t0\tchr1\t100\t60\t10M\tchr1\t200\t100\tACGT\tIIII\tNM:i:1";

        let stripped = DataRecord::from_line(line, false);
        assert_eq!(
            stripped.optional_fields.get("NM").map(String::as_str),
            Some("1")
        );

        let kept = DataRecord::from_line(line, true);
        assert_eq!(
            kept.optional_fields.get("NM").map(String::as_str),
            Some("NM:i:1")
        );
    }

    #[test]
    fn from_line_preserves_colons_in_tag_values() {
        // A `Z` value may itself contain colons; only the first two colons split the
        // TAG:TYPE prefix.
        let line = "r1\t0\t*\t0\t0\t*\t*\t0\t0\t*\t*\tXX:Z:a:b:c";

        let record = DataRecord::from_line(line, false);
        assert_eq!(
            record.optional_fields.get("XX").map(String::as_str),
            Some("a:b:c")
        );
    }

    #[test]
    fn from_alignment_serializes_a_default_record() {
        // noodles' default record serializes to the SAM line:
        //   *  4  *  0  255  *  *  0  0  *  *
        let header = sam::Header::default();
        let record = sam::Record::default();

        let data = from_alignment(&record, &header, false).unwrap();

        assert_eq!(
            data.mandatory_fields,
            vec!["*", "4", "*", "0", "255", "*", "*", "0", "0", "*", "*"]
        );
        assert!(data.optional_fields.is_empty());
    }
}
