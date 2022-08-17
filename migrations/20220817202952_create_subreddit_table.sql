CREATE TABLE subreddits (
  id INTEGER NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  upvote_threshold INTEGER NOT NULL,
  time_cutoff_seconds INTEGER NOT NULL,
    
  UNIQUE(name)
);
