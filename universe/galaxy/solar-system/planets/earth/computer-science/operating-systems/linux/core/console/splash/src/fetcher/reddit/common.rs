//! Cross-cutting helpers for the `reddit_*` family: option parsing, name normalization,
//! the shared `Post` DTO, and `Post`-based rendering / sample bodies.

use serde::Deserialize;

use crate::fetcher::FetchContext;
use crate::fetcher::FetchError;
use crate::fetcher::github::common::cache_key;
use crate::payload::{
    Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::client::SITE_BASE;

/// Shapes the post-listing fetchers (`reddit_subreddit_posts`, `reddit_user_posts`) emit.
/// Includes `ImageLinkedList` because Reddit posts can carry thumbnails.
pub(super) const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Entries,
];

/// Shapes for the comment fetcher (`reddit_user_comments`). Excludes `ImageLinkedList`
/// because Reddit comments don't have images — declaring it would lie about the output
/// contract.
pub(super) const COMMENT_SHAPES: &[Shape] =
    &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

pub(super) const DEFAULT_COUNT: u32 = 10;
pub(super) const MIN_COUNT: u32 = 1;
pub(super) const MAX_COUNT: u32 = 30;

const SUBREDDIT_NAME_MAX: usize = 21;
const USER_NAME_MAX: usize = 20;

pub(super) fn cache_key_for(name: &str, ctx: &FetchContext) -> String {
    let extra = ctx
        .options
        .as_ref()
        .and_then(|value| toml::to_string(value).ok())
        .unwrap_or_default();
    cache_key(name, ctx, &extra)
}

pub(super) fn normalized_count(value: Option<u32>) -> usize {
    value.unwrap_or(DEFAULT_COUNT).clamp(MIN_COUNT, MAX_COUNT) as usize
}

pub(super) fn normalize_subreddit(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/r/")
        .trim_start_matches("/r/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("subreddit must not be empty".into()));
    }
    if trimmed.len() > SUBREDDIT_NAME_MAX {
        return Err(FetchError::Failed("subreddit is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(FetchError::Failed(
            "subreddit must contain only [A-Za-z0-9_]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub(super) fn normalize_user(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/user/")
        .trim_start_matches("/u/")
        .trim_start_matches("u/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("user must not be empty".into()));
    }
    if trimmed.len() > USER_NAME_MAX {
        return Err(FetchError::Failed("user is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(FetchError::Failed(
            "user must contain only [A-Za-z0-9_-]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub(super) fn network_unavailable_body(shape: Shape, err: &str) -> Body {
    let line = format!("⚠ {err}");
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![Entry {
                key: "reddit unavailable".into(),
                value: Some(err.into()),
                status: None,
            }],
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: line,
                url: None,
            }],
        }),
        Shape::ImageLinkedList => Body::ImageLinkedList(ImageLinkedListData {
            items: vec![ImageLinkedItem {
                title: line,
                url: None,
                thumbnail_path: None,
                subtitle: None,
            }],
        }),
        _ => Body::TextBlock(TextBlockData { lines: vec![line] }),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Post {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub score: Option<i64>,
    #[serde(default)]
    pub num_comments: Option<u64>,
    #[serde(default)]
    pub subreddit: Option<String>,
    #[serde(default)]
    pub permalink: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Reddit returns one of: a thumbnail URL (http/https), the strings `"self"` / `"default"`
    /// / `"nsfw"` / `"spoiler"`, or an empty string. Only the URL form is usable as an image —
    /// see [`Post::thumbnail_url`].
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl Post {
    /// HTTP(S) thumbnail URL if Reddit gave us one. Filters out the placeholder strings Reddit
    /// returns when the post isn't a media post (`"self"`, `"default"`, …) and the empty case.
    pub(super) fn thumbnail_url(&self) -> Option<&str> {
        self.thumbnail
            .as_deref()
            .map(str::trim)
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
    }
}

pub(super) fn render_posts(posts: &[Post], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: posts
                .iter()
                .map(|post| Entry {
                    key: post_title(post),
                    value: Some(post_meta(post)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: posts
                .iter()
                .map(|post| LinkedLine {
                    text: format!("{}  {}", post_meta(post), post_title(post)),
                    url: post_link(post),
                })
                .collect(),
        }),
        Shape::ImageLinkedList => render_posts_image_linked(posts, &vec![None; posts.len()]),
        _ => Body::TextBlock(TextBlockData {
            lines: posts
                .iter()
                .map(|post| format!("{}  {}", post_meta(post), post_title(post)))
                .collect(),
        }),
    }
}

/// `ImageLinkedList` rendering with pre-resolved thumbnail paths. Kept separate from
/// [`render_posts`] because the thumbnail download is async — callers run it once at fetch
/// time and pass the resolved paths back in, so the sync renderer path can still serve
/// thumbnail-less callers (`sample_post_body`, error fallbacks) without an executor.
pub(super) fn render_posts_image_linked(
    posts: &[Post],
    thumbnail_paths: &[Option<std::path::PathBuf>],
) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: posts
            .iter()
            .enumerate()
            .map(|(i, post)| ImageLinkedItem {
                title: post_title(post),
                url: post_link(post),
                thumbnail_path: thumbnail_paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(post_meta(post)),
            })
            .collect(),
    })
}

pub(super) fn sample_post_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
                Some("https://www.reddit.com/r/programming/comments/abc123/demo/"),
            ),
            (
                "r/rust · 934↑ 104c  Why async Rust feels different from Go",
                Some("https://example.com/async-rust-vs-go"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "Show: terminal dashboard with local trust model",
                Some("https://www.reddit.com/r/programming/comments/abc123/demo/"),
                None,
                Some("r/programming · 1520↑ 218c"),
            ),
            (
                "Why async Rust feels different from Go",
                Some("https://example.com/async-rust-vs-go"),
                None,
                Some("r/rust · 934↑ 104c"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
            "r/rust · 934↑ 104c  Why async Rust feels different from Go",
        ]),
        Shape::Entries => samples::entries(&[
            (
                "Show: terminal dashboard with local trust model",
                "r/programming · 1520↑ 218c",
            ),
            (
                "Why async Rust feels different from Go",
                "r/rust · 934↑ 104c",
            ),
        ]),
        _ => return None,
    })
}

pub(super) fn post_title(post: &Post) -> String {
    post.title.as_deref().unwrap_or("(no title)").to_string()
}

pub(super) fn post_meta(post: &Post) -> String {
    let subreddit = post
        .subreddit
        .as_deref()
        .map(|name| format!("r/{name} · "))
        .unwrap_or_default();
    let score = post.score.unwrap_or(0);
    let comments = post.num_comments.unwrap_or(0);
    format!("{subreddit}{score}↑ {comments}c")
}

fn post_link(post: &Post) -> Option<String> {
    post.url
        .as_deref()
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .map(String::from)
        .or_else(|| {
            post.permalink
                .as_deref()
                .map(|path| format!("{SITE_BASE}{path}"))
        })
}

