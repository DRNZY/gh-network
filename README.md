# gh-network

High-performance, async GitHub developer networking and community discovery engine written in Rust.

```
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

- **Async Multi-Repo Harvesting:** Scrapes stargazers, contributors, and active developers across 30+ core ecosystem repositories concurrently using Tokio.
- **Candidate Pool Caching:** Persistent JSON storage in `~/.config/github/candidate_pool.json` prevents redundant API calls.
- **Shadow-Throttle Detection:** Probes mutation status with `GET /user/following/:user` to safely detect GitHub's daily mutation limits without burning quota.
- **Case-Insensitive HTTP Header Parsing:** Native `reqwest::header::HeaderMap` for real-time `X-RateLimit-Remaining` tracking.
- **Paced Follow Sequence:** Randomized human-like jitter intervals (850ms - 1350ms) to respect GitHub rate limits.

## Installation

```bash
cargo build --release
cp target/release/gh-network ~/.local/bin/gh-network
```

## Usage

```bash
# View live profile metrics and rate limit budget
gh-network status

# Asynchronously harvest candidates across ecosystems
gh-network harvest

# Follow a batch of candidates from the pool
gh-network follow --max 100

# Reconcile local cache with live GitHub following list
gh-network sync
```

## License

MIT
