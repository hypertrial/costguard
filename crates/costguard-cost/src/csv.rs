use anyhow::Result;

pub fn csv_records(input: &str) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            other => field.push(other),
        }
    }
    if quoted {
        anyhow::bail!("unterminated quoted CSV field");
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_quoted_field() {
        let text = "model_or_table,bytes_per_run\n\"line1\nline2\",1000\n";
        let records = csv_records(text).expect("records");
        assert_eq!(records[1][0], "line1\nline2");
    }
}
