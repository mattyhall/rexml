use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use rexml::ts_float_seconds;
use serde::Deserialize;
use sqlx::{query, Connection, SqliteConnection, SqlitePool};
use std::error::Error;
use tracing::{debug, info, instrument, warn};
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
    info!(%subreddit, %url, %query_string, "sending request");

    let res = client.get(url).query(&query).send().await;
    debug!(%subreddit, ?res, "got result");

    let res: Listing = res?.json().await?;
    debug!(%subreddit, ?res, "parsed");

    return Ok(res
        .data
        .children
        .into_iter()
        .map(|child| (child.kind, child.data))
        .collect());
}

#[instrument]
async fn get_subreddit_results(
    subreddit: String,
    cutoff: chrono::Duration,
) -> Result<(), Box<dyn Error>> {
    info!(%subreddit, "scraping");

    let mut after: Option<String> = None;

    'a: loop {
        let mut res = get_page(&subreddit, after).await?;
        info!(%subreddit, "got {} results", res.len());
        if res.len() == 0 {
            break;
        }

        for (_, post) in &res {
            debug!(%subreddit, "({}) {} - {}", post.ups, post.title, post.url);
            if post.created < Utc::now() - cutoff {
                break 'a;
            }
        }

        let (kind, post) = res.pop().unwrap();
        after = Some(format!("{}_{}", kind, post.id));
    }

    Ok(())
}

#[instrument]
async fn posts_worker(pool: &SqlitePool) -> Result<(), Box<dyn Error>> {
    loop {
        info!("scraping posts");

        let mut conn = pool.acquire().await?;
        let mut rows = query!("SELECT name, time_cutoff_seconds FROM subreddits").fetch(&mut conn);
        let mut futs = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let dur = chrono::Duration::seconds(row.time_cutoff_seconds);
            futs.push(get_subreddit_results(row.name, dur));
        }

        let results = futures::future::join_all(futs).await;
        for res in results {
            match res {
                Ok(()) => {}
                Err(e) => warn!("error whilst getting results: {}", e),
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
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

    let mut pool = SqlitePool::connect("sqlite://rexml.db").await?;

    {
        let mut conn = pool.acquire().await?;
        sqlx::migrate!().run(&mut conn).await?;
    }

    posts_worker(&pool).await?;

    Ok(())
}
