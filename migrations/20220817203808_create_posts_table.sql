CREATE TABLE posts(
  id INTEGER PRIMARY KEY NOT NULL,

  reddit_id TEXT NOT NULL,
  subreddit INTEGER NOT NULL,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  permalink TEXT NOT NULL,
  created INTEGER NOT NULL,
  ups INTEGER NOT NULL,
  threshold_passed INTEGER,

  FOREIGN KEY(subreddit) REFERENCES subreddits(id)
  UNIQUE(reddit_id)
); 
 
