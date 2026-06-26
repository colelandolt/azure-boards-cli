//! Markdown -> HTML conversion for work item rich-text fields
//! (`System.Description`, `Microsoft.VSTS.Common.AcceptanceCriteria`).
//! Azure DevOps stores these as HTML; agents author markdown, so `--markdown`
//! converts on the way in. The inverse (HTML -> text) lives in `html.rs`.

use pulldown_cmark::{html, Options, Parser};

/// Convert CommonMark (plus tables, strikethrough, task lists) to HTML.
/// pulldown-cmark escapes HTML metacharacters in text, so this is safe to
/// send straight to the API.
pub fn to_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_markdown() {
        let html = to_html("## Title\n\nSome **bold** and `code`.\n\n- a\n- b\n");
        assert!(html.contains("<h2>Title</h2>"), "{html}");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<code>code</code>"), "{html}");
        assert!(html.contains("<li>a</li>"), "{html}");
    }

    #[test]
    fn fenced_code_block_preserved() {
        let html = to_html("```sql\nSELECT 1\n```");
        assert!(html.contains("<pre><code"), "{html}");
        assert!(html.contains("SELECT 1"), "{html}");
    }

    #[test]
    fn escapes_html_metacharacters_in_text() {
        let html = to_html("a < b && c > d");
        assert!(html.contains("&lt;"), "{html}");
        assert!(html.contains("&amp;"), "{html}");
        assert!(
            !html.contains("a < b"),
            "raw angle brackets must be escaped: {html}"
        );
    }

    #[test]
    fn plain_text_becomes_paragraph() {
        assert_eq!(to_html("just text"), "<p>just text</p>");
    }
}
