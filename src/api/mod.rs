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

//! The JSON API under `/api`, used by FreeX Web and the FreeX apps (see docs/api.md). Each
//! handler checks access itself through [`crate::http::Ctx`].

mod account;
mod admin;
mod extras;
mod lists;
mod media;
mod metadata;
mod playback;
mod profile;
mod server;
mod users;

pub use account::Challenge;
pub use extras::Grants;
pub use server::{SERVER_NAME, VERSION};

use crate::AppState;
use crate::http::{self, ApiError};
use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use std::sync::Arc;

pub fn router(app: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/status", get(server::status))
        .route("/api/setup", post(account::setup))
        .route("/api/login", post(account::login))
        .route("/api/login/verify", post(account::verify_second_factor))
        .route("/api/logout", post(account::logout))
        .route("/api/me", get(account::me))
        .route("/api/me/password", put(account::change_password))
        .route("/api/me/preferences", put(account::preferences))
        .route("/api/me/sessions", get(account::sessions))
        .route("/api/me/sessions/logout-others", post(account::logout_others))
        .route("/api/me/sessions/{sid}", delete(account::end_session))
        .route("/api/me/logins", get(account::logins))
        .route("/api/me/2fa/setup", post(account::two_factor_setup))
        .route("/api/me/2fa/enable", post(account::two_factor_enable))
        .route("/api/me/2fa/disable", post(account::two_factor_disable))
        .route("/api/me/2fa/recovery", post(account::new_recovery_codes))
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/{id}", put(users::update).delete(users::delete))
        .route("/api/libraries", get(server::libraries).post(server::create_library))
        .route("/api/libraries/{id}", put(server::update_library).delete(server::delete_library))
        .route("/api/libraries/{id}/items", get(media::library_items))
        .route("/api/libraries/{id}/hubs", get(media::library_hubs))
        .route("/api/libraries/{id}/scan", post(media::scan_library))
        .route("/api/scan", post(media::scan_all))
        .route("/api/hubs", get(media::hubs))
        .route("/api/search", get(media::search))
        .route("/api/items/{id}", get(media::item))
        .route("/api/items/{id}/children", get(media::children))
        .route("/api/items/{id}/image/{kind}", get(media::image))
        .route("/api/items/{id}/previews", get(media::previews))
        .route("/api/fs", get(media::browse_folders))
        .route("/api/items/{id}/playback", post(playback::start))
        .route("/api/items/{id}/stream", get(playback::stream))
        .route("/api/transcode/{session}/{file}", get(playback::transcode_file))
        .route("/api/transcode/{session}", delete(playback::stop_transcode))
        .route("/api/items/{id}/subtitles/{sid}", get(playback::subtitle))
        .route("/api/items/{id}/progress", put(playback::progress))
        .route("/api/items/{id}/watched", post(playback::watched).delete(playback::unwatched))
        .route("/api/me/history", get(playback::my_history).delete(playback::clear_history))
        .route("/api/me/history/{hid}", delete(playback::remove_history))
        .route("/api/history", get(playback::all_history))
        .route("/api/items/{id}/metadata/search", get(metadata::search))
        .route("/api/metadata/image", get(metadata::search_image))
        .route("/api/items/{id}/metadata/match", post(metadata::pin))
        .route("/api/items/{id}/metadata/refresh", post(metadata::refresh))
        .route("/api/items/{id}/metadata/unmatch", post(metadata::unpin))
        .route("/api/libraries/{id}/metadata", post(metadata::refresh_library))
        .route("/api/extras/status", get(metadata::status))
        .route("/api/people/{pid}", get(metadata::person))
        .route("/api/people/{pid}/image", get(metadata::person_image))
        .route("/api/libraries/{id}/collections", get(lists::library_collections))
        .route("/api/collections", post(lists::create_collection))
        .route("/api/collections/{cid}", get(lists::collection).put(lists::update_collection).delete(lists::delete_collection))
        .route("/api/collections/{cid}/image/{kind}", get(lists::collection_image))
        .route("/api/collections/{cid}/items", post(lists::add_collection_item))
        .route("/api/collections/{cid}/items/{item}", delete(lists::remove_collection_item))
        .route("/api/playlists", get(lists::playlists).post(lists::create_playlist))
        .route("/api/playlists/{pid}", get(lists::playlist).put(lists::update_playlist).delete(lists::delete_playlist))
        .route("/api/playlists/{pid}/items", post(lists::add_playlist_item))
        .route("/api/playlists/{pid}/items/{index}", delete(lists::remove_playlist_item))
        .route("/api/watchlist", get(lists::watchlist))
        .route("/api/watchlist/{id}", put(lists::watchlist_add).delete(lists::watchlist_remove))
        .route(
            "/api/me/avatar",
            put(profile::upload_avatar).delete(profile::delete_avatar).layer(axum::extract::DefaultBodyLimit::max(profile::MAX_UPLOAD)),
        )
        .route("/api/users/{id}/avatar", get(profile::avatar))
        .route("/api/admin/activity", get(profile::activity))
        .route("/api/admin/settings", get(admin::get_settings).put(admin::save_settings))
        .route("/api/admin/restart", post(admin::restart))
        .route("/api/libraries/{id}/markers", post(extras::analyze_markers))
        .route("/api/together", post(extras::create_room))
        .route("/api/together/{code}", get(extras::join_room).delete(extras::leave_room))
        .route("/api/together/{code}/events", get(extras::room_events))
        .route("/api/together/{code}/state", post(extras::room_state))
        .route("/api/items/{id}/cast", post(extras::start_cast))
        .route("/api/cast/{token}/stream", get(extras::cast_stream))
        .route("/api/cast/{token}/hls/{file}", get(extras::cast_hls))
        .route("/api/cast/{token}/subtitles/{sid}", get(extras::cast_subtitle))
        .route("/api/cast/{token}/image/{kind}", get(extras::cast_image))
        .method_not_allowed_fallback(|| async { ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "error.methodNotAllowed").into_response() });
    api.fallback(http::web).layer(axum::middleware::from_fn_with_state(app.clone(), http::context)).with_state(app)
}
