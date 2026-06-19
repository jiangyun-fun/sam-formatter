//! Output writers for CSV/TSV/PSV/Parquet (and arbitrary delimited text).

use std::error::Error;
use std::fs::File;
use std::io::{self, Write};
use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use csv::WriterBuilder;
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::file::properties::WriterProperties;

use crate::cli::MANDATORY_HEADERS;
use crate::record::DataRecord;

// Internal enum to cleanly manage different writer implementations.
enum WriterImpl {
    Csv(csv::Writer<Box<dyn Write>>),
    Delimited(Box<dyn Write>, String),
    Parquet {
        writer: ArrowWriter<File>,
        schema: SchemaRef,
        buffer: Vec<DataRecord>,
        batch_size: usize,
    },
}

/// Handles writing output in various formats (CSV, TSV, PSV, Parquet, custom).
pub struct OutputWriter {
    writer_impl: WriterImpl,
    sorted_tags: Vec<String>,
}

impl OutputWriter {
    /// Creates a new `OutputWriter`.
    pub fn new(
        output_path: Option<String>,
        format: crate::cli::OutputFormat,
        custom_delimiter: String,
        no_quotes: bool,
        sorted_tags: &[String],
    ) -> Result<Self, Box<dyn Error>> {
        let writer_impl = match format {
            crate::cli::OutputFormat::Csv => {
                let writer: Box<dyn Write> = match &output_path {
                    Some(path) => Box::new(File::create(path)?),
                    None => Box::new(io::BufWriter::new(io::stdout().lock())),
                };
                let mut builder = WriterBuilder::new();
                builder.delimiter(b',');
                if no_quotes {
                    builder.quote_style(csv::QuoteStyle::Never);
                }
                WriterImpl::Csv(builder.from_writer(writer))
            }
            crate::cli::OutputFormat::Tsv
            | crate::cli::OutputFormat::Psv
            | crate::cli::OutputFormat::Custom => {
                let writer: Box<dyn Write> = match &output_path {
                    Some(path) => Box::new(File::create(path)?),
                    None => Box::new(io::BufWriter::new(io::stdout().lock())),
                };
                let delimiter = match format {
                    crate::cli::OutputFormat::Tsv => "\t".to_string(),
                    crate::cli::OutputFormat::Psv => "|".to_string(),
                    _ => custom_delimiter,
                };
                WriterImpl::Delimited(writer, delimiter)
            }
            crate::cli::OutputFormat::Parquet => {
                let path = output_path.ok_or("Parquet output requires a file path.")?;
                let file = File::create(path)?;

                // Build fields from the mandatory headers (with their DataTypes).
                let mut fields: Vec<Field> = MANDATORY_HEADERS
                    .iter()
                    .map(|(name, dtype)| Field::new(*name, dtype.clone(), true))
                    .collect();

                // Optional tags are recorded as Utf8.
                fields.extend(
                    sorted_tags
                        .iter()
                        .map(|tag| Field::new(tag, DataType::Utf8, true)),
                );

                let schema = Arc::new(Schema::new(fields));

                let props = WriterProperties::builder().build();
                let writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
                WriterImpl::Parquet {
                    writer,
                    schema,
                    buffer: Vec::with_capacity(1024),
                    batch_size: 1024,
                }
            }
        };

        Ok(OutputWriter {
            writer_impl,
            sorted_tags: sorted_tags.to_vec(),
        })
    }

    /// Writes the header row to the output.
    pub fn write_header(&mut self) -> Result<(), Box<dyn Error>> {
        let mandatory_names: Vec<String> = MANDATORY_HEADERS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();

        match &mut self.writer_impl {
            WriterImpl::Csv(csv_writer) => {
                let mut record = mandatory_names.clone();
                record.extend(self.sorted_tags.iter().cloned());
                csv_writer.write_record(&record)?;
            }
            WriterImpl::Delimited(writer, delimiter) => {
                let mut full_header = mandatory_names.join(delimiter);
                if !self.sorted_tags.is_empty() {
                    full_header.push_str(delimiter);
                    full_header.push_str(&self.sorted_tags.join(delimiter));
                }
                writeln!(writer, "{full_header}")?;
            }
            WriterImpl::Parquet { .. } => {
                // The schema is set at creation; no separate header write is needed.
            }
        }
        Ok(())
    }

