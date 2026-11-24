use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use registry::{DocAsset, MAX_PREVIEW, MAX_README};

const MAX_BLOCKS: usize = 2000;
const MAX_TEXT: usize = 16_384;
const MAX_NESTING: usize = 16;
const CACHE_HEADER: usize = 64;
const CACHE_BYTES: usize = MAX_README + MAX_PREVIEW + CACHE_HEADER;
static DETAILS: Mutex<()> = Mutex::new(());

#[derive(Debug, PartialEq, Serialize)]
struct Span {
    text: String,
    strong: bool,
    em: bool,
    code: bool,
    href: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        spans: Vec<Span>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Code {
        text: String,
    },
    Quote {
        blocks: Vec<Block>,
    },
    Rule,
    Table {
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Problem {
    Offline,
    Invalid,
    Unavailable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum Origin {
    Cache,
    Network,
}

#[derive(Default, Serialize)]
pub(crate) struct AddonDetails {
    readme: Option<Vec<Block>>,
    preview: Option<String>,
    origin: Option<Origin>,
    reference: Option<String>,
    verified: bool,
    problem: Option<Problem>,
}

fn release_tag<'a>(addon: &Addon, url: &'a str) -> Option<&'a str> {
    if !registry::valid_url(url, addon.repository) {
        return None;
    }
    url.strip_prefix(&format!(
        "https://github.com{}/releases/download/",
        addon.repository
    ))?
    .split_once('/')
    .map(|(tag, _)| tag)
}

#[derive(Clone, Copy)]
struct Source<'a> {
    repository: &'a str,
    reference: &'a str,
    readme: Option<&'a DocAsset>,
    preview: Option<&'a DocAsset>,
}

impl Source<'_> {
    fn url(&self, path: &str) -> String {
        format!(
            "https://raw.githubusercontent.com{}/{}/{path}",
            self.repository, self.reference
        )
    }

    fn cache_header(&self) -> String {
        let mut hash = Sha256::new();
        hash.update([u8::from(self.readme.is_some())]);
        hash.update(self.reference.as_bytes());
        hex(&hash.finalize())
    }

    fn decode(&self, bytes: &[u8], origin: Origin) -> Result<AddonDetails, Problem> {
        let header = self.cache_header();
        let content = bytes
            .strip_prefix(header.as_bytes())
            .ok_or(Problem::Invalid)?;
        let (readme, preview) = match self.readme {
            Some(asset) => content
                .split_at_checked(asset.size)
                .ok_or(Problem::Invalid)?,
            None => (content, &[][..]),
        };
        validate_asset(readme, self.readme, MAX_README)?;
        let preview = match self.preview {
            Some(asset) => {
                validate_asset(preview, Some(asset), MAX_PREVIEW)?;
                let mime = image_mime(preview).ok_or(Problem::Invalid)?;
                Some(format!("data:{mime};base64,{}", STANDARD.encode(preview)))
            }
            None if preview.is_empty() => None,
            None => return Err(Problem::Invalid),
        };
        let markdown = std::str::from_utf8(readme).map_err(|_| Problem::Invalid)?;
        Ok(AddonDetails {
            readme: Some(markdown_blocks(markdown)),
            preview,
            origin: Some(origin),
            reference: Some(
                if self.readme.is_some() {
                    &self.reference[..12]
                } else {
                    self.reference
                }
                .into(),
            ),
            verified: self.readme.is_some(),
            problem: None,
        })
    }
}

fn validate_asset(bytes: &[u8], asset: Option<&DocAsset>, limit: usize) -> Result<(), Problem> {
    if bytes.len() > limit
        || asset.is_some_and(|asset| {
            bytes.len() != asset.size
                || !hex(&Sha256::digest(bytes)).eq_ignore_ascii_case(&asset.sha256)
        })
    {
        return Err(Problem::Invalid);
    }
    Ok(())
}

fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, Problem> {
    update::fetch(url, limit as u64, |url, redirect| {
        !redirect && https_parts(url).is_some_and(|(host, _)| host == "raw.githubusercontent.com")
    })
    .map_err(|problem| match problem {
        registry::Problem::Offline => Problem::Offline,
        registry::Problem::Status => Problem::Unavailable,
        _ => Problem::Invalid,
    })
}

fn cached_details(
    root: &Path,
    source: Source<'_>,
    fetch: impl Fn(&str, usize) -> Result<Vec<u8>, Problem>,
) -> Result<AddonDetails, Problem> {
    // Git tags can contain characters Windows reserves in directory names.
    let key = if source.reference.len() <= 120 && safe_components(source.reference).is_ok() {
        source.reference.to_owned()
    } else {
        format!("#{}", source.cache_header())
    };
    let dir = root.join(&key);
    let file = dir.join("assets");
    if file.exists() {
        let result = read_bounded(&file, CACHE_BYTES)
            .map_err(|_| Problem::Invalid)
            .and_then(|bytes| source.decode(&bytes, Origin::Cache));
        if result.is_err() {
            let _ = fs::remove_file(&file);
        } else {
            prune(root, &key);
        }
        return result;
    }

    let readme = fetch(
        &source.url(source.readme.map_or("README.md", |asset| &asset.path)),
        source.readme.map_or(MAX_README, |asset| asset.size),
    )?;
    validate_asset(&readme, source.readme, MAX_README)?;
    let mut bytes = source.cache_header().into_bytes();
    bytes.extend(readme);
    if let Some(asset) = source.preview {
        let preview = fetch(&source.url(&asset.path), asset.size)?;
        validate_asset(&preview, Some(asset), MAX_PREVIEW)?;
        bytes.extend(preview);
    }
    let details = source.decode(&bytes, Origin::Network)?;
    // One atomic file publishes both assets together, after all checks.
    if let Err(error) = crate::store::write_atomic(&file, &bytes) {
        eprintln!("Could not cache addon details: {error}");
    } else {
        prune(root, &key);
    }
    Ok(details)
}

fn prune(root: &Path, keep: &str) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(keep)
                && fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.is_dir() && !is_package_link(&metadata))
                && let Err(error) = fs::remove_dir_all(path)
            {
                eprintln!("Could not prune addon details: {error}");
            }
        }
    }
}

fn details(app: &tauri::AppHandle, id: &str) -> Result<AddonDetails, Problem> {
    let addon = CATALOG
        .iter()
        .copied()
        .find(|addon| addon.id == id)
        .ok_or(Problem::Unavailable)?;
    let _guard = DETAILS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let registry = registry::current(app).map_err(|_| Problem::Unavailable)?;
    let source = if let Some(docs) = registry.docs(addon) {
        Source {
            repository: addon.repository,
            reference: &docs.commit,
            readme: Some(&docs.readme),
            preview: docs.preview.as_ref(),
        }
    } else {
        let release = registry
            .details_release(addon)
            .ok_or(Problem::Unavailable)?;
        Source {
            repository: addon.repository,
            reference: release_tag(addon, &release.url).ok_or(Problem::Unavailable)?,
            readme: None,
            preview: None,
        }
    };
    let root = app
        .path()
        .app_local_data_dir()
        .map_err(|_| Problem::Unavailable)?;
    cached_details(&root.join("addon-details").join(id), source, fetch)
}

