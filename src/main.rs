use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use rexml::ts_float_seconds;
use serde::Deserialize;
use sqlx::{query, sqlite::SqlitePoolOptions, SqlitePool};
use std::error::Error;
use tracing::{debug, info, instrument, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use tracing_tree::HierarchicalLayer;

#[derive(Debug, Clone, Deserialize)]
struct Post {
    title: String,
    ups: u32,
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

    Ok(res
        .data
        .children
        .into_iter()
        .map(|child| (child.kind, child.data))
        .collect())
}

#[instrument]
async fn get_subreddit_results(
    pool: &SqlitePool,
    subreddit: String,
    subreddit_id: i64,
    cutoff: chrono::Duration,
    threshold: u32,
) -> Result<(), Box<dyn Error>> {
    info!(%subreddit, "scraping");

    let mut after: Option<String> = None;

    'a: loop {
        let mut res = get_page(&subreddit, after).await?;
        info!(%subreddit, "got {} results", res.len());
        if res.is_empty() {
            break;
        }

        {
            let mut conn = pool.acquire().await?;
            for (kind, post) in &res {
                debug!(%subreddit, "({}) {} - {}", post.ups, post.title, post.url);
                if post.created < Utc::now() - cutoff {
                    break 'a;
                }

                let ups = sqlx::query!("SELECT ups FROM posts WHERE reddit_id=?", post.id)
                    .fetch_optional(&mut conn)
                    .await?;

                if ups.is_none() {
                    let created = post.created.timestamp();

                    sqlx::query!(
                        "INSERT INTO posts(reddit_id, subreddit, kind, title, url, permalink, created, ups)
                         VALUES (?,?,?,?,?,?,?,?)",
                         post.id, subreddit_id, kind, post.title, post.url, post.permalink, created, post.ups
                    ).execute(&mut conn).await?;
                }

                if post.ups >= threshold && (ups.is_none() || (ups.unwrap().ups as u32) < threshold)
                {
                    info!(%subreddit, %post.id, "passed the threshold");

                    let now_timestamp = Utc::now().timestamp();
                    sqlx::query!(
                        "UPDATE posts SET ups = ?, threshold_passed = ? WHERE reddit_id = ? AND subreddit = ?",
                        post.ups,
                        now_timestamp,
                        post.id,
                        subreddit_id,
                    )
                    .execute(&mut conn)
                    .await?;
                }
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

        let futs = {
            let mut conn = pool.acquire().await?;
            let mut rows =
                query!("SELECT id, name, time_cutoff_seconds, upvote_threshold FROM subreddits")
                    .fetch(&mut conn);
            let mut futs = Vec::new();
            while let Some(row) = rows.try_next().await? {
                debug!(?row, "got subreddit");

                let dur = chrono::Duration::seconds(row.time_cutoff_seconds);
                futs.push(get_subreddit_results(
                    pool,
                    row.name,
                    row.id,
                    dur,
                    row.upvote_threshold as u32,
                ));
            }
            futs
        };

        let results = futures::future::join_all(futs).await;
        for res in results {
            match res {
                Ok(()) => {}
                Err(e) => warn!("error whilst getting results: {}", e),
            }
        }

        info!("waiting to scrape posts");
        tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
    }
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

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite://rexml.db")
        .await?;

    {
        let mut conn = pool.acquire().await?;
        sqlx::migrate!().run(&mut conn).await?;
    }

    posts_worker(&pool).await?;

    Ok(())
}
