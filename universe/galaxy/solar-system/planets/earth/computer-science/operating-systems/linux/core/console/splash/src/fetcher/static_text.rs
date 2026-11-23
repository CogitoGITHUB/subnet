use async_trait::async_trait;

use crate::payload::{Body, MarkdownTextBlockData, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::{FetchContext, FetchError, Fetcher, Safety};

/// Emits `format` verbatim. Declares both text shapes: `Text` returns the entire format as one
/// string, `TextBlock` splits on `\n` so users can ship multi-line fixed text blocks ("welcome
/// to this project", setup notes, etc.) without needing a dedicated fetcher. Empty or missing
/// format produces empty output — the widget renders nothing rather than a spurious blank line.
pub struct StaticText;

#[async_trait]
impl Fetcher for StaticText {
    fn name(&self) -> &str {
        "basic_static"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a constant string supplied by the widget's `format` option, split into lines on `\\n` for `TextBlock`, collapsed to single-spaces for `Text`, and passed through verbatim for `MarkdownTextBlock`. Use it for greetings, project banners, or fixed welcome notes that don't need a dedicated fetcher."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 24
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text, Shape::TextBlock, Shape::MarkdownTextBlock]
    }
    fn default_shape(&self) -> Shape {
        Shape::TextBlock
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("Hello, splashboard!")),
            Shape::TextBlock => Some(samples::text_block(&["Hello, splashboard!"])),
            Shape::MarkdownTextBlock => Some(samples::markdown(
                "# Hello, splashboard!\n\nMarkdown body with **bold** and `code`.",
            )),
            _ => None,
        }
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let source = ctx.format.as_deref().unwrap_or("");
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: source.lines().collect::<Vec<_>>().join(" "),
            }),
            Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: source.to_string(),
            }),
            _ => {
                let lines = if source.is_empty() {
                    Vec::new()
                } else {
                    source.split('\n').map(String::from).collect()
                };
                Body::TextBlock(TextBlockData { lines })
            }
        };
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body,
        })
    }
}