    /// Writes a single data record to the output.
    pub fn write_record(&mut self, record: &DataRecord) -> Result<(), Box<dyn Error>> {
        match &mut self.writer_impl {
            WriterImpl::Csv(csv_writer) => {
                let mut values: Vec<String> = record.mandatory_fields.clone();
                values.extend(
                    self.sorted_tags
                        .iter()
                        .map(|tag| record.optional_fields.get(tag).cloned().unwrap_or_default()),
                );
                csv_writer.write_record(&values)?
            }
            WriterImpl::Delimited(writer, delimiter) => {
                let mut values: Vec<String> = record.mandatory_fields.clone();
                values.extend(
                    self.sorted_tags
                        .iter()
                        .map(|tag| record.optional_fields.get(tag).cloned().unwrap_or_default()),
                );
                writeln!(writer, "{}", values.join(delimiter))?
            }
            WriterImpl::Parquet {
                buffer, batch_size, ..
            } => {
                buffer.push(record.clone());
                if buffer.len() >= *batch_size {
                    self.flush_parquet_buffer()?;
                }
            }
        }
        Ok(())
    }

    /// Flushes any buffered Parquet records to the file.
    fn flush_parquet_buffer(&mut self) -> Result<(), Box<dyn Error>> {
        let WriterImpl::Parquet {
            writer,
            schema,
            buffer,
            ..
        } = &mut self.writer_impl
        else {
            return Ok(());
        };

        if buffer.is_empty() {
            return Ok(());
        }

        let mut columns: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());

        // Mandatory fields: use MANDATORY_HEADERS to decide builder/type.
        for (i, (_, dtype)) in MANDATORY_HEADERS.iter().enumerate() {
            match dtype {
                DataType::Int32 => {
                    let mut builder = Int32Builder::new();
                    for rec in buffer.iter() {
                        let text = rec
                            .mandatory_fields
                            .get(i)
                            .map(String::as_str)
                            .unwrap_or("");
                        if text.is_empty() {
                            builder.append_null();
                        } else {
                            match text.parse::<i32>() {
                                Ok(value) => builder.append_value(value),
                                Err(_) => builder.append_null(),
                            }
                        }
                    }
                    columns.push(Arc::new(builder.finish()));
                }
                DataType::Utf8 => {
                    let mut builder = StringBuilder::new();
                    for rec in buffer.iter() {
                        let text = rec
                            .mandatory_fields
                            .get(i)
                            .map(String::as_str)
                            .unwrap_or("");
                        if text.is_empty() {
                            builder.append_null();
                        } else {
                            builder.append_value(text);
                        }
                    }
                    columns.push(Arc::new(builder.finish()));
                }
                other => {
                    // Fallback: convert to string.
                    let mut builder = StringBuilder::new();
                    for rec in buffer.iter() {
                        let text = rec
                            .mandatory_fields
                            .get(i)
                            .map(String::as_str)
                            .unwrap_or("");
                        if text.is_empty() {
                            builder.append_null();
                        } else {
                            builder.append_value(text);
                        }
                    }
                    columns.push(Arc::new(builder.finish()));
                    eprintln!(
                        "Warning: unsupported mandatory data type in schema: {:?}",
                        other
                    );
                }
            }
        }

        // Optional fields: treat all as Utf8.
        for tag in &self.sorted_tags {
            let mut builder = StringBuilder::new();
            for rec in buffer.iter() {
                match rec.optional_fields.get(tag).map(String::as_str) {
                    Some(value) if !value.is_empty() => builder.append_value(value),
                    _ => builder.append_null(),
                }
            }
            columns.push(Arc::new(builder.finish()));
        }

        let batch = RecordBatch::try_new(schema.clone(), columns)?;
        writer.write(&batch)?;
        buffer.clear();
        Ok(())
    }

    /// Flushes the underlying writer and closes it.
    pub fn flush(mut self) -> Result<(), Box<dyn Error>> {
        self.flush_parquet_buffer()?;

        match self.writer_impl {
            WriterImpl::Csv(mut csv_writer) => csv_writer.flush()?,
            WriterImpl::Delimited(mut writer, _) => writer.flush()?,
            WriterImpl::Parquet { writer, .. } => {
                // `close` returns file metadata, which we don't need.
                writer.close()?;
            }
        }
        Ok(())
    }
}
