use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Convert CommonMark into Telegram's limited HTML, never passing model HTML through.
/// Parser closes incomplete streaming blocks; unsupported layout remains readable text.
pub fn markdown(s: &str) -> String {
    let mut out = String::new();
    let mut links = Vec::new();
    for event in Parser::new_ext(s, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Strong | Tag::Heading { .. } => out.push_str("<b>"),
                Tag::Emphasis => out.push_str("<i>"),
                Tag::Strikethrough => out.push_str("<s>"),
                Tag::CodeBlock(_) => out.push_str("<pre>"),
                Tag::Item => out.push_str("• "),
                Tag::Link { dest_url, .. } => {
                    let safe = dest_url.starts_with("https://") || dest_url.starts_with("http://");
                    links.push(safe);
                    if safe { out.push_str(&format!("<a href=\"{}\">", escape(&dest_url))); }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => out.push_str("</b>"),
                TagEnd::Heading(_) => out.push_str("</b>\n"),
                TagEnd::Emphasis => out.push_str("</i>"),
                TagEnd::Strikethrough => out.push_str("</s>"),
                TagEnd::CodeBlock => out.push_str("</pre>\n"),
                TagEnd::Paragraph | TagEnd::Item => out.push('\n'),
                TagEnd::Link => { if links.pop() == Some(true) { out.push_str("</a>"); } }
                _ => {}
            },
            Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) => out.push_str(&escape(&s)),
            Event::Code(s) => out.push_str(&format!("<code>{}</code>", escape(&s))),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("—\n"),
            _ => {}
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_is_safe_and_streaming_code_is_closed() {
        assert_eq!(markdown("**Bold** and `a < b`"), "<b>Bold</b> and <code>a &lt; b</code>");
        assert_eq!(markdown("```rust\nlet x = 1;"), "<pre>let x = 1;</pre>");
        assert!(markdown("<script>x</script>").contains("&lt;script&gt;"));
        assert!(!markdown("[run](javascript:alert)").contains("href"));
        assert_eq!(markdown("😀 _текст_"), "😀 <i>текст</i>");
    }
}
