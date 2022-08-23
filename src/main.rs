use axum::{
    extract::{Extension, Path},
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDateTime, Utc};
use futures::TryStreamExt;
use minidom::Element;
use rexml::{ts_float_seconds, HttpError};
use serde::Deserialize;
use sqlx::{query, sqlite::SqlitePoolOptions, SqlitePool};
use std::error::Error;
use tokio::sync::mpsc;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, instrument};
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
async fn posts_worker(
    pool: &SqlitePool,
    mut rx: mpsc::Receiver<bool>,
) -> Result<(), Box<dyn Error>> {
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
                Err(e) => error!("error whilst getting results: {}", e),
            }
        }

        info!("waiting to scrape posts");
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(5 * 60)) => { info!("posts worker finished sleep"); },
            _ = rx.recv() => { info!("posts worker woken up"); },
        }
    }
}

fn timestamp_to_utc(ts: i64) -> DateTime<Utc> {
    DateTime::from_utc(NaiveDateTime::from_timestamp(ts, 0), Utc)
}

async fn handler(
    Extension(State { pool, .. }): Extension<State>,
    Path(subreddit): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    debug!(%subreddit, "got request");
    let mut conn = pool.acquire().await?;

    let res = sqlx::query!(
        "SELECT id FROM subreddits WHERE subreddits.name = ? LIMIT 1",
        subreddit
    )
    .fetch_one(&mut conn)
    .await;
    match res {
        Ok(_) => {}
        Err(sqlx::Error::RowNotFound) => {
            debug!(%subreddit, "subreddit not registered");
            return Err(HttpError::NotFound);
        }
        Err(e) => return Err(e.into()),
    }

    let rows = sqlx::query!(
        "SELECT p.title, p.url, p.threshold_passed
          FROM subreddits s
          LEFT JOIN posts p ON p.subreddit = s.id
          WHERE s.name = ? AND p.threshold_passed IS NOT NULL
          ORDER BY p.threshold_passed DESC
          LIMIT 50",
        subreddit
    )
    .fetch_all(&mut conn)
    .await?;

    let n_results = rows.len();
    debug!(%n_results, "got posts");

    let entries = rows.iter().map(|row| {
        let updated = timestamp_to_utc(row.threshold_passed.unwrap());
        Element::builder("entry", "")
            .append(
                Element::builder("id", "")
                    .append(row.url.clone().unwrap())
                    .build(),
            )
            .append(
                Element::builder("title", "")
                    .append(row.title.clone().unwrap())
                    .build(),
            )
            .append(
                Element::builder("link", "")
                    .attr("href", row.url.clone().unwrap())
                    .build(),
            )
            .append(
                Element::builder("updated", "")
                    .append(updated.to_rfc3339())
                    .build(),
            )
            .build()
    });

    let mut feed = Element::builder("feed", "")
        .attr("xmlns", "http://www.w3.org/2005/Atom")
        .append(
            Element::builder("id", "")
                .append(format!("http://rexml.mattjhall.xyz/{}", subreddit))
                .build(),
        )
        .append(
            Element::builder("link", "")
                .attr("rel", "self")
                .attr("href", format!("http://rexml.mattjhall.xyz/{}", subreddit))
                .build(),
        )
        .append(
            Element::builder("title", "")
                .append(format!("{} posts", subreddit))
                .build(),
        )
        .append_all(entries);

    if !rows.is_empty() {
        feed = feed.append(
            Element::builder("updated", "")
                .append(timestamp_to_utc(rows[0].threshold_passed.unwrap()).to_rfc3339())
                .build(),
        )
    }

    let feed = feed.build();

    let mut res = Vec::new();
    feed.write_to(&mut res)
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;
    let resp = Response::builder()
        .header(CONTENT_TYPE, "application/atom+xml")
        .body(axum::body::Body::from(res))
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;
    Ok(resp)
}

#[derive(Debug, Deserialize)]
struct CreateSubreddit {
    upvote_threshold: i64,
    time_cutoff_seconds: i64,
}

async fn post_handler(
    Extension(State { pool, tx }): Extension<State>,
    Path(subreddit): Path<String>,
    Json(payload): Json<CreateSubreddit>,
) -> Result<impl IntoResponse, HttpError> {
    info!(%subreddit, ?payload, "trying to create subreddit record");
    let mut conn = pool.acquire().await?;
    let res = query!(
        "INSERT INTO subreddits(name, upvote_threshold, time_cutoff_seconds) VALUES (?,?,?)",
        subreddit,
        payload.upvote_threshold,
        payload.time_cutoff_seconds
    )
    .execute(&mut conn)
    .await;

    let e = match res {
        Ok(_) => {
            let _ = tx.send(true).await;
            return Ok(());
        }
        Err(sqlx::Error::Database(e)) => {
            if let Some(code) = e.code() {
                if code == "2067" {
                    return Err(HttpError::AlreadyExists);
                }
            }

            sqlx::Error::Database(e).into()
        }
        Err(e) => e.into(),
    };

    Err(e)
}

#[derive(Clone)]
struct State {
    pool: SqlitePool,
    tx: mpsc::Sender<bool>,
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

    let conn_str = std::env::var("REXML_DB_URL").unwrap_or("sqlite://rexml.db".into());
    info!(%conn_str, "connecting");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&conn_str)
        .await?;

    {
        let mut conn = pool.acquire().await?;
        sqlx::migrate!().run(&mut conn).await?;
    }

    let (tx, rx) = mpsc::channel(1);

    let state = State {
        pool: pool.clone(),
        tx,
    };

    let app = Router::new()
        .route("/:subreddit", get(handler))
        .layer(Extension(state.clone()))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        );

    let priv_app = Router::new()
        .route("/:subreddit", post(post_handler))
        .layer(Extension(state))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        );

    let server =
        axum::Server::bind(&"0.0.0.0:4328".parse().unwrap()).serve(app.into_make_service());

    let priv_server =
        axum::Server::bind(&"0.0.0.0:4329".parse().unwrap()).serve(priv_app.into_make_service());

    let worker = posts_worker(&pool, rx);

    let _ = futures::join!(server, priv_server, worker);

    Ok(())
}
