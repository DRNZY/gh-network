use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_LENGTH, USER_AGENT};
use serde::{Deserialize, Serialize};

const TARGET_REPOS: &[&str] = &[
    // Wayland & Linux Desktop Runtimes
    "elkowar/eww",
    "Aylur/ags",
    "Alexays/Waybar",
    "hyprwm/Hyprland",
    "rust-windowing/winit",
    "fastfetch-cli/fastfetch",
    "alacritty/alacritty",
    // Modern Rust Systems & TUIs
    "ratatui/ratatui",
    "helix-editor/helix",
    "zellij-org/zellij",
    "sxyazi/yazi",
    "BurntSushi/ripgrep",
    "sharkdp/fd",
    "sharkdp/bat",
    "eza-community/eza",
    "dandavison/delta",
    "casey/just",
    "astral-sh/uv",
    "tauri-apps/tauri",
    "neovim/neovim",
];

const INFLUENTIAL_USERS: &[&str] = &[
    "vaxerski",
    "elkowar",
    "Aylur",
    "Alexays",
    "sharkdp",
    "BurntSushi",
    "orhun",
];

#[derive(Parser)]
#[command(
    name = "gh-network",
    about = "High-precision GitHub developer networking and relationship optimizer",
    version = "0.2.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show current profile metrics, rate limits, and network status
    Status,
    /// Harvest prospective developers from precision Wayland and Rust repositories
    Harvest,
    /// Follow a paced batch of prospective developers (25-40 default, 5-10s delay)
    Follow {
        #[arg(short, long, default_value_t = 30)]
        max: usize,
    },
    /// Prune non-mutual accounts that haven't followed back after a grace period (default 7 days)
    Prune {
        #[arg(short, long, default_value_t = 7)]
        days: u64,
        #[arg(short, long, default_value_t = 30)]
        max: usize,
    },
    /// Sync local cache and timestamps with live GitHub following/followers lists
    Sync,
    /// Execute the complete daily maintenance cycle (Prune non-mutuals -> Harvest -> Paced Follow)
    Daily {
        #[arg(short, long, default_value_t = 30)]
        follow_batch: usize,
        #[arg(short, long, default_value_t = 7)]
        prune_days: u64,
        #[arg(short, long, default_value_t = 30)]
        prune_batch: usize,
    },
    /// Run as an autonomous continuous background daemon
    Daemon {
        #[arg(short, long, default_value_t = 24)]
        interval_hours: u64,
        #[arg(short, long, default_value_t = 30)]
        batch: usize,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct UserProfile {
    login: String,
    followers: u64,
    following: u64,
    public_repos: u64,
}

#[derive(Debug, Deserialize)]
struct RateLimitResponse {
    resources: Resources,
}

#[derive(Debug, Deserialize)]
struct Resources {
    core: RateLimitCore,
}

#[derive(Debug, Deserialize)]
struct RateLimitCore {
    limit: u64,
    remaining: u64,
    reset: u64,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
    #[serde(rename = "type")]
    user_type: Option<String>,
}

fn get_config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("github")
}

fn get_token() -> Result<String, String> {
    let token_path = get_config_dir().join("token");
    if token_path.exists() {
        if let Ok(token) = fs::read_to_string(&token_path) {
            let t = token.trim().to_string();
            if !t.is_empty() {
                return Ok(t);
            }
        }
    }
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let t = token.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    Err("No GitHub token found in ~/.config/github/token or GITHUB_TOKEN environment variable.".to_string())
}

fn get_cache_file() -> PathBuf {
    get_config_dir().join("followed_users.json")
}

fn get_history_file() -> PathBuf {
    get_config_dir().join("follow_history.json")
}

fn get_pool_file() -> PathBuf {
    get_config_dir().join("candidate_pool.json")
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_cache() -> HashSet<String> {
    let path = get_cache_file();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(set) = serde_json::from_str::<HashSet<String>>(&data) {
                return set;
            }
        }
    }
    HashSet::new()
}

fn save_cache(cache: &HashSet<String>) {
    let path = get_cache_file();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let mut list: Vec<_> = cache.iter().cloned().collect();
    list.sort();
    if let Ok(data) = serde_json::to_string_pretty(&list) {
        let _ = fs::write(path, data);
    }
}

