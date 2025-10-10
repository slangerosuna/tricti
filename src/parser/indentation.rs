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

fn split_block_colon(candidate: &str) -> Option<(&str, &str)> {
    let mut angle_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut idx = 0;
    let bytes = candidate.as_bytes();
    while idx < bytes.len() {
        match bytes[idx] {
            b'<' => angle_depth += 1,
            b'>' => {
                if angle_depth > 0 {
                    angle_depth -= 1;
                }
            }
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            b'{' => brace_depth += 1,
            b'}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
            }
            b':' => {
                let after = &candidate[idx + 1..];
                let colon_ready = angle_depth == 0 && paren_depth == 0 && brace_depth == 0
                    || (angle_depth > 0
                        && paren_depth == 0
                        && brace_depth == 0
                        && !after.contains('>'));
                if !colon_ready {
                    idx += 1;
                    continue;
                }

                let before = &candidate[..idx];

                if before.ends_with(':') {
                    idx += 1;
                    continue;
                }
                if after.starts_with(':') || after.starts_with('=') {
                    idx += 1;
                    continue;
                }

                return Some((before, after));
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn leading_keyword_allows_block(head: &str) -> bool {
    let trimmed = head.trim_start();
    let mut tokens = trimmed.split_whitespace();
    if let Some(first) = tokens.next() {
        if matches!(
            first,
            "if" | "else" | "for" | "loop" | "while" | "unsafe" | "match" | "impl"
        ) || first.starts_with("impl<")
        {
            return true;
        }
        if first == "mod" {
            return true;
        }
        if first == "pub" {
            if let Some(next) = tokens.next() {
                if next == "mod" || next == "impl" {
                    return true;
                }
                if next == "fn" {
                    // fall through for blocks like "pub fn" where colon shouldn't open block
                }
            }
        }
        if first == "}" {
            if let Some(next) = tokens.next() {
                if matches!(next, "else") {
                    return true;
                }
            }
        }
    }

    trimmed
        .split_whitespace()
        .any(|token| matches!(token, "match"))
}

fn looks_like_struct_literal_head(head: &str) -> bool {
    let trimmed = head.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.contains("=>") {
        return false;
    }

    let (has_assign, after_assign) = if let Some((_, rhs)) = trimmed.split_once(":=") {
        (true, rhs.trim_start())
    } else if let Some((_, rhs)) = trimmed.split_once('=') {
        (true, rhs.trim_start())
    } else {
        (false, "")
    };

    if !has_assign {
        return false;
    }

    if let Some(keyword) = after_assign.split_whitespace().next() {
        if matches!(
            keyword,
            "match" | "if" | "for" | "while" | "loop" | "unsafe" | "do"
        ) {
            return false;
        }
    }

    let mut chars = trimmed.chars();
    let first = match chars.next() {
        Some(ch) => ch,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    for ch in chars {
        if ch.is_ascii_alphanumeric() {
            continue;
        }
        if matches!(
            ch,
            '_' | ':'
                | '<'
                | '>'
                | ','
                | ' '
                | '['
                | ']'
                | '.'
                | ';'
                | '\''
                | '&'
                | '*'
                | '?'
                | '='
        ) {
            continue;
        }
        return false;
    }

    true
}

fn convert_do_segment(segment: &str) -> Option<String> {
    if let Some(pos) = segment.find("=> do") {
        let before = &segment[..pos];
        let after = &segment[pos + 5..];
        let trimmed_after = after.trim_start();

        if trimmed_after.is_empty() {
            let mut out = before.to_string();
            out.push_str("=> {");
            return Some(out);
        }
        if trimmed_after.starts_with('{') {
            let mut out = before.to_string();
            out.push_str("=>");
            if after.starts_with(' ') || after.starts_with('\t') {
                out.push(' ');
            }
            out.push_str(trimmed_after);
            return Some(out);
        }
        let (body_core, trailing_ws) = strip_trailing(trimmed_after);
        let mut body = body_core.trim_end().to_string();
        if body.ends_with('}') {
            body.pop();
            while body.ends_with(' ') || body.ends_with('\t') {
                body.pop();
            }
        }

        let mut out = before.to_string();
        out.push_str("=> {");
        if !body.is_empty() {
            out.push(' ');
            out.push_str(&body);
            out.push(' ');
        }
        out.push('}');
        out.push_str(trailing_ws);
        return Some(out);
    }
    None
}

fn convert_struct_like(segment: &str, keyword: &str) -> Option<String> {
    let trimmed_start = segment.trim_start();
    if trimmed_start.starts_with("use ") || trimmed_start.starts_with("pub use ") {
        return None;
    }

    let (core, trailing_ws) = strip_trailing(segment);
    let direct = format!(":: {}", keyword);

    if core.ends_with(&direct) {
        let prefix = &core[..core.len() - direct.len()];
        let mut out = String::with_capacity(core.len() + 3 + trailing_ws.len());
        out.push_str(prefix);
        out.push_str(&direct);
        out.push_str(" {");
        out.push_str(trailing_ws);
        return Some(out);
    }

    let trimmed = core.trim_end();
    if let Some(idx) = trimmed.rfind("::") {
        let after = trimmed[idx + 2..].trim();
        if after.is_empty() {
            return None;
        }

        let tokens: Vec<&str> = after.split_whitespace().collect();
        if tokens.is_empty() {
            return None;
        }

        if tokens.last().copied() != Some(keyword) {
            return None;
        }

        let attrs_ok = tokens[..tokens.len() - 1]
            .iter()
            .all(|token| token.starts_with('@'));
        if !attrs_ok {
            return None;
        }

        let prefix = &trimmed[..idx];
        let mut out = String::with_capacity(core.len() + 3 + trailing_ws.len());
        out.push_str(prefix);
        out.push_str(":: ");
        out.push_str(&tokens.join(" "));
        out.push_str(" {");
        out.push_str(trailing_ws);
        return Some(out);
    }

    None
}

fn convert_attribute_args(segment: &str) -> Option<String> {
    let (core, trailing_ws) = strip_trailing(segment);
    let trimmed = core.trim_start();
    if !trimmed.starts_with('@') {
        return None;
    }

    if trimmed.contains('(') {
        return None;
    }

    let leading = &core[..core.len() - trimmed.len()];
    let rest = &trimmed[1..];
    let mut name_end = rest.len();
    for (idx, ch) in rest.char_indices() {
        if ch == ' ' || ch == '\t' {
            name_end = idx;
            break;
        }
    }

    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }

    let args = rest[name_end..].trim();
    if args.is_empty() {
        return None;
    }

    if args.contains(':') {
        return None;
    }

    let mut out = String::with_capacity(segment.len() + 2);
    out.push_str(leading);
    out.push('@');
    out.push_str(name);
    out.push('(');
    out.push_str(args);
    out.push(')');
    out.push_str(trailing_ws);
    Some(out)
}

fn convert_inline_segment(segment: &str) -> String {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some((head, tail)) = split_block_colon(trimmed) {
        if leading_keyword_allows_block(head) {
            let inner_body = tail.trim_start();
            let trimmed_head = head.trim_end();

            if inner_body.is_empty() {
                let mut out = trimmed_head.to_string();
                out.push_str(" {}");
                return out;
            }

            if let Some(idx) = inner_body.find(" else:") {
                let (then_part, else_raw) = inner_body.split_at(idx);
                let else_part = else_raw[" else:".len()..].trim_start();
                let then_converted = convert_inline_segment(then_part);
                let else_converted = convert_inline_segment(else_part);

                let mut out = trimmed_head.to_string();
                out.push_str(" {");
                if !then_converted.is_empty() {
                    out.push(' ');
                    out.push_str(&then_converted);
                    out.push(' ');
                }
                out.push_str("}");
                out.push(' ');
                out.push_str("else {");
                if !else_converted.is_empty() {
                    out.push(' ');
                    out.push_str(&else_converted);
                    out.push(' ');
                }
                out.push('}');
                return out;
            }

            if inner_body.starts_with('{') {
                let mut out = trimmed_head.to_string();
                out.push(' ');
                out.push_str(inner_body);
                return out;
            }

            let nested = convert_inline_segment(inner_body);
            let mut out = trimmed_head.to_string();
            out.push_str(" {");
            if !nested.is_empty() {
                out.push(' ');
                out.push_str(&nested);
                out.push(' ');
            }
            out.push('}');
            return out;
        }
    }

    trimmed.to_string()
}

fn convert_block_leader(segment: &str) -> (String, bool) {
    let trace_question_else = std::env::var("TRACE_QUESTION_ELSE").is_ok();
    if trace_question_else && segment.contains("? else") {
        eprintln!("TRACE segment: {:?}", segment);
    }
    if let Some(converted) = convert_do_segment(segment) {
        let opens_block =
            converted.trim_end().ends_with("{") && segment.trim_end().ends_with("=> do");
        return ensure_arrow_block(converted, opens_block);
    }

    if let Some(converted) = convert_attribute_args(segment) {
        return (converted, false);
    }

    for keyword in ["struct", "table", "compose", "db", "enum", "trait"] {
        if let Some(converted) = convert_struct_like(segment, keyword) {
            return ensure_arrow_block(converted, true);
        }
    }

    let (stripped, trailing_ws) = strip_trailing(segment);
    if let Some((head, tail)) = split_block_colon(stripped) {
        let trimmed_head = head.trim_end();
        if leading_keyword_allows_block(head) {
            let inline_body = tail.trim_start();
            if inline_body.is_empty() {
                let mut base = trimmed_head.to_string();
                base.push_str(" {");
                base.push_str(trailing_ws);
                return ensure_arrow_block(base, true);
            }
            if let Some(idx) = inline_body.find(" else:") {
                let (then_part, else_part_raw) = inline_body.split_at(idx);
                let else_part = else_part_raw[" else:".len()..].trim_start();
                let then_converted = convert_inline_segment(then_part);
                let else_converted = convert_inline_segment(else_part);
                let mut base = trimmed_head.to_string();
                base.push_str(" {");
                if !then_converted.is_empty() {
                    base.push(' ');
                    base.push_str(&then_converted);
                    base.push(' ');
                }
                base.push('}');
                base.push(' ');
                base.push_str("else {");
                if !else_converted.is_empty() {
                    base.push(' ');
                    base.push_str(&else_converted);
                    base.push(' ');
                }
                base.push('}');
                base.push_str(trailing_ws);
                return (base, false);
            }
            if inline_body.starts_with('{') {
                let mut base = trimmed_head.to_string();
                base.push(' ');
                base.push_str(inline_body);
                base.push_str(trailing_ws);
                return ensure_arrow_block(base, false);
            }
            let converted_inline = convert_inline_segment(inline_body);
            let mut base = trimmed_head.to_string();
            base.push_str(" {");
            base.push(' ');
            base.push_str(&converted_inline);
            base.push_str(" }");
            base.push_str(trailing_ws);
            return ensure_arrow_block(base, false);
        } else if trimmed_head.ends_with("? else") {
            if trace_question_else {
                eprintln!(
                    "TRACE matched question_else: head={:?} tail={:?}",
                    trimmed_head, tail
                );
            }
            let inline_body = tail.trim_start();
            if inline_body.is_empty() {
                let mut base = trimmed_head.to_string();
                base.push_str(" {");
                base.push_str(trailing_ws);
                if trace_question_else {
                    eprintln!("TRACE returns block opener: {:?}", base);
                }
                return (base, true);
            }
            if inline_body.starts_with('{') {
                let mut base = trimmed_head.to_string();
                base.push(' ');
                base.push_str(inline_body);
                base.push_str(trailing_ws);
                if trace_question_else {
                    eprintln!("TRACE returns inline brace body: {:?}", base);
                }
                return (base, false);
            }
            let converted_inline = convert_inline_segment(inline_body);
            let mut base = trimmed_head.to_string();
            base.push_str(" {");
            if !converted_inline.is_empty() {
                base.push(' ');
                base.push_str(&converted_inline);
                base.push(' ');
            }
            base.push('}');
            base.push_str(trailing_ws);
            if trace_question_else {
                eprintln!("TRACE returns inline converted: {:?}", base);
            }
            return (base, false);
        } else if tail.trim().is_empty() && looks_like_struct_literal_head(trimmed_head) {
            let mut base = trimmed_head.to_string();
            base.push_str(" {");
            base.push_str(trailing_ws);
            return ensure_arrow_block(base, true);
        } else if let Some(arrow_idx) = trimmed_head.rfind("=>") {
            let after_arrow = trimmed_head[arrow_idx + 2..].trim_start();
            if leading_keyword_allows_block(after_arrow) {
                let inline_body = tail.trim_start();
                if inline_body.is_empty() {
                    let mut base = trimmed_head.to_string();
                    base.push_str(" {");
                    base.push_str(trailing_ws);
                    return (base, true);
                }
                if inline_body.starts_with('{') {
                    let mut base = trimmed_head.to_string();
                    base.push(' ');
                    base.push_str(inline_body);
                    base.push_str(trailing_ws);
                    return (base, false);
                }
                let converted_inline = convert_inline_segment(inline_body);
                let mut base = trimmed_head.to_string();
                base.push_str(" {");
                if !converted_inline.is_empty() {
                    base.push(' ');
                    base.push_str(&converted_inline);
                    base.push(' ');
                }
                base.push('}');
                base.push_str(trailing_ws);
                return (base, false);
            }
        }
    }

    let trimmed = stripped.trim_start();
    if trimmed.starts_with("else") {
        let mut remainder = &trimmed["else".len()..];
        if let Some(ch) = remainder.chars().next() {
            if ch == ':' {
                // fall back to default handling for colon-style else
            } else if ch.is_whitespace() {
                remainder = remainder.trim_start();
                let converted = convert_inline_segment(remainder);
                let mut base = String::from("else {");
                if !converted.is_empty() {
                    base.push(' ');
                    base.push_str(&converted);
                    base.push(' ');
                }
                base.push('}');
                base.push_str(trailing_ws);
                return (base, false);
            } else {
                // else followed immediately by identifier without space; treat similarly
                let converted = convert_inline_segment(remainder);
                let mut base = String::from("else {");
                if !converted.is_empty() {
                    base.push(' ');
                    base.push_str(&converted);
                    base.push(' ');
                }
                base.push('}');
                base.push_str(trailing_ws);
                return (base, false);
            }
        } else {
            let mut base = String::from("else {");
            base.push('}');
            base.push_str(trailing_ws);
            return (base, false);
        }
    }

    ensure_arrow_block(segment.to_string(), false)
}

fn ensure_arrow_block(base: String, opens_block: bool) -> (String, bool) {
    let (core, trailing_ws) = strip_trailing(&base);
    let trimmed = core.trim_end();
    if trimmed.ends_with("=>") && !trimmed.ends_with("=> {") {
        let mut expanded = trimmed.to_string();
        expanded.push_str(" {");
        expanded.push_str(trailing_ws);
        return (expanded, true);
    }
    (base, opens_block)
}

pub fn desugar_indentation(source: &str) -> String {
    let mut normalized = source.replace("\r\n", "\n");
    normalized = normalized.replace('\r', "\n");

    let mut output = String::with_capacity(normalized.len() + 128);
    let mut pending: Vec<PendingBlock> = Vec::new();
    let mut open_blocks: Vec<OpenBlock> = Vec::new();
    let mut lines = normalized.split_inclusive('\n').peekable();
    #[cfg(test)]
    let trace = std::env::var("TRACE_INDENT").ok().is_some();
    #[cfg(not(test))]
    let trace = false;

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

        let mut leading_closers = if trimmed.is_empty() {
            0
        } else {
            trimmed.chars().take_while(|&c| c == '}').count()
        };

        if !trimmed.is_empty() {
            while let Some(last) = open_blocks.last() {
                if indent < last.content_indent {
                    let block = open_blocks.pop().unwrap();
                    if leading_closers > 0 {
                        leading_closers -= 1;
                        if trace {
                            eprintln!(
                                "consumed leading close at indent {} (content {}) before line {:?}",
                                block.brace_indent, block.content_indent, trimmed
                            );
                        }
                        continue;
                    }
                    if trace {
                        eprintln!(
                            "closing block at indent {} (content {}) before line {:?}",
                            block.brace_indent, block.content_indent, trimmed
                        );
                    }
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
                    if trace {
                        eprintln!(
                            "opening block content indent {} brace {}",
                            indent, block.opening_indent
                        );
                    }
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

        let mut adjusted_segment_storage: Option<String> = None;
        let segment = if rest.starts_with("elif") {
            let next = rest.chars().nth(4);
            if next.map(|c| c.is_whitespace() || c == ':').unwrap_or(false) {
                let mut rebuilt = String::from("else if");
                rebuilt.push_str(&rest[4..]);
                adjusted_segment_storage = Some(rebuilt);
                adjusted_segment_storage.as_ref().unwrap().as_str()
            } else {
                rest
            }
        } else {
            rest
        };

        let (converted_core, opens_block) = convert_block_leader(segment);
        let trimmed_converted = converted_core.trim_start();
        let attach_to_previous = (trimmed.starts_with("else") || trimmed.starts_with("elif"))
            && (trimmed_converted.starts_with("else {")
                || trimmed_converted.starts_with("else if"));

        if attach_to_previous {
            while output.ends_with(' ') || output.ends_with('\t') {
                output.pop();
            }
            if output.ends_with('\n') {
                output.pop();
                while output.ends_with(' ') || output.ends_with('\t') {
                    output.pop();
                }
            }
            if !output.is_empty() && !output.ends_with(' ') && !output.ends_with('{') {
                output.push(' ');
            }
            output.push_str(trimmed_converted);
        } else {
            output.push_str(&" ".repeat(indent));
            output.push_str(&converted_core);
        }
        if has_newline {
            output.push('\n');
        }

        if opens_block {
            if trace {
                eprintln!("pending block indent {} for {:?}", indent, rest.trim_end());
            }
            pending.push(PendingBlock {
                opening_indent: indent,
            });
        }
    }

    while let Some(block) = open_blocks.pop() {
        if trace {
            eprintln!(
                "final close block indent {} content {}",
                block.brace_indent, block.content_indent
            );
        }
        let mut close_line = String::new();
        close_line.push_str(&" ".repeat(block.brace_indent));
        close_line.push('}');
        output.push_str(&close_line);
        output.push('\n');
    }

    while let Some(block) = pending.pop() {
        if trace {
            eprintln!("final pending block indent {}", block.opening_indent);
        }
        let mut close_line = String::new();
        close_line.push_str(&" ".repeat(block.opening_indent));
        close_line.push_str("{}");
        output.push_str(&close_line);
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{convert_block_leader, desugar_indentation};

    #[test]
    fn converts_for_inline_unsafe_block() {
        let input = "for i in 0..self.len: unsafe: *(new_data + i) = *(self.data + i)\n";
        let output = desugar_indentation(input);
        let (converted, _) = convert_block_leader(
            "for i in 0..self.len: unsafe: *(new_data + i) = *(self.data + i)",
        );
        assert!(output.contains("for i in 0..self.len { unsafe {"));
        assert!(!output.contains(": unsafe"));
    }

    #[test]
    fn converts_else_without_colon() {
        let input = "if index >= self.len: ret none\nelse some unsafe { *(self.data + index) }\n";
        let output = desugar_indentation(input);
        eprintln!("OUTPUT:{}", output);
        assert!(output.contains(
            "if index >= self.len { ret none } else { some unsafe { *(self.data + index) } }"
        ));
    }

    #[test]
    fn converts_colon_else_block() {
        let input = "if value < 0:\n    ret -value\nelse:\n    ret value\n";
        let output = desugar_indentation(input);
        eprintln!("COLON OUTPUT:{}", output);
        eprintln!("SPLIT:{:?}", super::split_block_colon("if value < 0:"));
        assert!(output.contains("if value < 0 {"));
        assert!(output.contains("} else {"));
    }

    #[test]
    fn converts_elif_chain() {
        let input = "if ch >= 0:\n    ret 1\nelif ch == 1:\n    ret 2\nelse:\n    ret 3\n";
        let output = desugar_indentation(input);
        eprintln!("ELIF OUTPUT:{}", output);
        assert!(output.contains("if ch >= 0 {"));
        assert!(output.contains("} else if ch == 1 {"));
        assert!(output.contains("} else {"));
    }

    #[test]
    fn converts_arrow_match_block() {
        let input = "fmt :: () => match value:\n    some item => do\n        ret item\n    none => do\n        ret none\n";
        let output = desugar_indentation(input);
        assert!(output.contains("=> match value {"));
        assert!(output.contains("some item =>"));
        assert!(output.contains("none =>"));
    }

    #[test]
    fn converts_struct_literal_block() {
        let input = "result := TripmBus:\n    build_requests: build_requests,\n    build_sender: build_sender,\n    build_receiver: build_receiver,\n";
        let output = desugar_indentation(input);
        assert!(output.contains("result := TripmBus {"));
        assert!(output.contains("build_requests: build_requests,"));
        assert!(output.contains("build_receiver: build_receiver,"));
        assert!(output.contains("}"));
    }

    #[test]
    fn converts_struct_literal_block_with_path() {
        let input = "component := services::TripmBus:\n    build_requests: build_requests,\n";
        let output = desugar_indentation(input);
        assert!(output.contains("component := services::TripmBus {"));
        assert!(output.contains("build_requests: build_requests,"));
        assert!(output.contains("}"));
    }

    #[test]
    fn converts_struct_literal_block_with_type_arguments() {
        let input = "entry := Wrapper<TripmBus>:\n    inner: TripmBus::new(),\n";
        let output = desugar_indentation(input);
        assert!(output.contains("entry := Wrapper<TripmBus> {"));
        assert!(output.contains("inner: TripmBus::new(),"));
        assert!(output.contains("}"));
    }

    #[test]
    fn converts_question_else_block() {
        let input = "    file := File::open(path)? else:\n        ret none\n";
        let output = desugar_indentation(&input);
        assert!(output.contains("file := File::open(path)? else {"));
        assert!(output.contains("        ret none"));
        assert!(output.contains("    }"));
    }

    #[test]
    #[ignore]
    fn dump_collections_desugar() {
        let src = std::fs::read_to_string("../tripm/src/main.tri").unwrap();
        let output = desugar_indentation(&src);
        std::fs::write("tmp/tripm_main_desugared.tri", &output).unwrap();
        println!("wrote tmp/tripm_main_desugared.tri");
        panic!("dump complete");
    }
}
