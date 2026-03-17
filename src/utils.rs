pub fn slice_last_lines(text: &str, requested_lines: usize) -> &str {
    if text.is_empty() || requested_lines == 0 {
        return "";
    }
    let mut seen = 0;
    let bytes = text.as_bytes();
    for (idx, &b) in bytes.iter().enumerate().rev() {
        if b == b'\n' {
            seen += 1;
            if seen > requested_lines {
                return &text[idx + 1..];
            }
        }
    }
    text
}

pub fn infer_prompt(viewport: &str, pwd: &str) -> bool {
    let trimmed = viewport.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    let last_line = slice_last_lines(trimmed, 1);
    if last_line.is_empty() {
        return false;
    }
    let last_char = last_line.chars().last().unwrap_or(' ');
    if last_char == '>' || last_char == '$' || last_char == '#' {
        return true;
    }
    if !pwd.is_empty() && last_line.starts_with(pwd) {
        return true;
    }
    false
}
