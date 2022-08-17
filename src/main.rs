use std::error::Error;
use tracing_subscriber::{EnvFilter, Registry, util::SubscriberInitExt, prelude::__tracing_subscriber_SubscriberExt};
use tracing_tree::HierarchicalLayer;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct Post {
    title: String,
    ups: u64,
    permalink: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ListingChild {
    data: Post,
}

#[derive(Debug, Clone, Deserialize)]
struct ListingData {
    children: Vec<ListingChild>,
}

#[derive(Debug, Clone, Deserialize)]
struct Listing {
    data: ListingData,
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

    let res: Listing = reqwest::get("https://reddit.com/r/rust/new.json").await?.json().await?;

    for post in res.data.children {
        let post = post.data;
        println!("({}) {}: {}", post.ups, post.title, post.url);
    }
  
    Ok(())
}
