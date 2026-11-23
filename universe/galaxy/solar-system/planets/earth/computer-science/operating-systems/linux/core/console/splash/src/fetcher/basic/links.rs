//! `basic_links` — user-authored bookmark list emitted as the row-shaped variants. Pairs with a
//! per-directory `.splash/dashboard.toml` so a repo can ship pinned URLs (docs, CI,
//! dashboards) that surface automatically on `cd`.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::ImageLinkedList,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "links",
    type_hint: "list of {label, url?, subtitle?, thumbnail?}",
    required: false,
    default: Some("[]"),
    description: "Pinned links rendered as rows. `label` is required; `url` makes the row OSC8-clickable in `LinkedTextBlock` / `ImageLinkedList`; `thumbnail` (file path) lights up the `ImageLinkedList` variant.",
}];

pub struct BasicLinks;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub links: Vec<LinkConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    pub label: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl RealtimeFetcher for BasicLinks {
    fn name(&self) -> &str {
        "basic_links"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a user-authored list of pinned links from `[widget.options].links`. `LinkedTextBlock` (default) and `ImageLinkedList` wrap rows with a URL in OSC 8 escape sequences so terminals surface them as clickable; `Text` headlines the first label, `TextBlock` is plain labels, `MarkdownTextBlock` emits `- [label](url)` bullets, `Entries` is label → url rows. Right for per-directory bookmarks (repo docs, CI dashboard, deploy console)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                ("GitHub", Some("https://github.com/")),
                ("Docs", Some("https://example.com/docs")),
            ]),
            Shape::Text => samples::text("GitHub"),
            Shape::TextBlock => samples::text_block(&["GitHub", "Docs"]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- [GitHub](https://github.com/)\n- [Docs](https://example.com/docs)",
            ),
            Shape::Entries => samples::entries(&[
                ("GitHub", "https://github.com/"),
                ("Docs", "https://example.com/docs"),
            ]),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                ("GitHub", Some("https://github.com/"), None, Some("source")),
                (
                    "Docs",
                    Some("https://example.com/docs"),
                    None,
                    Some("reference"),
                ),
            ]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        common::bare(body_for_shape(&opts.links, shape))
    }
}

fn body_for_shape(links: &[LinkConfig], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: links.first().map(|l| l.label.clone()).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: links.iter().map(|l| l.label.clone()).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: links
                .iter()
                .map(markdown_bullet)
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: links
                .iter()
                .map(|l| Entry {
                    key: l.label.clone(),
                    value: l.url.clone(),
                    status: None,
                })
                .collect(),
        }),
        Shape::ImageLinkedList => Body::ImageLinkedList(ImageLinkedListData {
            items: links
                .iter()
                .map(|l| ImageLinkedItem {
                    title: l.label.clone(),
                    url: l.url.clone(),
                    thumbnail_path: l.thumbnail.clone(),
                    subtitle: l.subtitle.clone(),
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: links
                .iter()
                .map(|l| LinkedLine {
                    text: l.label.clone(),
                    url: l.url.clone(),
                })
                .collect(),
        }),
    }
}

fn markdown_bullet(link: &LinkConfig) -> String {
    match &link.url {
        Some(url) => format!("- [{}]({})", link.label, url),
        None => format!("- {}", link.label),
    }
}

