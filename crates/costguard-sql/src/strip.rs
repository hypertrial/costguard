#[derive(Debug, Clone, Default)]
pub struct JinjaStripMap {
    segments: Vec<StripSegment>,
}

#[derive(Debug, Clone, Copy)]
struct StripSegment {
    sanitized_start: usize,
    sanitized_end: usize,
    raw_start: usize,
}

impl JinjaStripMap {
    pub fn map_sanitized_range(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        if start >= end {
            return None;
        }
        let raw_start = self.raw_offset(start)?;
        let raw_end = self.raw_offset(end.saturating_sub(1))? + 1;
        Some((raw_start, raw_end))
    }

    fn raw_offset(&self, sanitized_index: usize) -> Option<usize> {
        for segment in &self.segments {
            if sanitized_index >= segment.sanitized_start && sanitized_index < segment.sanitized_end
            {
                let delta = sanitized_index - segment.sanitized_start;
                return Some(segment.raw_start + delta);
            }
        }
        None
    }
}

pub fn strip_jinja(text: &str) -> String {
    strip_jinja_with_map(text).0
}

pub fn strip_jinja_with_map(text: &str) -> (String, JinjaStripMap) {
    let mut sanitized = String::with_capacity(text.len());
    let mut segments = Vec::new();
    let mut index = 0usize;
    while index < text.len() {
        if let Some(start) = find_jinja_start(&text[index..]) {
            let abs_start = index + start;
            if abs_start > index {
                push_literal_segment(text, index, abs_start, &mut sanitized, &mut segments);
            }
            if let Some(end) = find_jinja_end(&text[abs_start..]) {
                let abs_end = abs_start + end;
                let placeholder = if text[abs_start..].starts_with("{%") {
                    " "
                } else {
                    " __jinja__ "
                };
                let sanitized_start = sanitized.len();
                sanitized.push_str(placeholder);
                segments.push(StripSegment {
                    sanitized_start,
                    sanitized_end: sanitized.len(),
                    raw_start: abs_start,
                });
                index = abs_end;
                continue;
            }
        }
        push_literal_segment(text, index, text.len(), &mut sanitized, &mut segments);
        break;
    }
    (sanitized, JinjaStripMap { segments })
}

fn push_literal_segment(
    text: &str,
    raw_start: usize,
    raw_end: usize,
    sanitized: &mut String,
    segments: &mut Vec<StripSegment>,
) {
    if raw_start >= raw_end {
        return;
    }
    let slice = &text[raw_start..raw_end];
    let sanitized_start = sanitized.len();
    sanitized.push_str(slice);
    segments.push(StripSegment {
        sanitized_start,
        sanitized_end: sanitized.len(),
        raw_start,
    });
}

fn find_jinja_start(text: &str) -> Option<usize> {
    text.find("{{").or_else(|| text.find("{%"))
}

fn find_jinja_end(text: &str) -> Option<usize> {
    if text.starts_with("{{") {
        return text.find("}}").map(|idx| idx + 2);
    }
    if text.starts_with("{%") {
        return text.find("%}").map(|idx| idx + 2);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::LineIndex;

    #[test]
    fn maps_spans_through_jinja_replacement() {
        let text = "select * from {{ ref('orders') }} cross join t";
        let (sanitized, map) = strip_jinja_with_map(text);
        assert!(sanitized.contains("__jinja__"));
        let cross_start = sanitized.find("cross join").expect("cross join");
        let cross_end = cross_start + "cross join".len();
        let (raw_start, raw_end) = map
            .map_sanitized_range(cross_start, cross_end)
            .expect("mapped span");
        assert_eq!(&text[raw_start..raw_end], "cross join");
        let line_index = LineIndex::new(text);
        let span = line_index.span(raw_start, raw_end);
        assert!(span.line >= 1);
    }
}
