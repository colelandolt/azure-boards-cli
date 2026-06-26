//! Minimal ANSI colorizers for jsonc/yamlc. Only ever called when stdout is a
//! TTY and NO_COLOR is unset; machine output never passes through here.

const KEY: &str = "\x1b[36m"; // cyan
const STR: &str = "\x1b[32m"; // green
const NUM: &str = "\x1b[33m"; // yellow
const LIT: &str = "\x1b[35m"; // magenta (null/bool)
const RESET: &str = "\x1b[0m";

/// Colorize pretty-printed JSON line by line. Operates on output we generated
/// ourselves (serde_json pretty), so simple lexing is safe.
pub fn colorize_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for line in text.lines() {
        out.push_str(&colorize_json_line(line));
        out.push('\n');
    }
    out.pop();
    out
}

fn colorize_json_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    // "key": value
    if let Some(rest) = trimmed.strip_prefix('"') {
        if let Some(idx) = find_string_end(rest) {
            let (key, after) = rest.split_at(idx);
            if let Some(value_part) = after.strip_prefix("\": ") {
                return format!(
                    "{indent}{KEY}\"{key}\"{RESET}: {}",
                    colorize_json_value(value_part)
                );
            }
        }
    }
    format!("{indent}{}", colorize_json_value(trimmed))
}

fn colorize_json_value(v: &str) -> String {
    let (body, trail) = match v.strip_suffix(',') {
        Some(b) => (b, ","),
        None => (v, ""),
    };
    if body.starts_with('"') {
        format!("{STR}{body}{RESET}{trail}")
    } else if body == "null" || body == "true" || body == "false" {
        format!("{LIT}{body}{RESET}{trail}")
    } else if body.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        format!("{NUM}{body}{RESET}{trail}")
    } else {
        v.to_string()
    }
}

fn find_string_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Colorize YAML: keys before the first ": ", literals and numbers after.
pub fn colorize_yaml(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let content = trimmed.strip_prefix("- ").map(|r| ("- ", r));
        let (prefix, rest) = content.unwrap_or(("", trimmed));
        if let Some((key, val)) = rest.split_once(": ") {
            out.push_str(&format!(
                "{indent}{prefix}{KEY}{key}{RESET}: {}",
                colorize_yaml_value(val)
            ));
        } else if let Some(key) = rest.strip_suffix(':') {
            out.push_str(&format!("{indent}{prefix}{KEY}{key}{RESET}:"));
        } else {
            out.push_str(&format!("{indent}{prefix}{}", colorize_yaml_value(rest)));
        }
        out.push('\n');
    }
    out
}

fn colorize_yaml_value(v: &str) -> String {
    if v == "null" || v == "~" || v == "true" || v == "false" {
        format!("{LIT}{v}{RESET}")
    } else if v.starts_with(|c: char| c.is_ascii_digit() || c == '-') && v.parse::<f64>().is_ok() {
        format!("{NUM}{v}{RESET}")
    } else {
        format!("{STR}{v}{RESET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_keys_and_values_get_colors() {
        let colored = colorize_json("{\n  \"id\": 5,\n  \"name\": \"x\"\n}");
        assert!(colored.contains("\x1b[36m\"id\"\x1b[0m"));
        assert!(colored.contains("\x1b[33m5\x1b[0m"));
        assert!(colored.contains("\x1b[32m\"x\"\x1b[0m"));
    }
}
