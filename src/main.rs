use chrono::{DateTime, Utc};
use rexml::ts_float_seconds;
use serde::Deserialize;
use sqlx::{Connection, SqliteConnection};
use std::error::Error;
use tracing::{debug, info, instrument};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use tracing_tree::HierarchicalLayer;

#[derive(Debug, Clone, Deserialize)]
struct Post {
    title: String,
    ups: u64,
    permalink: String,
    url: String,
    id: String,

    #[serde(deserialize_with = "ts_float_seconds::deserialize")]
    created: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct ListingChild {
    data: Post,
    kind: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ListingData {
    children: Vec<ListingChild>,
}

#[derive(Debug, Clone, Deserialize)]
struct Listing {
    data: ListingData,
}

#[instrument]
async fn get_page(
    subreddit: &str,
    after: Option<String>,
) -> Result<Vec<(String, Post)>, Box<dyn Error>> {
    let client = reqwest::Client::new();

    let url = format!("https://reddit.com/r/{}/new.json", subreddit);
    let mut query: Vec<(&str, String)> = vec![];

    if let Some(after) = after {
        query.push(("after", after));
    }

    let query_string = query
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>()
        .join(",");
    info!(%url, %query_string, "sending request");

    let res = client.get(url).query(&query).send().await;
    debug!(?res, "got result");

    let res: Listing = res?.json().await?;
    debug!(?res, "parsed");

    return Ok(res
        .data
        .children
        .into_iter()
        .map(|child| (child.kind, child.data))
        .collect());
}

#[instrument]
async fn get_subreddit_results(subreddit: &str) -> Result<(), Box<dyn Error>> {
    let mut after: Option<String> = None;

    'a: loop {
        let mut res = get_page(subreddit, after).await?;
        info!("got {} results", res.len());
        if res.len() == 0 {
            break;
        }

        for (_, post) in &res {
            info!("({}) {} - {}", post.ups, post.title, post.url);
            if post.created < Utc::now() - chrono::Duration::days(1) {
                break 'a;
            }
        }

        let (kind, post) = res.pop().unwrap();
        after = Some(format!("{}_{}", kind, post.id));
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    Registry::default()
        .with(EnvFilter::from_default_env())
        .with(
            HierarchicalLayer::new(2)
                .with_targets(true)
                .with_bracketed_fields(true),
        )
        .init();

    let mut conn = SqliteConnection::connect("sqlite://rexml.db").await?;

    sqlx::migrate!().run(&mut conn).await?;

    get_subreddit_results("rust").await?;

    Ok(())
}
