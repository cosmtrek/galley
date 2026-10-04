use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd, html};

use super::{Block, BlockKind, Doc, Frontmatter, ListInfo, html_text_content};

static SANITIZER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let mut b = ammonia::Builder::default();
    b.add_generic_attributes(["data-cell"]);
    b.add_tag_attributes("ol", ["start"]);
    b
});

/// Maps the name in an `assets/<name>` reference to the URL it is served from.
pub type AssetUrl<'a> = &'a dyn Fn(&str) -> String;

pub fn rewrite_asset_url(url: &str, asset_url: AssetUrl) -> String {
    match url.strip_prefix("assets/").or_else(|| url.strip_prefix("./assets/")) {
        Some(rest) => asset_url(rest),
        None => url.to_string(),
    }
}

/// Parses a report. Block ids are left empty; the caller assigns them via alignment.
pub fn parse(markdown: &str, asset_url: AssetUrl) -> Doc {
    let (fm, body_start) = split_frontmatter(markdown);
    let body = &markdown[body_start..];
    let lines = LineIndex::new(markdown);

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let events: Vec<(Event, Range<usize>)> = Parser::new_ext(body, opts).into_offset_iter().collect();

    let mut blocks = Vec::new();
    let mut list_group = 0u32;
    let mut i = 0;
    while i < events.len() {
        let (ev, range) = &events[i];
        match ev {
            Event::Start(Tag::List(start)) => {
                list_group += 1;
                let info = ListInfo { group: list_group, ordered: start.is_some(), start: *start };
                let end = matching_end(&events, i);
                let mut j = i + 1;
                while j < end {
                    if let Event::Start(Tag::Item) = events[j].0 {
                        let item_end = matching_end(&events, j);
                        let r = events[j].1.clone();
                        let mut b = build_block(&events[j..=item_end], body, asset_url);
                        b.src_lines = lines.span(r.start + body_start, r.end + body_start);
                        b.list = Some(info.clone());
                        blocks.push(b);
                        j = item_end + 1;
                    } else {
                        j += 1;
                    }
                }
                i = end + 1;
            }
            Event::Start(_) => {
                let end = matching_end(&events, i);
                let mut b = build_block(&events[i..=end], body, asset_url);
                b.src_lines = lines.span(range.start + body_start, range.end + body_start);
                blocks.push(b);
                i = end + 1;
            }
            Event::Rule => {
                blocks.push(Block {
                    id: String::new(),
                    kind: BlockKind::Rule,
                    section_id: String::new(),
                    level: None,
                    text: String::new(),
                    cells: None,
                    src_lines: lines.span(range.start + body_start, range.end + body_start),
                    list: None,
                    html: "<hr>".into(),
                    sig: format!("hr{i}"),
                });
                i += 1;
            }
            _ => i += 1,
        }
    }

    let title = fm
        .title
        .clone()
        .or_else(|| {
            blocks
                .iter()
                .find(|b| b.kind == BlockKind::Heading && b.level == Some(1))
                .map(|b| b.text.clone())
        })
        .unwrap_or_else(|| "未命名报告".to_string());
    Doc { title, summary: fm.summary.unwrap_or_default(), blocks }
}

fn matching_end(events: &[(Event, Range<usize>)], start: usize) -> usize {
    let mut depth = 0i32;
    for (k, (ev, _)) in events.iter().enumerate().skip(start) {
        match ev {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    events.len() - 1
}

/// Raw HTML is never rendered; it is shown as literal text.
fn neutralize<'a>(ev: &Event<'a>, asset_url: AssetUrl) -> Event<'a> {
    match ev {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s.clone()),
        Event::Start(Tag::Image { link_type, dest_url, title, id }) => Event::Start(Tag::Image {
            link_type: *link_type,
            dest_url: CowStr::from(rewrite_asset_url(dest_url, asset_url)),
            title: title.clone(),
            id: id.clone(),
        }),
        other => other.clone(),
    }
}

fn render_events(events: &[(Event, Range<usize>)], asset_url: AssetUrl) -> String {
    let mut out = String::new();
    html::push_html(&mut out, events.iter().map(|(e, _)| neutralize(e, asset_url)));
    out
}