fn load_history() -> HashMap<String, u64> {
    let path = get_history_file();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, u64>>(&data) {
                return map;
            }
        }
    }
    let cache = load_cache();
    let now = current_timestamp();
    let mut map = HashMap::new();
    for user in cache {
        map.insert(user, now);
    }
    map
}

fn save_history(history: &HashMap<String, u64>) {
    let path = get_history_file();
    let _ = fs::create_dir_all(path.parent().unwrap());
    if let Ok(data) = serde_json::to_string_pretty(history) {
        let _ = fs::write(path, data);
    }
}

fn load_pool() -> Vec<String> {
    let path = get_pool_file();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&data) {
                return list;
            }
        }
    }
    Vec::new()
}

fn save_pool(pool: &[String]) {
    let path = get_pool_file();
    let _ = fs::create_dir_all(path.parent().unwrap());
    if let Ok(data) = serde_json::to_string_pretty(pool) {
        let _ = fs::write(path, data);
    }
}

fn create_client(token: &str) -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("token {}", token)).unwrap(),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github.v3+json"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("gh-network-rust/0.2.0"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
}

fn extract_rate_remaining(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let token = match get_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    let client = create_client(&token);

    match cli.command.unwrap_or(Commands::Status) {
        Commands::Status => show_status(&client).await?,
        Commands::Harvest => harvest_pool(&client).await?,
        Commands::Follow { max } => follow_pipeline(&client, max).await?,
        Commands::Prune { days, max } => prune_non_mutuals(&client, days, max).await?,
        Commands::Sync => sync_network(&client).await?,
        Commands::Daily { follow_batch, prune_days, prune_batch } => {
            run_daily_cycle(&client, follow_batch, prune_days, prune_batch).await?
        }
        Commands::Daemon { interval_hours, batch } => run_daemon(&client, interval_hours, batch).await?,
    }

    Ok(())
}

async fn show_status(client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.get("https://api.github.com/user").send().await?;
    let user: UserProfile = resp.json().await?;

    let rate_resp = client.get("https://api.github.com/rate_limit").send().await?;
    let rate: RateLimitResponse = rate_resp.json().await?;

    let cache = load_cache();
    let history = load_history();
    let pool = load_pool();

    let now = current_timestamp();
    let reset_min = if rate.resources.core.reset > now {
        (rate.resources.core.reset - now) / 60
    } else {
        0
    };

    println!("{}", "==================================================".cyan());
    println!("  {}  @{}", "GitHub Profile:".bold(), user.login.green().bold());
    println!("  {}     {}", "Followers:".bold(), user.followers.to_string().yellow());
    println!("  {}     {}", "Following:".bold(), user.following.to_string().yellow());
    println!("  {}  {}", "Public Repos:".bold(), user.public_repos.to_string().cyan());
    println!("{}", "--------------------------------------------------".cyan());
    println!(
        "  {}     {}/{} (resets in {}m)",
        "Rate Budget:".bold(),
        rate.resources.core.remaining.to_string().green(),
        rate.resources.core.limit,
        reset_min
    );
    println!("  {}   {} tracked developers", "Follow History:".bold(), history.len());
    println!("  {}   {} active cache", "Active Following:".bold(), cache.len());
    println!("  {}   {} targeted candidates", "Candidate Pool:".bold(), pool.len());
    println!("{}", "==================================================".cyan());

    Ok(())
}

async fn fetch_all_followers(client: &reqwest::Client) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let mut followers = HashSet::new();
    let mut page = 1;
    loop {
        let url = format!("https://api.github.com/user/followers?per_page=100&page={}", page);
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            break;
        }
        let users: Vec<GithubUser> = resp.json().await?;
        if users.is_empty() {
            break;
        }
        for u in users {
            followers.insert(u.login);
        }
        page += 1;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(followers)
}

async fn fetch_all_following(client: &reqwest::Client) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let mut following = HashSet::new();
    let mut page = 1;
    loop {
        let url = format!("https://api.github.com/user/following?per_page=100&page={}", page);
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            break;
        }
        let users: Vec<GithubUser> = resp.json().await?;
        if users.is_empty() {
            break;
        }
        for u in users {
            following.insert(u.login);
        }
        page += 1;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(following)
}

