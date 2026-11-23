use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{
    BadgeData, Bar, BarsData, Body, CalendarData, EntriesData, Entry, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, NumberSeriesData, Payload, Status, TextBlockData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::NumberSeries,
    Shape::Bars,
    Shape::Calendar,
    Shape::Badge,
    Shape::Timeline,
];

/// Repository age measured from the first commit reachable from `HEAD`. `Text` defaults to a
/// compact duration ("2y 3m"); switch via `format = "since"` for "since 2024-01-15" or
/// `format = "full"` for the combined "2y 3m · since 2024-01-15" — handy as a preset subtitle.
/// Structural shapes (`Entries`, `Bars`, `NumberSeries`) expose the years / months / days split;
/// `Calendar` shows the month of the first commit with that day highlighted; `Badge` reports the
/// age tier (`fresh` < 30d, `young` < 1y, `mature` < 5y, `ancient` ≥ 5y); `Timeline` carries the
/// first-commit event so a `list_timeline` widget can render "First commit · 2y ago".
pub struct GitAge;

#[async_trait]
impl Fetcher for GitAge {
    fn name(&self) -> &str {
        "git_age"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Repository age from the first commit reachable from HEAD. Text/TextBlock/Markdown/Linked variants format the duration; Entries/Bars/NumberSeries expose the years/months/days split; Calendar highlights the first-commit day; Badge tags an age tier; Timeline emits the first-commit event."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_for(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let today = Utc::now().date_naive();
        Ok(payload(build(
            &repo,
            today,
            ctx.shape.unwrap_or(Shape::Text),
            ctx.format.as_deref(),
        )?))
    }
}

fn sample_for(shape: Shape) -> Option<Body> {
    let first = NaiveDate::from_ymd_opt(2024, 1, 15)?;
    let age = Age {
        years: 2,
        months: 3,
        days: 12,
    };
    Some(match shape {
        Shape::Text => samples::text("2y 3m"),
        Shape::TextBlock => text_block_body(&age, first),
        Shape::MarkdownTextBlock => markdown_body(&age, first),
        Shape::LinkedTextBlock => linked_body(&age, first),
        Shape::Entries => entries_body(&age, first),
        Shape::NumberSeries => number_series_body(&age),
        Shape::Bars => bars_body(&age),
        Shape::Calendar => calendar_body(first),
        Shape::Badge => badge_body(&age),
        Shape::Timeline => timeline_body(first, 1_705_276_800),
        _ => return None,
    })
}

fn build(
    repo: &gix::Repository,
    today: NaiveDate,
    shape: Shape,
    format: Option<&str>,
) -> Result<Body, FetchError> {
    let Some((first, first_seconds)) = first_commit(repo)? else {
        return Ok(empty_body(shape));
    };
    let age = Age::between(first, today);
    Ok(match shape {
        Shape::TextBlock => text_block_body(&age, first),
        Shape::MarkdownTextBlock => markdown_body(&age, first),
        Shape::LinkedTextBlock => linked_body(&age, first),
        Shape::Entries => entries_body(&age, first),
        Shape::NumberSeries => number_series_body(&age),
        Shape::Bars => bars_body(&age),
        Shape::Calendar => calendar_body(first),
        Shape::Badge => badge_body(&age),
        Shape::Timeline => timeline_body(first, first_seconds),
        _ => text_body(format_text(&age, first, format)),
    })
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData { lines: Vec::new() }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData { items: Vec::new() }),
        Shape::Entries => Body::Entries(EntriesData { items: Vec::new() }),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData { values: Vec::new() }),
        Shape::Bars => Body::Bars(BarsData { bars: Vec::new() }),
        Shape::Calendar => Body::Calendar(CalendarData {
            year: 1970,
            month: 1,
            day: None,
            events: Vec::new(),
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Warn,
            label: "no commits".into(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData { events: Vec::new() }),
        _ => text_body(""),
    }
}

fn first_commit(repo: &gix::Repository) -> Result<Option<(NaiveDate, i64)>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(None);
    };
    let walker = repo
        .rev_walk([head_id.detach()])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(fail)?;
    let oldest = walker
        .filter_map(Result::ok)
        .filter_map(|info| repo.find_commit(info.id).ok())
        .filter_map(|c| c.time().ok().map(|t| t.seconds))
        .min();
    Ok(oldest.and_then(|s| seconds_to_date(s).map(|d| (d, s))))
}