fn sanitize(html: &str) -> String {
    SANITIZER.clean(html).to_string().trim_end().to_string()
}

fn build_block(events: &[(Event, Range<usize>)], body: &str, asset_url: AssetUrl) -> Block {
    let mut kind = BlockKind::Paragraph;
    let mut level = None;
    let mut sig = String::new();
    let mut cells = None;

    let html = match &events[0].0 {
        Event::Start(Tag::Heading { level: l, .. }) => {
            kind = BlockKind::Heading;
            level = Some(*l as u8);
            sanitize(&render_events(events, asset_url))
        }
        Event::Start(Tag::Item) => {
            kind = BlockKind::ListItem;
            sanitize(&render_events(events, asset_url))
        }
        Event::Start(Tag::BlockQuote(_)) => {
            kind = BlockKind::Quote;
            sanitize(&render_events(events, asset_url))
        }
        Event::Start(Tag::CodeBlock(cb)) => {
            let lang = match cb {
                CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_ascii_lowercase(),
                CodeBlockKind::Indented => String::new(),
            };
            let source: String = events
                .iter()
                .filter_map(|(e, _)| match e {
                    Event::Text(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect();
            let figure = match lang.as_str() {
                "mermaid" => render_mermaid(&source).map(|svg| diagram_html(&svg, "流程图")),
                "svg" => svg_source(&source).map(|svg| diagram_html(&svg, "插图")),
                _ => None,
            };
            match figure {
                Some(h) => {
                    kind = BlockKind::Diagram;
                    sig = format!("{lang}:{source}");
                    h
                }
                None => {
                    kind = if lang == "chart" { BlockKind::Chart } else { BlockKind::Code };
                    sanitize(&render_events(events, asset_url))
                }
            }
        }
        Event::Start(Tag::Table(_)) => {
            kind = BlockKind::Table;
            let (h, c) = render_table(events, asset_url);
            cells = Some(c);
            sanitize(&h)
        }
        Event::Start(Tag::HtmlBlock) => {
            let raw: String = events
                .iter()
                .filter_map(|(e, _)| match e {
                    Event::Html(s) | Event::Text(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect();
            match svg_source(&raw) {
                Some(svg) => {
                    kind = BlockKind::Diagram;
                    sig = format!("svg:{}", raw.trim());
                    diagram_html(&svg, "插图")
                }
                None => sanitize(&format!("<p>{}</p>", escape_html(raw.trim_end()))),
            }
        }
        Event::Start(Tag::Paragraph)
            if matches!(events.get(1), Some((Event::InlineHtml(h), _)) if h.trim_start().to_ascii_lowercase().starts_with("<svg")) =>
        {
            // A one-line `<svg>…</svg>` is not an HTML block in CommonMark, only inline HTML.
            let raw = &body[events[0].1.clone()];
            match svg_source(raw) {
                Some(svg) => {
                    kind = BlockKind::Diagram;
                    sig = format!("svg:{}", raw.trim());
                    diagram_html(&svg, "插图")
                }
                None => sanitize(&render_events(events, asset_url)),
            }
        }
        Event::Start(Tag::Paragraph) => {
            let inner = &events[1..events.len() - 1];
            let only_image = matches!(inner.first(), Some((Event::Start(Tag::Image { .. }), _)))
                && matches!(inner.last(), Some((Event::End(TagEnd::Image), _)))
                && matching_end(inner, 0) == inner.len() - 1;
            if only_image {
                kind = BlockKind::Image;
                if let Event::Start(Tag::Image { dest_url, .. }) = &inner[0].0 {
                    sig = dest_url.to_string();
                }
            }
            sanitize(&render_events(events, asset_url))
        }
        _ => sanitize(&render_events(events, asset_url)),
    };

    let text = html_text_content(&html);
    Block {
        id: String::new(),
        kind,
        section_id: String::new(),
        level,
        text,
        cells,
        src_lines: (0, 0),
        list: None,
        html,
        sig,
    }
}

/// Tables are rendered by hand so cells carry `data-cell="row,col"` and no whitespace text
/// nodes appear between cells (keeping block text equal to the concatenated cell texts).
fn render_table(events: &[(Event, Range<usize>)], asset_url: AssetUrl) -> (String, Vec<Vec<String>>) {
    let mut out = String::from("<table>");
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_head = false;
    let mut row: Vec<String> = Vec::new();
    let mut i = 1;
    while i < events.len() - 1 {
        match &events[i].0 {
            Event::Start(Tag::TableHead) => {
                in_head = true;
                out.push_str("<thead><tr>");
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                out.push_str("</tr></thead><tbody>");
                rows.push(std::mem::take(&mut row));
            }
            Event::Start(Tag::TableRow) => out.push_str("<tr>"),
            Event::End(TagEnd::TableRow) => {
                out.push_str("</tr>");
                rows.push(std::mem::take(&mut row));
            }
            Event::Start(Tag::TableCell) => {
                let end = matching_end(events, i);
                let inner = render_events(&events[i + 1..end], asset_url);
                let tag = if in_head { "th" } else { "td" };
                let (r, c) = (rows.len(), row.len());
                out.push_str(&format!("<{tag} data-cell=\"{r},{c}\">{inner}</{tag}>"));
                row.push(html_text_content(&sanitize(&inner)));
                i = end;
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str("</tbody></table>");
    (out, rows)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const MAX_DIAGRAM_SOURCE: usize = 256 * 1024;

fn render_mermaid(source: &str) -> Option<String> {
    if source.trim().is_empty() || source.len() > MAX_DIAGRAM_SOURCE {
        return None;
    }
    // The renderer is third-party layout code; a panic on odd input must not fail the import.
    std::panic::catch_unwind(|| {
        mermaid_rs_renderer::render_with_options(source, mermaid_rs_renderer::RenderOptions::default()).ok()
    })
    .ok()
    .flatten()
}

/// Accepts a standalone SVG document and makes sure it declares the SVG namespace, which
/// browsers require before they decode it as an image.
fn svg_source(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.len() > MAX_DIAGRAM_SOURCE {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let open = lower.find("<svg")?;
    let head = lower[..open].trim();
    let prolog_only = head.is_empty() || (head.starts_with("<?xml") || head.starts_with("<!")) && head.ends_with('>');
    if !prolog_only || !lower.ends_with("</svg>") {
        return None;
    }
    let tag_end = open + s[open..].find('>')?;
    if s[open..tag_end].contains("xmlns=") {
        return Some(s.to_string());
    }
    let at = open + 4;
    Some(format!("{} xmlns=\"http://www.w3.org/2000/svg\"{}", &s[..at], &s[at..]))
}

/// SVG is shown through `<img>` rather than inlined: images never run scripts, load external
/// resources, or leak `<style>` into the page, so untrusted SVG needs no sanitizing, and the
/// block has no text nodes that could shift anchor offsets.
fn diagram_html(svg: &str, alt: &str) -> String {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    format!("<figure class=\"diagram\"><img src=\"data:image/svg+xml;base64,{data}\" alt=\"{alt}\"></figure>")
}

fn split_frontmatter(md: &str) -> (Frontmatter, usize) {
    let mut fm = Frontmatter::default();
    let Some(rest) = md.strip_prefix("---\n").or_else(|| md.strip_prefix("---\r\n")) else {
        return (fm, 0);
    };
    let open_len = md.len() - rest.len();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let l = line.trim_end();
        if l == "---" {
            return (fm, open_len + offset);
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "title" => fm.title = Some(v),
                "summary" => fm.summary = Some(v),
                _ => {}
            }
        }
    }
    (Frontmatter::default(), 0)
}

struct LineIndex {
    starts: Vec<usize>,
    src: Vec<u8>,
}

impl LineIndex {
    fn new(s: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in s.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts, src: s.as_bytes().to_vec() }
    }

    fn line_of(&self, byte: usize) -> u32 {
        (self.starts.partition_point(|&s| s <= byte)) as u32
    }

    fn span(&self, start: usize, end: usize) -> (u32, u32) {
        let mut e = end.min(self.src.len());
        while e > start + 1 && matches!(self.src[e - 1], b'\n' | b'\r') {
            e -= 1;
        }
        (self.line_of(start), self.line_of(e.saturating_sub(1).max(start)))
    }
}
