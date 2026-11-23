//! `reddit_subreddit_posts` — public listing for a subreddit (`top|hot|new|rising`).

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
    Post, SHAPES, cache_key_for, network_unavailable_body, normalize_subreddit, normalized_count,
    render_posts, render_posts_image_linked, sample_post_body,
};

const DEFAULT_SUBREDDIT: &str = "programming";

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "subreddit",
        type_hint: "string",
        required: false,
        default: Some("\"programming\""),
        description: "Subreddit name without `/r/` prefix.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of posts to display.",
    },
    OptionSchema {
        name: "type",
        type_hint: "\"top\" | \"hot\" | \"new\" | \"rising\"",
        required: false,
        default: Some("\"top\""),
        description: "Subreddit listing type.",
    },
    OptionSchema {
        name: "period",
        type_hint: "\"hour\" | \"day\" | \"week\" | \"month\" | \"year\" | \"all\"",
        required: false,
        default: Some("\"day\""),
        description: "Ranking window used when `type = \"top\"`.",
    },
];

pub struct RedditSubredditPostsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default, rename = "type")]
    r#type: Option<ListingType>,
    #[serde(default)]
    period: Option<Period>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ListingType {
    #[default]
    Top,
    Hot,
    New,
    Rising,
}

impl ListingType {
    fn path(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Hot => "hot",
            Self::New => "new",
            Self::Rising => "rising",
        }
    }

    fn needs_period(self) -> bool {
        matches!(self, Self::Top)
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Period {
    Hour,
    #[default]
    Day,
    Week,
    Month,
    Year,
    All,
}

impl Period {
    fn as_query(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }
}

#[async_trait]
impl Fetcher for RedditSubredditPostsFetcher {
    fn name(&self) -> &str {
        "reddit_subreddit_posts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Posts from a single subreddit's public listing — `top` / `hot` / `new` / `rising`, with a `period` window for `top`. Pair with `reddit_user_posts` for one user's submissions or `reddit_user_comments` for their comments."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 15
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
        let listing_type = opts.r#type.unwrap_or_default();
        let period = opts.period.unwrap_or_default();
        let subreddit =
            normalize_subreddit(opts.subreddit.as_deref().unwrap_or(DEFAULT_SUBREDDIT))?;
        match fetch_subreddit_posts(&subreddit, count, listing_type, period).await {
            Ok(posts) => Ok(payload(render_for_shape(&posts, shape).await)),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

/// Wraps the sync [`render_posts`] but takes the async detour for `ImageLinkedList`: downloads
/// each post's thumbnail (Reddit serves them as remote URLs) into the shared cache and pins the
/// resolved local paths onto each card. Posts without a thumbnail keep `None`, so the renderer
/// shows a blank thumbnail cell rather than an error.
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

async fn fetch_subreddit_posts(
    subreddit: &str,
    count: usize,
    listing_type: ListingType,
    period: Period,
) -> Result<Vec<Post>, FetchError> {
    fetch_listing(&listing_path(subreddit, count, listing_type, period)).await
}

fn listing_path(
    subreddit: &str,
    count: usize,
    listing_type: ListingType,
    period: Period,
) -> String {
    let mut path = format!(
        "/r/{subreddit}/{}.json?limit={count}&raw_json=1",
        listing_type.path()
    );
    if listing_type.needs_period() {
        path.push_str("&t=");
        path.push_str(period.as_query());
    }
    path
}

