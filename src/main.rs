use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_LENGTH, USER_AGENT};
use serde::{Deserialize, Serialize};

const TARGET_REPOS: &[&str] = &[
    "hyprwm/Hyprland",
    "Alexays/Waybar",
    "elkowar/eww",
    "Aylur/ags",
    "alacritty/alacritty",
    "neovim/neovim",
    "zed-industries/zed",
    "fastfetch-cli/fastfetch",
    "rust-windowing/winit",
    "rust-lang/rust",
    "astral-sh/uv",
    "bevyengine/bevy",
    "BurntSushi/ripgrep",
    "sharkdp/fd",
    "sharkdp/bat",
    "casey/just",
    "eza-community/eza",
    "dandavison/delta",
    "tauri-apps/tauri",
    "denoland/deno",
    "oven-sh/bun",
    "musescore/MuseScore",
    "audacity/audacity",
    "x42/libltc",
    "mixxxdj/mixxx",
    "electron/electron",
    "vercel/next.js",
    "facebook/react-native",
    "vitejs/vite",
];

const INFLUENTIAL_USERS: &[&str] = &[
    "vaxerski",
    "JohnMwendwa",
    "sharkdp",
    "BurntSushi",
    "antirez",
    "mitsuhiko",
    "dhh",
    "kripken",
    "Aylur",
    "elkowar",
];

#[derive(Parser)]
#[command(name = "gh-network", about = "High-performance GitHub developer network accelerator", version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show current profile metrics, rate limits, and throttle status
    Status,
    /// Harvest prospective developers asynchronously across target repositories
    Harvest,
    /// Follow a batch of prospective developers from the candidate pool
    Follow {
        #[arg(short, long, default_value_t = 100)]
        max: usize,
    },
    /// Sync local cache with live GitHub following list
    Sync,
    /// Run as an autonomous background daemon executing follow cycles every 24h
    Daemon {
        #[arg(short, long, default_value_t = 24)]
        interval_hours: u64,
        #[arg(short, long, default_value_t = 350)]
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

fn get_pool_file() -> PathBuf {
    get_config_dir().join("candidate_pool.json")
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
        HeaderValue::from_static("gh-network-rust/0.1.0"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
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
        Commands::Sync => sync_following(&client).await?,
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
    let pool = load_pool();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
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
    println!("  {}   {} developers", "Follow Cache:".bold(), cache.len());
    println!("  {}   {} candidates ready", "Pool Cache:".bold(), pool.len());
    println!("{}", "==================================================".cyan());

    Ok(())
}

async fn harvest_pool(client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Harvesting active developers asynchronously...".cyan().bold());
    let cache = load_cache();
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
                        if u.user_type.as_deref().unwrap_or("User") == "User" && !cache.contains(&u.login) {
                            candidates.insert(u.login);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }

        pb.inc(1);
    }
    pb.finish_with_message("Harvest complete");

    for user in INFLUENTIAL_USERS {
        for page in 1..=3 {
            let url = format!("https://api.github.com/users/{}/followers?per_page=100&page={}", user, page);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(users) = resp.json::<Vec<GithubUser>>().await {
                    for u in users {
                        if u.user_type.as_deref().unwrap_or("User") == "User" && !cache.contains(&u.login) {
                            candidates.insert(u.login);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    let pool: Vec<_> = candidates.into_iter().collect();
    save_pool(&pool);

    println!(
        "{} Saved {} unique prospective developers to candidate pool.",
        "Success:".green().bold(),
        pool.len().to_string().yellow().bold()
    );

    Ok(())
}

async fn follow_pipeline(client: &reqwest::Client, max_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut cache = load_cache();
    let mut pool = load_pool();

    let mut available: Vec<String> = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
    if available.is_empty() {
        println!("{}", "Candidate pool empty. Running harvest first...".yellow());
        harvest_pool(client).await?;
        pool = load_pool();
        available = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
    }

    println!(
        "{} Targeting up to {} accounts from {} candidates...",
        "Executing Follow Pipeline:".cyan().bold(),
        max_count.to_string().green(),
        available.len().to_string().yellow()
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
            // Shadow throttle detection check
            if followed == 0 || followed % 25 == 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let verify_resp = client.get(&follow_url).send().await?;
                if verify_resp.status().as_u16() != 204 {
                    println!(
                        "{} GitHub daily mutation cooldown (shadow throttle) active. Pausing follow pipeline.",
                        "Notice:".yellow().bold()
                    );
                    println!(
                        "  GitHub has throttled new follow writes for today (~500/day limit reached). Account remains fully healthy."
                    );
                    break;
                }
            }

            followed += 1;
            cache.insert(login.clone());
            println!(
                "  [{}/{}] Followed @{} (Rate Budget: {})",
                followed.to_string().green(),
                max_count,
                login.cyan(),
                rem_str.yellow()
            );
        } else if status.as_u16() == 403 || status.as_u16() == 429 {
            println!("{} Rate limit reached (HTTP {}). Pausing.", "Rate Limit:".red().bold(), status);
            break;
        } else {
            println!("  [-] Skip @{} (HTTP {})", login, status);
            cache.insert(login.clone());
        }

        if followed % 10 == 0 {
            save_cache(&cache);
            let remaining_pool: Vec<String> = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
            save_pool(&remaining_pool);
        }

        let delay_ms = rng.gen_range(850..1350);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    save_cache(&cache);
    let remaining_pool: Vec<String> = pool.iter().filter(|u| !cache.contains(*u)).cloned().collect();
    save_pool(&remaining_pool);

    println!("\n{} Followed +{} developers in this batch.", "Completed:".green().bold(), followed);
    show_status(client).await?;

    Ok(())
}

async fn sync_following(client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Syncing live GitHub following list...".cyan().bold());
    let mut live_following = HashSet::new();
    let mut page = 1;

    loop {
        let url = format!("https://api.github.com/user/following?per_page=100&page={}", page);
        let resp = client.get(&url).send().await?;
        let users: Vec<GithubUser> = resp.json().await?;
        if users.is_empty() {
            break;
        }
        for u in users {
            live_following.insert(u.login);
        }
        page += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    save_cache(&live_following);
    println!(
        "{} Synced {} active followed developers to local cache.",
        "Success:".green().bold(),
        live_following.len().to_string().yellow()
    );

    Ok(())
}

async fn run_daemon(
    client: &reqwest::Client,
    interval_hours: u64,
    batch_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{} Starting autonomous 24h follow daemon (Interval: {}h, Batch: {})...",
        "Daemon:".cyan().bold(),
        interval_hours.to_string().yellow(),
        batch_size.to_string().green()
    );

    loop {
        let pool = load_pool();
        let cache = load_cache();
        let available_count = pool.iter().filter(|u| !cache.contains(*u)).count();

        // If candidate pool is low, automatically harvest new developers
        if available_count < batch_size * 2 {
            println!(
                "{} Candidate pool low ({} remaining). Harvesting fresh targets...",
                "Daemon:".cyan().bold(),
                available_count.to_string().yellow()
            );
            let _ = harvest_pool(client).await;
        }

        println!(
            "{} Executing scheduled daily follow cycle...",
            "Daemon:".green().bold()
        );
        let _ = follow_pipeline(client, batch_size).await;

        let sleep_duration = Duration::from_secs(interval_hours * 3600);
        println!(
            "{} Cycle complete. Sleeping for {}h until next automated execution.",
            "Daemon:".cyan().bold(),
            interval_hours.to_string().yellow()
        );

        tokio::time::sleep(sleep_duration).await;
    }
}

