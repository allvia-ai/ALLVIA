use anyhow::{anyhow, Result};
use calamine::{open_workbook, Data, Reader, Xlsx};
use pdf_extract::extract_text as extract_pdf_text;
use std::fs;
use std::path::Path;

pub struct ContentExtractor;

impl ContentExtractor {
    pub fn extract(path: &str) -> Result<String> {
        let p = Path::new(path);
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "txt" | "md" | "json" | "csv" | "rs" | "py" | "js" | "html" => {
                let content = fs::read_to_string(p)?;
                Ok(content)
            }
            "pdf" => {
                let text = extract_pdf_text(path).map_err(|e| anyhow!("PDF Error: {}", e))?;
                Ok(text)
            }
            "xlsx" | "xls" => {
                let mut workbook: Xlsx<_> =
                    open_workbook(path).map_err(|e| anyhow!("Excel Error: {}", e))?;

                let mut output = String::new();

                if let Some(name) = workbook.sheet_names().first().cloned() {
                    if let Ok(range) = workbook.worksheet_range(&name) {
                        for row in range.rows() {
                            let row_str: Vec<String> = row
                                .iter()
                                .map(|c| match c {
                                    Data::String(s) => s.clone(),
                                    Data::Float(f) => f.to_string(),
                                    Data::Int(i) => i.to_string(),
                                    Data::Bool(b) => b.to_string(),
                                    _ => "".to_string(),
                                })
                                .collect();
                            output.push_str(&row_str.join(", "));
                            output.push('\n');
                        }
                    }
                }
                Ok(output)
            }
            _ => Err(anyhow!("Unsupported file extension: {}", ext)),
        }
    }
}
