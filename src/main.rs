/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! FreeX Media Server: your media, your server, your freedom.
//!
//! `freex-media-server [config.toml]` reads the configuration (writing a default one on the
//! first start), opens the database in the data directory and serves the API and FreeX Web.

mod activity;
mod api;
mod auth;
mod config;
mod db;
mod files;
mod http;
mod i18n;
mod logbuf;
mod maintenance;
mod markers;
mod media;
mod metadata;
mod settings;
mod system;
mod tls;
mod together;
mod transcode;
mod webhooks;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Everything the request handlers share.
pub struct AppState {
    pub config: config::Config,
    pub db: db::Db,
    pub guard: auth::LoginGuard,
    /// Sign-ins waiting for their second factor, by hash of the challenge.
    pub challenges: Mutex<HashMap<String, api::Challenge>>,
    /// Serializes the creation of the first admin.
    pub setup_lock: tokio::sync::Mutex<()>,
    /// Compared against for unknown accounts, so a sign-in takes the same time either way.
    pub dummy_hash: String,
    pub web: http::WebBundle,
    pub build: String,
    pub catalog: media::catalog::Catalog,
    pub scanner: media::scanner::Scanner,
    pub thumbs: media::thumbs::Thumbs,
    pub subtitles: media::subtitles::Subtitles,
    pub tools: media::probe::Tools,
    pub metadata: metadata::Service,
    pub activity: activity::Activity,
    /// None when detection is switched off or ffmpeg is missing.
    pub markers: Option<markers::Service>,
    /// HTTP calls on playback events and new titles.
    pub webhooks: webhooks::Webhooks,
    /// The nightly maintenance window.
    pub maintenance: maintenance::Maintenance,
    /// Load of the machine for the dashboard.
    pub system: system::System,
    /// Seek preview thumbnails; None if switched off or ffmpeg is missing.
    pub previews: Option<media::previews::Previews>,
    pub together: Arc<together::Together>,
    pub cast_grants: api::Grants,
    /// The compatibility layer for Jellyfin apps (its key, Quick Connect codes, playbacks).
    pub jellyfin: api::Jellyfin,
    /// None when conversion is switched off or ffmpeg is missing.
    pub transcoder: Option<Arc<transcode::Transcoder>>,
    /// Path of the configuration file the server was started with.
    pub config_path: PathBuf,
    /// Whether the server itself speaks HTTPS.
    pub tls: bool,
    /// Set by the settings page: the server restarts with the configuration file as it is now.
    pub restart: tokio::sync::Notify,
}

/// Why the server loop ended.
enum Outcome {
    Stop,
    Restart,
}

fn main() {
    logbuf::init();
    let path = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("config.toml"));
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        tracing::warn!("TLS provider already installed");
    }
    // A restart from the settings page ends the loop body and starts it again with the new
    // configuration; the process, and so the service, keeps running.
    let mut restarted = false;
    loop {
        match run(path.clone()) {
            Ok(Outcome::Stop) => break,
            Ok(Outcome::Restart) => {
                tracing::info!("Restarting with the configuration as it is now");
                restarted = true;
            }
            Err(e) if restarted && settings::restore_previous(&path) => {
                // A change from the settings page that does not start: back to the configuration before it.
                tracing::error!("{e}");
                tracing::warn!(
                    "Starting again with the previous configuration; the failed one is kept as {}",
                    path.with_extension("toml.failed").display()
                );
                restarted = false;
            }
            Err(e) => {
                tracing::error!("{e}");
                std::process::exit(1);
            }
        }
    }
}

