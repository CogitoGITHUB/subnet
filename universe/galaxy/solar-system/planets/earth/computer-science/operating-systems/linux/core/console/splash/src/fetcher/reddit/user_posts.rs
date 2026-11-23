//! `reddit_user_posts` — recent submissions for a Reddit user.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::client::fetch_listing;
use super::common::{
    Post, SHAPES, cache_key_for, network_unavailable_body, normalize_user, normalized_count,
    render_posts, render_posts_image_linked, sample_post_body,
};

const DEFAULT_USER: &str = "spez";

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: false,
        default: Some("\"spez\""),
        description: "Reddit username (without `/u/` prefix).",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of submissions to display.",
    },
];

pub struct RedditUserPostsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

#[async_trait]
impl Fetcher for RedditUserPostsFetcher {
    fn name(&self) -> &str {
        "reddit_user_posts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent submissions by a single Reddit user. Use `reddit_user_comments` for that user's comments instead, or `reddit_subreddit_posts` to follow a subreddit rather than a person."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_post_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let user = normalize_user(opts.user.as_deref().unwrap_or(DEFAULT_USER))?;
        match fetch_user_posts(&user, count).await {
            Ok(posts) => Ok(payload(render_for_shape(&posts, shape).await)),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

/// Sibling of [`reddit_subreddit_posts::render_for_shape`]: pre-downloads thumbnails for the
/// `ImageLinkedList` shape so the renderer can show them, and falls through to the sync
/// `render_posts` for every other shape.
async fn render_for_shape(posts: &[Post], shape: Shape) -> Body {
    if !matches!(shape, Shape::ImageLinkedList) {
        return render_posts(posts, shape);
    }
    let urls: Vec<Option<String>> = posts
        .iter()
        .map(|p| p.thumbnail_url().map(str::to_string))
        .collect();
    let paths = thumbnails::download_many(&urls).await;
    render_posts_image_linked(posts, &paths)
}

async fn fetch_user_posts(user: &str, count: usize) -> Result<Vec<Post>, FetchError> {
    let path = format!("/user/{user}/submitted.json?limit={count}&raw_json=1");
    fetch_listing(&path).await
}