async fn harvest_pool(client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Harvesting niche Wayland & Rust developers...".cyan().bold());
    let cache = load_cache();
    let followers = fetch_all_followers(client).await.unwrap_or_default();
    let mut candidates = HashSet::new();

    let pb = ProgressBar::new(TARGET_REPOS.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} repos ({msg})")?
            .progress_chars("#>-"),
    );

    for repo in TARGET_REPOS {
        pb.set_message(repo.to_string());

        for page in 1..=2 {
            let url = format!("https://api.github.com/repos/{}/stargazers?per_page=100&page={}", repo, page);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(users) = resp.json::<Vec<GithubUser>>().await {
                    for u in users {
                        if u.user_type.as_deref().unwrap_or("User") == "User"
                            && !cache.contains(&u.login)
                            && !followers.contains(&u.login)
                        {
                            candidates.insert(u.login);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        pb.inc(1);
    }
    pb.finish_with_message("Repo harvest complete");

    for user in INFLUENTIAL_USERS {
        for page in 1..=2 {
            let url = format!("https://api.github.com/users/{}/followers?per_page=100&page={}", user, page);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(users) = resp.json::<Vec<GithubUser>>().await {
                    for u in users {
                        if u.user_type.as_deref().unwrap_or("User") == "User"
                            && !cache.contains(&u.login)
                            && !followers.contains(&u.login)
                        {
                            candidates.insert(u.login);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let pool: Vec<_> = candidates.into_iter().collect();
    save_pool(&pool);

    println!(
        "{} Saved {} precision candidates to local pool.",
        "Success:".green().bold(),
        pool.len().to_string().yellow().bold()
    );

    Ok(())
}

async fn follow_pipeline(client: &reqwest::Client, max_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut cache = load_cache();
    let mut history = load_history();
    let mut pool = load_pool();

    let mut available: Vec<String> = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
    if available.is_empty() {
        println!("{}", "Candidate pool empty. Harvesting fresh targets...".yellow());
        harvest_pool(client).await?;
        pool = load_pool();
        available = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
    }

    println!(
        "{} Paced follow execution for up to {} accounts (5-10s safe delay)...",
        "Executing Follow Batch:".cyan().bold(),
        max_count.to_string().green()
    );

    let mut rng = rand::thread_rng();
    let mut followed = 0;

    for login in available.iter().take(max_count) {
        let follow_url = format!("https://api.github.com/user/following/{}", login);
        let resp = client
            .put(&follow_url)
            .header(CONTENT_LENGTH, "0")
            .body("")
            .send()
            .await?;

        let status = resp.status();
        let headers = resp.headers().clone();
        let rem_str = extract_rate_remaining(&headers)
            .map(|r| r.to_string())
            .unwrap_or_else(|| "?".to_string());

        if status.as_u16() == 204 || status.as_u16() == 200 {
            // Shadow throttle detection verification
            if followed == 0 || followed % 10 == 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let verify_resp = client.get(&follow_url).send().await?;
                if verify_resp.status().as_u16() != 204 {
                    println!(
                        "{} GitHub daily mutation threshold reached. Pausing batch to preserve account health.",
                        "Notice:".yellow().bold()
                    );
                    break;
                }
            }

            followed += 1;
            let now = current_timestamp();
            cache.insert(login.clone());
            history.insert(login.clone(), now);

            println!(
                "  [{}/{}] Followed @{} (Remaining Rate: {})",
                followed.to_string().green(),
                max_count,
                login.cyan().bold(),
                rem_str.yellow()
            );
        } else if status.as_u16() == 403 || status.as_u16() == 429 {
            println!("{} Rate limit reached (HTTP {}). Stopping batch.", "Rate Limit:".red().bold(), status);
            break;
        } else {
            println!("  [-] Skipped @{} (HTTP {})", login, status);
            cache.insert(login.clone());
        }

        save_cache(&cache);
        save_history(&history);
        let remaining_pool: Vec<String> = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
        save_pool(&remaining_pool);

        // Controlled 5.0 - 10.0 second delay to emulate genuine human activity
        let delay_ms = rng.gen_range(5000..10000);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    println!("\n{} Completed follow batch (+{} developers).", "Done:".green().bold(), followed);
    Ok(())
}

async fn prune_non_mutuals(
    client: &reqwest::Client,
    days: u64,
    max_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{} Scanning following list for non-mutual accounts older than {} days...",
        "Pruning Non-Mutuals:".cyan().bold(),
        days.to_string().yellow()
    );

    let mut cache = load_cache();
    let mut history = load_history();
    let followers = fetch_all_followers(client).await?;
    let live_following = fetch_all_following(client).await?;

    let cutoff = current_timestamp().saturating_sub(days * 86400);
    let mut non_mutual_candidates: Vec<String> = Vec::new();

    for user in &live_following {
        if !followers.contains(user) {
            let followed_at = history.get(user).copied().unwrap_or(0);
            if followed_at <= cutoff {
                non_mutual_candidates.push(user.clone());
            }
        }
    }

    println!(
        "  Found {} non-mutual accounts eligible for pruning (Limit: {})...",
        non_mutual_candidates.len().to_string().yellow(),
        max_count.to_string().cyan()
    );

    let mut rng = rand::thread_rng();
    let mut pruned = 0;

    for user in non_mutual_candidates.iter().take(max_count) {
        let unfollow_url = format!("https://api.github.com/user/following/{}", user);
        let resp = client.delete(&unfollow_url).send().await?;
        let status = resp.status();

        if status.as_u16() == 204 || status.as_u16() == 200 || status.as_u16() == 404 {
            pruned += 1;
            cache.remove(user);
            history.remove(user);
            println!("  [{}/{}] Unfollowed @{}", pruned.to_string().red(), max_count, user.yellow());
        } else {
            println!("  [-] Failed to unfollow @{} (HTTP {})", user, status);
        }

        save_cache(&cache);
        save_history(&history);

        let delay_ms = rng.gen_range(2000..4000);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    println!("\n{} Pruned {} non-mutual accounts successfully.", "Done:".green().bold(), pruned);
    Ok(())
}

async fn sync_network(client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Synchronizing live following and followers lists...".cyan().bold());
    let live_following = fetch_all_following(client).await?;
    let mut history = load_history();
    let now = current_timestamp();

    for user in &live_following {
        history.entry(user.clone()).or_insert(now);
    }
    history.retain(|user, _| live_following.contains(user));

    save_cache(&live_following);
    save_history(&history);

    println!(
        "{} Synced {} active following relationships to local cache and history.",
        "Success:".green().bold(),
        live_following.len().to_string().yellow()
    );

    Ok(())
}

async fn run_daily_cycle(
    client: &reqwest::Client,
    follow_batch: usize,
    prune_days: u64,
    prune_batch: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "==================================================".cyan());
    println!("{}", "Starting Scheduled GitHub Daily Network Maintenance".green().bold());
    println!("{}", "==================================================".cyan());

    // 1. Sync live status
    let _ = sync_network(client).await;

    // 2. Prune old non-mutuals
    let _ = prune_non_mutuals(client, prune_days, prune_batch).await;

    // 3. Harvest if pool is low
    let pool = load_pool();
    let cache = load_cache();
    let available_count = pool.iter().filter(|u| !cache.contains(*u)).count();
    if available_count < follow_batch * 2 {
        let _ = harvest_pool(client).await;
    }

    // 4. Follow paced batch
    let _ = follow_pipeline(client, follow_batch).await;

    println!("{}", "==================================================".cyan());
    println!("{}", "Daily Maintenance Cycle Finished Successfully".green().bold());
    println!("{}", "==================================================".cyan());
    show_status(client).await?;

    Ok(())
}

async fn run_daemon(
    client: &reqwest::Client,
    interval_hours: u64,
    batch_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{} Starting autonomous background daemon (Cycle: {}h, Batch: {})...",
        "Daemon:".cyan().bold(),
        interval_hours.to_string().yellow(),
        batch_size.to_string().green()
    );

    loop {
        let _ = run_daily_cycle(client, batch_size, 7, 30).await;

        let sleep_duration = Duration::from_secs(interval_hours * 3600);
        println!(
            "{} Cycle complete. Sleeping for {}h until next automated execution.",
            "Daemon:".cyan().bold(),
            interval_hours.to_string().yellow()
        );

        tokio::time::sleep(sleep_duration).await;
    }
}