fn seconds_to_date(secs: i64) -> Option<NaiveDate> {
    DateTime::<Utc>::from_timestamp(secs, 0).map(|dt| dt.date_naive())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Age {
    years: u32,
    months: u32,
    days: u32,
}

impl Age {
    fn between(first: NaiveDate, today: NaiveDate) -> Self {
        if today <= first {
            return Self::zero();
        }
        let mut years = today.year() - first.year();
        let mut months = today.month() as i32 - first.month() as i32;
        let mut days = today.day() as i32 - first.day() as i32;
        if days < 0 {
            months -= 1;
            days += days_in_previous_month(today) as i32;
        }
        if months < 0 {
            years -= 1;
            months += 12;
        }
        Self {
            years: years.max(0) as u32,
            months: months.max(0) as u32,
            days: days.max(0) as u32,
        }
    }

    fn zero() -> Self {
        Self {
            years: 0,
            months: 0,
            days: 0,
        }
    }

    fn is_today(&self) -> bool {
        self.years == 0 && self.months == 0 && self.days == 0
    }

    fn tier(&self) -> &'static str {
        if self.years >= 5 {
            "ancient"
        } else if self.years >= 1 {
            "mature"
        } else if self.months >= 1 {
            "young"
        } else {
            "fresh"
        }
    }
}

fn days_in_previous_month(today: NaiveDate) -> u32 {
    let (y, m) = if today.month() == 1 {
        (today.year() - 1, 12u32)
    } else {
        (today.year(), today.month() - 1)
    };
    let next_first = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    let this_first = NaiveDate::from_ymd_opt(y, m, 1);
    match (this_first, next_first) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 30,
    }
}

fn format_duration(age: &Age) -> String {
    if age.is_today() {
        return "today".into();
    }
    if age.years > 0 {
        return match age.months {
            0 => format!("{}y", age.years),
            m => format!("{}y {}m", age.years, m),
        };
    }
    if age.months > 0 {
        return match age.days {
            0 => format!("{}m", age.months),
            d => format!("{}m {}d", age.months, d),
        };
    }
    format!("{}d", age.days)
}

fn format_long_duration(age: &Age) -> String {
    if age.is_today() {
        return "today".into();
    }
    let parts: Vec<_> = [(age.years, "y"), (age.months, "m"), (age.days, "d")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, suffix)| format!("{n}{suffix}"))
        .collect();
    parts.join(" ")
}

fn format_since(first: NaiveDate) -> String {
    format!("since {}", first.format("%Y-%m-%d"))
}

fn format_text(age: &Age, first: NaiveDate, format: Option<&str>) -> String {
    match format.map(str::trim) {
        Some("since") => format_since(first),
        Some("full") => format!("{} · {}", format_duration(age), format_since(first)),
        _ => format_duration(age),
    }
}

fn text_block_body(age: &Age, first: NaiveDate) -> Body {
    Body::TextBlock(TextBlockData {
        lines: vec![
            format!("Years:  {}", age.years),
            format!("Months: {}", age.months),
            format!("Days:   {}", age.days),
            format!("Since:  {}", first.format("%Y-%m-%d")),
        ],
    })
}

fn markdown_body(age: &Age, first: NaiveDate) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: format!(
            "**{}** since `{}`",
            format_long_duration(age),
            first.format("%Y-%m-%d")
        ),
    })
}

fn linked_body(age: &Age, first: NaiveDate) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: vec![
            LinkedLine {
                text: format_long_duration(age),
                url: None,
            },
            LinkedLine {
                text: format_since(first),
                url: None,
            },
        ],
    })
}

fn entries_body(age: &Age, first: NaiveDate) -> Body {
    Body::Entries(EntriesData {
        items: vec![
            entry("years", &age.years.to_string()),
            entry("months", &age.months.to_string()),
            entry("days", &age.days.to_string()),
            entry("first_commit_date", &first.format("%Y-%m-%d").to_string()),
        ],
    })
}

fn number_series_body(age: &Age) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: vec![age.years as u64, age.months as u64, age.days as u64],
    })
}

fn bars_body(age: &Age) -> Body {
    Body::Bars(BarsData {
        bars: vec![
            Bar {
                label: "years".into(),
                value: age.years as u64,
                value_label: None,
            },
            Bar {
                label: "months".into(),
                value: age.months as u64,
                value_label: None,
            },
            Bar {
                label: "days".into(),
                value: age.days as u64,
                value_label: None,
            },
        ],
    })
}

fn calendar_body(first: NaiveDate) -> Body {
    Body::Calendar(CalendarData {
        year: first.year(),
        month: first.month() as u8,
        day: Some(first.day() as u8),
        events: Vec::new(),
    })
}

fn badge_body(age: &Age) -> Body {
    Body::Badge(BadgeData {
        status: Status::Ok,
        label: format!("{} ({})", age.tier(), format_duration(age)),
    })
}

fn timeline_body(first: NaiveDate, seconds: i64) -> Body {
    Body::Timeline(TimelineData {
        events: vec![TimelineEvent {
            timestamp: seconds,
            title: "First commit".into(),
            detail: Some(first.format("%Y-%m-%d").to_string()),
            status: Some(Status::Ok),
        }],
    })
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