fn run(config_path: PathBuf) -> Result<Outcome, String> {
    let config = config::Config::load(&config_path)?;
    let tls = config.tls_files().map(|(c, k)| tls::load(&c, &k)).transpose()?;
    let data = config.data_dir();
    let cache = config.cache_dir();
    for dir in [&data, &cache] {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let db = db::Db::open(&data.join("freex.db"))?;
    let catalog = media::catalog::Catalog::load(&db);
    let metadata = {
        let t = &config.tmdb;
        // The addresses can be changed for tests against a local stand-in.
        let api = std::env::var("FREEX_TMDB_API").unwrap_or_else(|_| "https://api.themoviedb.org/3".into());
        let images = std::env::var("FREEX_TMDB_IMAGES").unwrap_or_else(|_| "https://image.tmdb.org/t/p".into());
        let tvmaze_api = std::env::var("FREEX_TVMAZE_API").unwrap_or_else(|_| "https://api.tvmaze.com".into());
        let tmdb = t.key().map(|(key, builtin)| {
            if builtin {
                tracing::info!("TMDB: using the key built into this server (tmdb.api_key is empty)");
            }
            metadata::tmdb::Tmdb::new(&api, &images, key, &t.language)
        });
        if tmdb.is_none() {
            tracing::info!("No TMDB key set (tmdb.api_key): only local .nfo files and artwork are used");
        }
        let tvmaze = (t.tvmaze && tmdb.is_some()).then(|| metadata::tmdb::TvMaze::new(&tvmaze_api));
        metadata::Service::new(&db, &data, tmdb, tvmaze, &t.region)
    };
    let web = http::WebBundle::load();
    let addr: SocketAddr = format!("{}:{}", config.server.bind, config.server.port)
        .parse()
        .map_err(|_| format!("server.bind '{}' is not an IP address", config.server.bind))?;

    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime
        .block_on(async move {
            let tools = media::probe::Tools::find(&config.ffmpeg.ffprobe, &config.ffmpeg.ffmpeg).await;
            let ffmpeg = tools.ffmpeg_ok.then(|| tools.ffmpeg.clone());
            let subtitles = media::subtitles::Subtitles::new(&cache, ffmpeg.clone());
            let previews = if !config.previews.enabled {
                tracing::info!("Seek previews are switched off (previews.enabled)");
                None
            } else {
                ffmpeg.clone().map(|f| media::previews::Previews::new(&cache, f, tools.tonemap.software))
            };
            let thumbs = media::thumbs::Thumbs::new(cache, ffmpeg.clone(), tools.tonemap.software);
            let markers = if !config.markers.enabled {
                tracing::info!("Intro/credits detection is switched off (markers.enabled)");
                None
            } else {
                ffmpeg.clone().map(|f| markers::Service::new(&db, f))
            };
            let together = together::Together::new();
            let transcoder = if !config.transcoder.enabled {
                tracing::info!("Conversion is switched off (transcoder.enabled)");
                None
            } else if !tools.ffmpeg_ok {
                None
            } else {
                let ffprobe = tools.ffprobe_ok.then(|| tools.ffprobe.clone());
                let dir = config.transcode_dir();
                match transcode::Transcoder::new(
                    tools.ffmpeg.clone(),
                    ffprobe,
                    dir.clone(),
                    config.transcoder.clone(),
                    tools.tonemap,
                    Some(config.cache_dir().join("keyframes")),
                ) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        tracing::error!("Conversion is off: cannot use {}: {e}", dir.display());
                        None
                    }
                }
            };
            let hooks = webhooks::Webhooks::new(&db);
            let app = Arc::new(AppState {
                build: web.build.clone(),
                web,
                db,
                guard: auth::LoginGuard::default(),
                challenges: Mutex::new(HashMap::new()),
                setup_lock: tokio::sync::Mutex::new(()),
                dummy_hash: auth::hash_password(&auth::token()),
                catalog,
                metadata,
                activity: activity::Activity::default(),
                markers,
                previews,
                system: system::System::new(),
                maintenance: maintenance::Maintenance::default(),
                webhooks: hooks,
                together,
                cast_grants: api::Grants::default(),
                jellyfin: api::Jellyfin::load(&config.data.directory),
                scanner: media::scanner::Scanner::default(),
                thumbs,
                subtitles,
                tools,
                transcoder,
                config,
                config_path,
                tls: tls.is_some(),
                restart: tokio::sync::Notify::new(),
            });
            // Bind first, so a port in use fails right away instead of after the start.
            let listener = std::net::TcpListener::bind(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            tracing::info!(
                "{} {} (FreeX Web {}) listening on {}://{addr}/",
                api::SERVER_NAME,
                api::VERSION,
                app.web.version["version"].as_str().unwrap_or("?"),
                if app.tls { "https" } else { "http" }
            );
            spawn_housekeeping(app.clone());
            fill_age_ratings(&app);
            system::System::spawn(app.clone());
            maintenance::Maintenance::spawn(app.clone());
            media::scanner::Scanner::spawn_timer(app.clone(), app.config.scanner.interval_minutes);
            let dlna = api::dlna::start(&app);
            let handle = axum_server::Handle::new();
            let stopper = handle.clone();
            let restart_wanted = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = restart_wanted.clone();
            let app_for_signal = app.clone();
            tokio::spawn(async move {
                let restart = shutdown_signal(&app_for_signal).await;
                flag.store(restart, std::sync::atomic::Ordering::Relaxed);
                // Open requests get a moment to finish; long streams (SSE, playback) are cut.
                stopper.graceful_shutdown(Some(Duration::from_secs(3)));
            });
            let service = api::router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
            let served = match tls {
                Some(config) => match axum_server::from_tcp_rustls(listener, config) {
                    Ok(server) => server.handle(handle).serve(service).await,
                    Err(e) => Err(e),
                },
                None => match axum_server::from_tcp(listener) {
                    Ok(server) => server.handle(handle).serve(service).await,
                    Err(e) => Err(e),
                },
            }
            .map_err(|e| e.to_string());
            if let Some(info) = &dlna {
                api::dlna::goodbye(info);
            }
            // End running conversions, so no ffmpeg outlives the server.
            if let Some(t) = &app.transcoder {
                t.stop_all();
            }
            served.map(|()| if restart_wanted.load(std::sync::atomic::Ordering::Relaxed) { Outcome::Restart } else { Outcome::Stop })
        })
        .inspect(|o| {
            if matches!(o, Outcome::Stop) {
                tracing::info!("Stopped");
            }
        })
}

/// Indexes written by 0.20 have no age ratings on their entries yet; they are taken from the
/// stored metadata at once instead of waiting for the next scan, so age limits work right away.
fn fill_age_ratings(app: &AppState) {
    let idx = app.catalog.snapshot();
    for lib in app.db.libraries() {
        let missing = idx
            .top_level(&lib.id)
            .iter()
            .any(|it| it.content_rating.is_empty() && app.metadata.get(&it.id).is_some_and(|m| !m.content_rating.is_empty()));
        if missing {
            app.catalog.update_library(&app.db, &lib.id, |items| app.metadata.apply_all(items));
        }
    }
}

/// Removes expired sessions and tidies the database once an hour.
fn spawn_housekeeping(app: Arc<AppState>) {
    tokio::spawn(async move {
        let max_idle = i64::from(app.config.security.session_days) * 86_400_000;
        loop {
            let n = app.db.remove_expired_sessions(max_idle);
            if n > 0 {
                tracing::info!("Removed {n} expired sessions");
            }
            let db = app.clone();
            let _ = tokio::task::spawn_blocking(move || db.db.tidy()).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Ctrl+C everywhere, SIGTERM (systemd) on Unix, or a restart from the settings page (true).
async fn shutdown_signal(app: &AppState) -> bool {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
        () = app.restart.notified() => {
            tracing::info!("Restart requested");
            return true;
        }
    }
    tracing::info!("Shutting down");
    false
}
