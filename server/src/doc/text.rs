/// Computes the `textContent` of an HTML fragment produced by our own renderer and sanitizer.
///
/// Block offsets are computed against this text on the server and against the DOM in the
/// browser, so both sides must agree character for character.
pub fn html_text_content(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                let mut quote: Option<char> = None;
                for c in chars.by_ref() {
                    match quote {
                        Some(q) if c == q => quote = None,
                        Some(_) => {}
                        None if c == '"' || c == '\'' => quote = Some(c),
                        None if c == '>' => break,
                        None => {}
                    }
                }
            }
            '&' => {
                let mut entity = String::new();
                let mut terminated = false;
                while let Some(&n) = chars.peek() {
                    if n == ';' {
                        chars.next();
                        terminated = true;
                        break;
                    }
                    if !n.is_ascii_alphanumeric() && n != '#' || entity.len() > 10 {
                        break;
                    }
                    entity.push(n);
                    chars.next();
                }
                match (terminated, decode_entity(&entity)) {
                    (true, Some(ch)) => out.push(ch),
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                        if terminated {
                            out.push(';');
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn decode_entity(e: &str) -> Option<char> {
    match e {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let num = e.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse().ok()?
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_decodes() {
        assert_eq!(
            html_text_content(r#"<p>a &amp; <em title="x>y">b</em>&lt;&#x4e2d;&#25991;</p>"#),
            "a & b<中文"
        );
        assert_eq!(html_text_content("<p>AT&T &unknown; ok</p>"), "AT&T &unknown; ok");
    }
}
