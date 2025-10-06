#[derive(Debug)]
struct PendingBlock {
    opening_indent: usize,
}

#[derive(Debug)]
struct OpenBlock {
    content_indent: usize,
    brace_indent: usize,
}

fn line_indent(line: &str) -> (usize, &'_ str) {
    let mut count = 0;
    for (idx, ch) in line.char_indices() {
        match ch {
            ' ' => count += 1,
            '\t' => count += 4,
            _ => return (count, &line[idx..]),
        }
    }
    (count, "")
}

fn strip_trailing(line: &str) -> (&'_ str, &'_ str) {
    let trimmed = line.trim_end_matches(|c: char| c == ' ' || c == '\t');
    let ws_len = line.len() - trimmed.len();
    (&line[..trimmed.len()], &line[trimmed.len()..])
}

fn ends_with_colon(candidate: &str) -> bool {
    if !candidate.ends_with(':') {
        return false;
    }
    if candidate.ends_with("::") {
        return false;
    }
    true
}

fn convert_do_segment(segment: &str) -> Option<String> {
    if let Some(pos) = segment.find("=> do") {
        let before = &segment[..pos];
        let after = &segment[pos + 5..];
        if after.trim().is_empty() {
            let mut out = before.to_string();
            out.push_str("=> {");
            return Some(out);
        }
        let mut out = before.to_string();
        out.push_str("=> {");
        out.push(' ');
        out.push_str(after.trim_start());
        out.push_str(" }");
        return Some(out);
    }
    None
}

fn convert_struct_like(segment: &str, keyword: &str) -> Option<String> {
    let needle = format!(":: {}", keyword);
    if segment.trim_end().ends_with(&needle) {
        let trimmed = segment.trim_end();
        let prefix = &trimmed[..trimmed.len() - needle.len()];
        let mut out = prefix.to_string();
        out.push_str(&format!(":: {} {{", keyword));
        return Some(out);
    }
    None
}

fn convert_block_leader(segment: &str) -> (String, bool) {
    if let Some(converted) = convert_do_segment(segment) {
        let opens_block =
            converted.trim_end().ends_with("{") && segment.trim_end().ends_with("=> do");
        return (converted, opens_block);
    }

    for keyword in ["struct", "table", "compose", "db"] {
        if let Some(converted) = convert_struct_like(segment, keyword) {
            return (converted, true);
        }
    }

    let (stripped, trailing_ws) = strip_trailing(segment);
    if ends_with_colon(stripped) {
        let mut base = stripped[..stripped.len() - 1].trim_end().to_string();
        base.push_str(" {");
        base.push_str(trailing_ws);
        return (base, true);
    }

    (segment.to_string(), false)
}

pub fn desugar_indentation(source: &str) -> String {
    let mut normalized = source.replace("\r\n", "\n");
    normalized = normalized.replace('\r', "\n");

    let mut output = String::with_capacity(normalized.len() + 128);
    let mut pending: Vec<PendingBlock> = Vec::new();
    let mut open_blocks: Vec<OpenBlock> = Vec::new();
    let mut lines = normalized.split_inclusive('\n').peekable();

    while let Some(raw_line) = lines.next() {
        let mut line = raw_line;
        let mut has_newline = false;
        if line.ends_with('\n') {
            has_newline = true;
            line = &line[..line.len() - 1];
        }

        let (indent, rest) = line_indent(line);
        let trimmed = rest.trim();
        let is_comment_line = trimmed.starts_with('#');

        if !trimmed.is_empty() {
            while let Some(last) = open_blocks.last() {
                if indent < last.content_indent {
                    let block = open_blocks.pop().unwrap();
                    let mut close_line = String::new();
                    close_line.push_str(&" ".repeat(block.brace_indent));
                    close_line.push('}');
                    output.push_str(&close_line);
                    output.push('\n');
                } else {
                    break;
                }
            }
        }

        if !trimmed.is_empty() {
            while let Some(last) = pending.last() {
                if indent > last.opening_indent {
                    let block = pending.pop().unwrap();
                    open_blocks.push(OpenBlock {
                        content_indent: indent,
                        brace_indent: block.opening_indent,
                    });
                } else {
                    break;
                }
            }
        }

        if trimmed.is_empty() {
            output.push_str(line);
            if has_newline {
                output.push('\n');
            }
            continue;
        }

        if is_comment_line {
            output.push_str(line);
            if has_newline {
                output.push('\n');
            }
            continue;
        }

        let (converted_core, opens_block) = convert_block_leader(rest);
        output.push_str(&" ".repeat(indent));
        output.push_str(&converted_core);
        if has_newline {
            output.push('\n');
        }

        if opens_block {
            pending.push(PendingBlock {
                opening_indent: indent,
            });
        }
    }

    while let Some(block) = open_blocks.pop() {
        let mut close_line = String::new();
        close_line.push_str(&" ".repeat(block.brace_indent));
        close_line.push('}');
        output.push_str(&close_line);
        output.push('\n');
    }

    while let Some(block) = pending.pop() {
        let mut close_line = String::new();
        close_line.push_str(&" ".repeat(block.opening_indent));
        close_line.push_str("{}");
        output.push_str(&close_line);
        output.push('\n');
    }

    output
}
