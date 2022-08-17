use std::{error::Error, time::Duration};
use tracing::{info, debug, instrument};
use tracing_subscriber::{EnvFilter, Registry, util::SubscriberInitExt, layer::SubscriberExt};
use tracing_tree::HierarchicalLayer;
use chrono::{DateTime, Utc, };
use serde::{Deserialize, de};

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

mod ts_float_seconds {
    use chrono::NaiveDateTime;
    use super::{de, DateTime, Utc};

    pub struct SecondsTimestampVisitor;

    pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_f64(SecondsTimestampVisitor)
    }

    impl<'de> de::Visitor<'de> for SecondsTimestampVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a unix timestamp in seconds")
        }

        /// Deserialize a timestamp in seconds since the epoch
        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let naive = NaiveDateTime::from_timestamp(value as i64, 0);
            Ok(DateTime::from_utc(naive, Utc))
        }
    }
}

#[instrument]
async fn get_page(subreddit: &str, after: Option<String>) -> Result<Vec<(String, Post)>, Box<dyn Error>> {
    let client = reqwest::Client::new();

    let url = format!("https://reddit.com/r/{}/new.json", subreddit);
    let mut query: Vec<(&str, String)> = vec![];

    if let Some(after) = after {
        query.push(("after", after));
    }

    let query_string = query.iter().map(|(k,v)| format!("{}={}", k, v)).collect::<Vec<String>>().join(",");
    info!(%url, %query_string, "sending request");

    let res = client.get(url).query(&query).send().await;
    debug!(?res, "got result");

    let res: Listing = res?.json().await?;
    debug!(?res, "parsed");

    return Ok(res.data.children.into_iter().map(|child| (child.kind, child.data)).collect());
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

    get_subreddit_results("rust").await?;

    Ok(())
}
