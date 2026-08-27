//! Diagnostic: discover Spotify Connect receivers on the LAN and hand one an
//! account, then check whether Spotify's device list picks it up.
//!
//!   cargo run --example connect_probe            # just discover
//!   cargo run --example connect_probe -- "House" # discover and activate

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let wanted = std::env::args().nth(1);

    println!("browsing for _spotify-connect._tcp ...");
    let receivers = fastpotify::zeroconf::discover(Duration::from_secs(4))?;
    if receivers.is_empty() {
        println!("  none found");
        return Ok(());
    }
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    for receiver in &receivers {
        print!(
            "  {} at {}:{}",
            receiver.name, receiver.address, receiver.port
        );
        match fastpotify::zeroconf::get_info(&http, receiver) {
            Ok(info) => println!(
                "  [{} {} | token={} | active_user={:?}]",
                info.remote_name, info.device_type, info.token_type, info.active_user
            ),
            Err(error) => println!("  (getInfo failed: {error})"),
        }
    }

    let Some(wanted) = wanted else { return Ok(()) };
    let Some(receiver) = receivers
        .iter()
        .find(|r| r.name.to_lowercase().contains(&wanted.to_lowercase()))
    else {
        println!("no receiver matching {wanted:?}");
        return Ok(());
    };

    let dirs = fastpotify::paths::AppDirs::discover();
    let credentials = fastpotify::zeroconf::Credentials::load(&dirs.credentials_dir())?;
    println!("\nhanding the account to {} ...", receiver.name);
    let info = fastpotify::zeroconf::get_info(&http, receiver)?;
    match fastpotify::zeroconf::add_user(&http, receiver, &info, &credentials, "Fastpotify") {
        Ok(()) => println!("  accepted"),
        Err(error) => {
            println!("  refused: {error}");
            return Ok(());
        }
    }

    println!("\nwaiting for it to appear in Spotify's device list ...");
    for attempt in 1..=10 {
        std::thread::sleep(Duration::from_secs(2));
        let listed = devices()?;
        if !listed.is_empty() {
            for name in &listed {
                println!("  [{attempt}] {name}");
            }
            return Ok(());
        }
        println!("  [{attempt}] still empty");
    }
    Ok(())
}

/// The account's devices as Spotify currently sees them.
fn devices() -> anyhow::Result<Vec<String>> {
    let home = std::env::var("HOME")?;
    let token: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!(
        "{home}/.local/state/fastpotify/web_api_token.json"
    ))?)?;
    let http = reqwest::blocking::Client::new();
    let body: serde_json::Value = http
        .get("https://api.spotify.com/v1/me/player/devices")
        .bearer_auth(token["access_token"].as_str().unwrap_or_default())
        .send()?
        .json()?;
    Ok(body["devices"]
        .as_array()
        .map(|devices| {
            devices
                .iter()
                .map(|d| {
                    format!(
                        "{} ({}, active={})",
                        d["name"].as_str().unwrap_or("?"),
                        d["type"].as_str().unwrap_or("?"),
                        d["is_active"]
                    )
                })
                .collect()
        })
        .unwrap_or_default())
}