#[tauri::command]
pub(crate) async fn addon_details(app: tauri::AppHandle, id: String) -> AddonDetails {
    tauri::async_runtime::spawn_blocking(move || details(&app, &id))
        .await
        .unwrap_or(Err(Problem::Unavailable))
        .unwrap_or_else(|problem| AddonDetails {
            problem: Some(problem),
            ..Default::default()
        })
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn markdown_blocks(markdown: &str) -> Vec<Block> {
    let mut events = Parser::new_ext(markdown, Options::ENABLE_TABLES).peekable();
    while matches!(events.peek(), Some(Event::Start(Tag::HtmlBlock))) {
        events.next();
        skip(&mut events, TagEnd::HtmlBlock);
    }
    if matches!(
        events.peek(),
        Some(Event::Start(Tag::Heading {
            level: HeadingLevel::H1,
            ..
        }))
    ) {
        events.next();
        skip(&mut events, TagEnd::Heading(HeadingLevel::H1));
    }
    Markdown {
        events,
        remaining: MAX_BLOCKS,
    }
    .blocks(None, 0, 0)
}

fn skip<'a>(events: &mut impl Iterator<Item = Event<'a>>, end: TagEnd) {
    let mut depth = 0;
    for event in events {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(found) if depth == 0 && found == end => break,
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
}

struct Markdown<I: Iterator> {
    events: std::iter::Peekable<I>,
    remaining: usize,
}

impl<'a, I: Iterator<Item = Event<'a>>> Markdown<I> {
    fn blocks(&mut self, end: Option<TagEnd>, nesting: usize, lists: usize) -> Vec<Block> {
        let mut blocks = Vec::new();
        while self.remaining > 0 {
            let Some(event) = self.events.next() else {
                break;
            };
            if matches!(&event, Event::End(found) if Some(*found) == end) {
                break;
            }
            self.remaining -= 1;
            let block = match event {
                Event::Start(Tag::Paragraph) => {
                    let spans = self.spans(TagEnd::Paragraph);
                    if spans.is_empty() {
                        continue;
                    }
                    Block::Paragraph { spans }
                }
                Event::Start(Tag::Heading { level, .. }) => Block::Heading {
                    level: if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                        2
                    } else {
                        3
                    },
                    spans: self.spans(TagEnd::Heading(level)),
                },
                Event::Start(Tag::CodeBlock(_)) => {
                    let mut text = String::new();
                    for event in self.events.by_ref() {
                        match event {
                            Event::Text(value) => append(&mut text, &value, MAX_TEXT),
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    Block::Code { text }
                }
                Event::Start(Tag::BlockQuote(kind)) if nesting < MAX_NESTING => Block::Quote {
                    blocks: self.blocks(Some(TagEnd::BlockQuote(kind)), nesting + 1, lists),
                },
                Event::Start(Tag::List(start)) if nesting < MAX_NESTING && lists < 3 => {
                    let mut items = Vec::new();
                    while self.remaining > 0
                        && matches!(self.events.next(), Some(Event::Start(Tag::Item)))
                    {
                        self.remaining -= 1;
                        items.push(self.blocks(Some(TagEnd::Item), nesting + 1, lists + 1));
                    }
                    Block::List {
                        ordered: start.is_some(),
                        start,
                        items,
                    }
                }
                Event::Start(Tag::Table(_)) => self.table(),
                Event::Rule => Block::Rule,
                Event::Start(Tag::HtmlBlock) => {
                    skip(&mut self.events, TagEnd::HtmlBlock);
                    continue;
                }
                Event::Start(tag @ (Tag::BlockQuote(_) | Tag::List(_))) => {
                    skip(&mut self.events, tag.to_end());
                    continue;
                }
                event => {
                    // Tight list items emit inline events without paragraph wrappers.
                    let spans = self.inline(Some(event), None);
                    if spans.is_empty() {
                        continue;
                    }
                    Block::Paragraph { spans }
                }
            };
            blocks.push(block);
        }
        blocks
    }

    fn table(&mut self) -> Block {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        while self.remaining > 0 {
            let header = match self.events.next() {
                Some(Event::Start(Tag::TableHead)) => true,
                Some(Event::Start(Tag::TableRow)) => false,
                _ => break,
            };
            self.remaining -= 1;
            let mut cells = Vec::new();
            while self.remaining > 0
                && matches!(self.events.next(), Some(Event::Start(Tag::TableCell)))
            {
                self.remaining -= 1;
                cells.push(self.spans(TagEnd::TableCell));
            }
            if header {
                head = cells;
            } else {
                rows.push(cells);
            }
        }
        Block::Table { head, rows }
    }

    fn spans(&mut self, end: TagEnd) -> Vec<Span> {
        self.inline(None, Some(end))
    }

    fn inline(&mut self, mut first: Option<Event<'a>>, end: Option<TagEnd>) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        let mut strong = 0;
        let mut em = 0;
        let mut image = 0;
        let mut href = None;
        let mut used = 0;
        loop {
            if first.is_none()
                && end.is_none()
                && !matches!(
                    self.events.peek(),
                    Some(
                        Event::Text(_)
                            | Event::Code(_)
                            | Event::InlineHtml(_)
                            | Event::SoftBreak
                            | Event::HardBreak
                            | Event::Start(
                                Tag::Strong | Tag::Emphasis | Tag::Link { .. } | Tag::Image { .. }
                            )
                            | Event::End(
                                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link | TagEnd::Image
                            )
                    )
                )
            {
                break;
            }
            let Some(event) = first.take().or_else(|| self.events.next()) else {
                break;
            };
            let (text, code) = match event {
                Event::End(found) if Some(found) == end => break,
                Event::Start(Tag::Strong) => {
                    strong += 1;
                    continue;
                }
                Event::End(TagEnd::Strong) => {
                    strong -= 1;
                    continue;
                }
                Event::Start(Tag::Emphasis) => {
                    em += 1;
                    continue;
                }
                Event::End(TagEnd::Emphasis) => {
                    em -= 1;
                    continue;
                }
                Event::Start(Tag::Image { .. }) => {
                    image += 1;
                    continue;
                }
                Event::End(TagEnd::Image) => {
                    image -= 1;
                    continue;
                }
                Event::Start(Tag::Link { dest_url, .. }) if image == 0 => {
                    href = (dest_url.len() <= 2048
                        && dest_url.starts_with("https://")
                        && !dest_url
                            .chars()
                            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
                        && tauri::Url::parse(&dest_url).is_ok())
                    .then(|| dest_url.into_string());
                    continue;
                }
                Event::End(TagEnd::Link) if image == 0 => {
                    href = None;
                    continue;
                }
                Event::Text(value) => (value, false),
                Event::Code(value) => (value, true),
                Event::SoftBreak | Event::HardBreak => ("\n".into(), false),
                _ => continue,
            };
            let mut truncated = String::new();
            append(&mut truncated, &text, MAX_TEXT - used);
            if truncated.is_empty() {
                continue;
            }
            used += truncated.len();
            let span = Span {
                text: truncated,
                strong: image == 0 && strong > 0,
                em: image == 0 && em > 0,
                code: image == 0 && code,
                href: if image == 0 { href.clone() } else { None },
            };
            if let Some(previous) = spans.last_mut().filter(|previous| {
                previous.strong == span.strong
                    && previous.em == span.em
                    && previous.code == span.code
                    && previous.href == span.href
            }) {
                previous.text.push_str(&span.text);
            } else {
                spans.push(span);
            }
        }
        spans
    }
}

fn append(text: &mut String, source: &str, limit: usize) {
    text.push_str(
        &source[..source.floor_char_boundary(limit.saturating_sub(text.len()).min(source.len()))],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_keeps_only_supported_blocks_and_safe_inline_content() {
        let blocks = markdown_blocks(concat!(
            "<script>hidden</script>\n\n# Title\n\n## Intro\n\n",
            "**bold** *em* `code` <i>text</i> ![*alt*](https://example.com/image.png) ",
            "[safe](https://example.com) [other](http://example.com) [local](/file) ",
            "[script](javascript:alert%281%29)\n\n",
            "##### Small\n\n3. item\n\n> Quote\n\n---\n\n",
            "| Header |\n| --- |\n| Cell |\n\n",
            "```text\nliteral <b>\n```\n"
        ));
        let value = serde_json::to_value(blocks).unwrap();
        let blocks = value.as_array().unwrap();
        assert_eq!(
            blocks
                .iter()
                .map(|block| block["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "heading",
                "paragraph",
                "heading",
                "list",
                "quote",
                "rule",
                "table",
                "code"
            ]
        );
        assert_eq!(blocks[0]["level"], 2);
        let spans = blocks[1]["spans"].as_array().unwrap();
        assert_eq!(spans[0]["strong"], true);
        assert!(
            spans
                .iter()
                .any(|span| span["text"] == "em" && span["em"] == true)
        );
        assert!(
            spans
                .iter()
                .any(|span| span["text"] == "code" && span["code"] == true)
        );
        let link = spans
            .iter()
            .filter_map(|span| span["href"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(link, ["https://example.com"]);
        let text = spans
            .iter()
            .map(|span| span["text"].as_str().unwrap())
            .collect::<String>();
        assert_eq!(text, "bold em code text alt safe other local script");
        assert_eq!(blocks[2]["level"], 3);
        assert_eq!(blocks[3]["start"], 3);
        assert_eq!(blocks[3]["items"][0][0]["spans"][0]["text"], "item");
        assert_eq!(blocks[4]["blocks"][0]["kind"], "paragraph");
        assert_eq!(blocks[6]["head"][0][0]["text"], "Header");
        assert_eq!(blocks[6]["rows"][0][0][0]["text"], "Cell");
        assert_eq!(blocks[7]["text"], "literal <b>\n");
    }

    #[test]
    fn markdown_bounds_blocks_text_and_nested_lists() {
        assert_eq!(
            markdown_blocks(&"one\n\n".repeat(MAX_BLOCKS + 1)).len(),
            MAX_BLOCKS
        );
        let value = serde_json::to_value(markdown_blocks(&"é".repeat(MAX_TEXT))).unwrap();
        assert_eq!(
            value[0]["spans"][0]["text"].as_str().unwrap().len(),
            MAX_TEXT
        );
        let value =
            serde_json::to_value(markdown_blocks("- a\n  - b\n    - c\n      - d\n")).unwrap();
        let nested = &value[0]["items"][0][1]["items"][0][1]["items"][0];
        assert_eq!(nested[0]["spans"][0]["text"], "c");
        assert_eq!(nested.as_array().unwrap().len(), 1);
    }

    #[test]
    fn tag_is_taken_from_the_catalog_releases_url() {
        let addon = &catalog::MATH;
        let base = format!("https://github.com{}/releases/download/", addon.repository);
        assert_eq!(
            release_tag(addon, &format!("{base}v0.1.0/math.zip")),
            Some("v0.1.0")
        );
        for suffix in [
            "../math.zip",
            "v%2f0/math.zip",
            "v0/other/math.zip",
            "v0/math.zip?x",
        ] {
            assert!(release_tag(addon, &format!("{base}{suffix}")).is_none());
        }
        assert!(
            release_tag(
                addon,
                "https://github.com/elsewhere/demo/releases/download/v0/demo.zip"
            )
            .is_none()
        );
    }

    #[test]
    fn preview_uses_image_magic_only() {
        assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        assert_eq!(image_mime(b"\xff\xd8\xff"), Some("image/jpeg"));
        assert_eq!(image_mime(b"RIFF0000WEBP"), Some("image/webp"));
        assert_eq!(image_mime(b"RIFF0000WAVE"), None);
        assert_eq!(image_mime(b"<svg>"), None);
    }

    #[test]
    fn pinned_cache_verifies_both_assets_before_publishing_and_works_offline() {
        let home = tempfile::tempdir().unwrap();
        let readme = b"# Title\n\nHello";
        let preview = b"\x89PNG\r\n\x1a\n";
        let readme_asset = DocAsset {
            path: "ADDON.md".into(),
            size: readme.len(),
            sha256: hex(&Sha256::digest(readme)),
        };
        let preview_asset = DocAsset {
            path: "docs/preview.png".into(),
            size: preview.len(),
            sha256: hex(&Sha256::digest(preview)),
        };
        let source = Source {
            repository: "/test/addon",
            reference: "0123456789012345678901234567890123456789",
            readme: Some(&readme_asset),
            preview: Some(&preview_asset),
        };
        let current = home.path().join(source.reference);
        for bad in [b"wrong".as_slice(), b"12345678".as_slice()] {
            assert!(matches!(
                cached_details(home.path(), source, |url, _| {
                    Ok(if url.ends_with("ADDON.md") {
                        readme.to_vec()
                    } else {
                        bad.to_vec()
                    })
                }),
                Err(Problem::Invalid)
            ));
            assert!(!current.exists());
        }
        let old = home.path().join("v0");
        fs::create_dir(&old).unwrap();
        let details = cached_details(home.path(), source, |url, limit| {
            if url.ends_with("ADDON.md") {
                assert_eq!(limit, readme.len());
                Ok(readme.to_vec())
            } else {
                assert!(url.ends_with("docs/preview.png"));
                assert_eq!(limit, preview.len());
                Ok(preview.to_vec())
            }
        })
        .unwrap();
        assert!(matches!(details.origin, Some(Origin::Network)));
        assert!(details.verified);
        assert_eq!(details.reference.as_deref(), Some("012345678901"));
        assert!(
            details
                .preview
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(!old.exists());
        let details = cached_details(home.path(), source, |_, _| {
            panic!("cache hit used the network")
        })
        .unwrap();
        assert!(matches!(details.origin, Some(Origin::Cache)));
        fs::write(current.join("assets"), b"corrupt").unwrap();
        assert!(matches!(
            cached_details(home.path(), source, |_, _| panic!(
                "cache hit used the network"
            )),
            Err(Problem::Invalid)
        ));
        assert!(!current.join("assets").exists());
    }

    #[test]
    fn fallback_is_bounded_unverified_and_uses_tag_cache() {
        let home = tempfile::tempdir().unwrap();
        let source = Source {
            repository: "/test/addon",
            reference: "v1",
            readme: None,
            preview: None,
        };
        assert!(matches!(
            cached_details(home.path(), source, |_, _| Err(Problem::Offline)),
            Err(Problem::Offline)
        ));
        assert!(matches!(
            cached_details(home.path(), source, |_, _| Ok(vec![b'x'; MAX_README + 1])),
            Err(Problem::Invalid)
        ));
        assert!(!home.path().join("v1").exists());
        let details = cached_details(home.path(), source, |url, limit| {
            assert_eq!(
                url,
                "https://raw.githubusercontent.com/test/addon/v1/README.md"
            );
            assert_eq!(limit, MAX_README);
            Ok(b"README".to_vec())
        })
        .unwrap();
        assert!(!details.verified);
        assert_eq!(details.reference.as_deref(), Some("v1"));
        assert!(details.preview.is_none());
        assert!(home.path().join("v1/assets").is_file());
    }

    #[test]
    fn invalid_utf8_or_unsupported_preview_never_enters_cache() {
        let home = tempfile::tempdir().unwrap();
        let invalid = b"<svg>bad</svg>";
        let readme = DocAsset {
            path: "ADDON.md".into(),
            size: 2,
            sha256: hex(&Sha256::digest(b"ok")),
        };
        let preview = DocAsset {
            path: "image.svg".into(),
            size: invalid.len(),
            sha256: hex(&Sha256::digest(invalid)),
        };
        let source = Source {
            repository: "/test/addon",
            reference: "0123456789012345678901234567890123456789",
            readme: Some(&readme),
            preview: Some(&preview),
        };
        assert!(matches!(
            cached_details(home.path(), source, |url, _| {
                Ok(if url.ends_with("ADDON.md") {
                    b"ok".to_vec()
                } else {
                    invalid.to_vec()
                })
            }),
            Err(Problem::Invalid)
        ));
        assert!(matches!(
            cached_details(
                home.path(),
                Source {
                    readme: None,
                    preview: None,
                    ..source
                },
                |_, _| Ok(vec![0xff])
            ),
            Err(Problem::Invalid)
        ));
        assert!(!home.path().join(source.reference).exists());
    }
}
