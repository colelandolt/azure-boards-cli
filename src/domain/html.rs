//! Minimal HTML handling for ADO rich-text fields: tag stripping, whitespace
//! collapse, entity decode, and inline <img> extraction with text context.
//! Matches the behavior of the legacy bash script (`ado inline-images`).

/// An inline image with the text immediately preceding it (up to 300 chars).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct InlineImage {
    pub index: usize,
    pub url: String,
    pub context: String,
}

pub fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&") // last, so double-encoded text stays sane
}

/// Normalized plain text for a rich-text field.
pub fn to_text(html: &str) -> String {
    // Block-level breaks become newlines before stripping.
    let with_breaks = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n");
    let stripped = strip_tags(&with_breaks);
    let decoded = decode_entities(&stripped);
    decoded
        .lines()
        .map(collapse_whitespace)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split HTML on <img> tags, returning each image src with the preceding
/// text context (stripped, collapsed, last 300 chars) — bash-script parity.
pub fn extract_images(html: &str) -> Vec<InlineImage> {
    let mut images = Vec::new();
    let mut preceding = String::new();
    let mut rest = html;
    let mut index = 1;
    while let Some(start) = rest.find("<img") {
        preceding.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find('>').map(|i| i + 1).unwrap_or(after.len());
        let tag = &after[..end];
        if let Some(url) = attr_value(tag, "src") {
            let context = collapse_whitespace(&decode_entities(&strip_tags(&preceding)));
            let context = last_chars(&context, 300);
            images.push(InlineImage {
                index,
                url,
                context,
            });
            index += 1;
            preceding.clear();
        }
        rest = &after[end..];
    }
    images
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(decode_entities(&tag[start..end]))
}

fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        s.chars().skip(count - n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_and_collapses() {
        assert_eq!(
            to_text("<div>Hello <b>world</b></div><p>Second&nbsp;line</p>"),
            "Hello world\nSecond line"
        );
    }

    #[test]
    fn extracts_images_with_context() {
        let html = r#"<p>Steps to reproduce the crash:</p><img src="https://dev.azure.com/org/_apis/wit/attachments/abc?fileName=a.png"><p>and after clicking save</p><img src="https://dev.azure.com/org/_apis/wit/attachments/def">"#;
        let images = extract_images(html);
        assert_eq!(images.len(), 2);
        assert!(images[0].url.contains("attachments/abc"));
        assert_eq!(images[0].context, "Steps to reproduce the crash:");
        assert_eq!(images[0].index, 1);
        assert_eq!(images[1].context, "and after clicking save");
    }

    #[test]
    fn context_capped_at_300_chars() {
        let long = "x".repeat(400);
        let html = format!(r#"<p>{long}</p><img src="u">"#);
        let images = extract_images(&html);
        assert_eq!(images[0].context.chars().count(), 300);
    }

    #[test]
    fn img_without_src_skipped() {
        assert!(extract_images("<img alt=\"x\"> nothing").is_empty());
    }
}
