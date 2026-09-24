# gh-network

A command-line tool written in Rust for discovering and networking with developers on GitHub.

```text
==================================================
  GitHub Profile:  @DRNZY
  Followers:     8
  Following:     688
  Public Repos:  14
--------------------------------------------------
  Rate Budget:     5000/5000 (resets in 60m)
  Follow Cache:   885 developers
  Pool Cache:   3077 candidates ready
==================================================
```

## Features

- Scrapes stargazers, contributors, and active developers across repositories concurrently using Tokio.
- Caches candidate profiles in `~/.config/github/candidate_pool.json` to prevent redundant API queries.
- Detects daily mutation limits by verifying follow status via `GET /user/following/:user`.
- Tracks `X-RateLimit-Remaining` directly from response headers.
- Adds randomized delays between requests to stay within rate limits.

## Installation

```bash
cargo build --release
cp target/release/gh-network ~/.local/bin/gh-network
```

## Usage

```bash
# View live profile metrics and rate limit budget
gh-network status

# Harvest candidates across target repositories
gh-network harvest

# Follow candidates from the local pool
gh-network follow --max 100

# Reconcile local cache with current GitHub following list
gh-network sync
```

## License

MIT
