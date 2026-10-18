use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inline: String,
    },
    Paragraph {
        inline: String,
    },
    Item {
        depth: usize,
        marker: String,
        inline: String,
    },
    Code {
        lang: String,
        text: String,
    },
    Quote {
        depth: usize,
        inline: String,
    },
    Rule,
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Image {
        alt: String,
    },
}

/// Splits Markdown into the blocks the reader draws. `inline` is the subset `StyledText` renders,
/// with every other ASCII punctuation escaped.
pub fn blocks(text: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut walker = Walker::default();
    for event in Parser::new_ext(text, options) {
        walker.event(event);
    }
    walker.flush();
    walker.blocks
}

#[derive(Default)]
struct Walker {
    blocks: Vec<Block>,
    inline: String,
    image: Option<String>,
    pending_image: Option<String>,
    links: Vec<String>,
    quote: usize,
    lists: Vec<Option<u64>>,
    marker: Option<String>,
    code: Option<(String, String)>,
    table: Option<Table>,
}

#[derive(Default)]
struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
}

impl Walker {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => self.push_inline(&code_span(&code)),
            Event::Html(html) => match &mut self.code {
                Some((_, buffer)) => buffer.push_str(&html),
                None => self.push_inline(&escape(&html)),
            },
            Event::InlineHtml(html) => self.push_inline(&escape(&html)),
            Event::SoftBreak => self.push_inline(" "),
            Event::HardBreak => self.push_inline("\\\n"),
            Event::Rule => {
                self.flush();
                self.blocks.push(Block::Rule);
            }
            Event::TaskListMarker(done) => self.push_inline(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(name) => self.push_inline(&escape(&format!("[{name}]"))),
            Event::InlineMath(math) | Event::DisplayMath(math) => self.push_inline(&escape(&math)),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { .. } => self.flush(),
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush();
                self.code = Some((String::new(), String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                self.marker = Some(self.next_marker());
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table::default());
            }
            Tag::Emphasis => self.push_inline("*"),
            Tag::Strong => self.push_inline("**"),
            Tag::Strikethrough => self.push_inline("~~"),
            Tag::Link { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.push_inline("[");
            }
            Tag::Image { .. } => self.image = Some(String::new()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Item => self.flush(),
            TagEnd::Heading(level) => {
                let inline = self.take_inline();
                self.blocks.push(Block::Heading {
                    level: level as u8,
                    inline,
                });
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                if let Some((lang, mut text)) = self.code.take() {
                    while text.ends_with('\n') {
                        text.pop();
                    }
                    self.blocks.push(Block::Code { lang, text });
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::TableCell => {
                let cell = self.take_inline();
                if let Some(table) = &mut self.table {
                    table.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.header = std::mem::take(&mut table.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.blocks.push(Block::Table {
                        header: table.header,
                        rows: table.rows,
                    });
                }
            }
            TagEnd::Emphasis => self.push_inline("*"),
            TagEnd::Strong => self.push_inline("**"),
            TagEnd::Strikethrough => self.push_inline("~~"),
            TagEnd::Link => {
                let url = self.links.pop().unwrap_or_default();
                let url = url.replace('>', "%3E").replace(' ', "%20");
                self.push_inline(&format!("](<{url}>)"));
            }
            TagEnd::Image => {
                if let Some(alt) = self.image.take() {
                    if self.inline.trim().is_empty() {
                        self.pending_image = Some(alt);
                    } else {
                        self.inline.push_str(&escape(&alt));
                    }
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, buffer)) = &mut self.code {
            buffer.push_str(text);
        } else if let Some(alt) = &mut self.image {
            alt.push_str(text);
        } else {
            self.push_inline(&escape(text));
        }
    }

    fn push_inline(&mut self, markup: &str) {
        if let Some(alt) = &mut self.image {
            alt.push_str(markup);
            return;
        }
        if let Some(alt) = self.pending_image.take() {
            self.inline.push_str(&escape(&alt));
        }
        self.inline.push_str(markup);
    }

    fn take_inline(&mut self) -> String {
        if let Some(alt) = self.pending_image.take() {
            self.inline.push_str(&escape(&alt));
        }
        std::mem::take(&mut self.inline).trim().to_string()
    }

    fn next_marker(&mut self) -> String {
        match self.lists.last_mut() {
            Some(Some(next)) => {
                let marker = format!("{next}.");
                *next += 1;
                marker
            }
            _ => "•".to_string(),
        }
    }

    fn flush(&mut self) {
        if self.inline.trim().is_empty()
            && self.marker.is_none()
            && self.quote == 0
            && self.lists.is_empty()
            && let Some(alt) = self.pending_image.take()
        {
            self.inline.clear();
            self.blocks.push(Block::Image { alt });
            return;
        }
        let inline = self.take_inline();
        let depth = self.lists.len();
        if let Some(marker) = self.marker.take() {
            self.blocks.push(Block::Item {
                depth,
                marker,
                inline,
            });
            return;
        }
        if inline.is_empty() {
            return;
        }
        let block = if depth > 0 {
            Block::Item {
                depth,
                marker: String::new(),
                inline,
            }
        } else if self.quote > 0 {
            Block::Quote {
                depth: self.quote,
                inline,
            }
        } else {
            Block::Paragraph { inline }
        };
        self.blocks.push(block);
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn code_span(code: &str) -> String {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(inline: &str) -> Block {
        Block::Paragraph {
            inline: inline.into(),
        }
    }

    fn item(depth: usize, marker: &str, inline: &str) -> Block {
        Block::Item {
            depth,
            marker: marker.into(),
            inline: inline.into(),
        }
    }

    #[test]
    fn headings_keep_their_level() {
        assert_eq!(
            blocks("# Título\n\n### Três\n"),
            [
                Block::Heading {
                    level: 1,
                    inline: "Título".into()
                },
                Block::Heading {
                    level: 3,
                    inline: "Três".into()
                },
            ]
        );
    }

    #[test]
    fn inline_styles_become_styled_text_markdown() {
        assert_eq!(
            blocks("a **b** *c* `d` ~~e~~ [f](http://x.y)\n"),
            [paragraph("a **b** *c* `d` ~~e~~ [f](<http://x.y>)")]
        );
    }

    #[test]
    fn plain_punctuation_is_escaped() {
        assert_eq!(
            blocks("1 * 2 = [x] <u>\n"),
            [paragraph(r"1 \* 2 \= \[x\] \<u\>")]
        );
    }

    #[test]
    fn code_spans_with_backticks_get_longer_fences() {
        assert_eq!(blocks("``a`b``\n"), [paragraph("``a`b``")]);
    }

    #[test]
    fn soft_breaks_join_lines() {
        assert_eq!(blocks("a\nb\n"), [paragraph("a b")]);
    }

    #[test]
    fn lists_nest_and_number() {
        assert_eq!(
            blocks("- a\n- b\n  1. c\n  2. d\n"),
            [
                item(1, "•", "a"),
                item(1, "•", "b"),
                item(2, "1.", "c"),
                item(2, "2.", "d"),
            ]
        );
    }

    #[test]
    fn loose_items_continue_without_a_marker() {
        assert_eq!(
            blocks("- a\n\n  more\n"),
            [item(1, "•", "a"), item(1, "", "more")]
        );
    }

    #[test]
    fn task_items_show_a_box() {
        assert_eq!(
            blocks("- [x] feito\n- [ ] falta\n"),
            [item(1, "•", "☑ feito"), item(1, "•", "☐ falta"),]
        );
    }

    #[test]
    fn quotes_keep_their_depth() {
        assert_eq!(
            blocks("> a\n>> b\n"),
            [
                Block::Quote {
                    depth: 1,
                    inline: "a".into()
                },
                Block::Quote {
                    depth: 2,
                    inline: "b".into()
                },
            ]
        );
    }

    #[test]
    fn code_blocks_keep_language_and_text() {
        assert_eq!(
            blocks("```rust\nfn x() {}\n```\n"),
            [Block::Code {
                lang: "rust".into(),
                text: "fn x() {}".into(),
            }]
        );
    }

    #[test]
    fn raw_html_is_shown_as_code() {
        assert_eq!(
            blocks("<div>\nx\n</div>\n"),
            [Block::Code {
                lang: String::new(),
                text: "<div>\nx\n</div>".into(),
            }]
        );
    }

    #[test]
    fn rules_and_tables() {
        assert_eq!(
            blocks("a\n\n---\n\n| h1 | h2 |\n|---|---|\n| a | b |\n"),
            [
                paragraph("a"),
                Block::Rule,
                Block::Table {
                    header: vec!["h1".into(), "h2".into()],
                    rows: vec![vec!["a".into(), "b".into()]],
                },
            ]
        );
    }

    #[test]
    fn a_lone_image_is_its_own_block() {
        assert_eq!(
            blocks("![logo](x.png)\n"),
            [Block::Image { alt: "logo".into() }]
        );
        assert_eq!(
            blocks("ver ![logo](x.png) aqui\n"),
            [paragraph("ver logo aqui")]
        );
    }
}
