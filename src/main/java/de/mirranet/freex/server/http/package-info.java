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

/**
 * The web server and the JSON API under {@code /api}, which FreeX Web and the FreeX apps use.
 * <p>
 * {@link de.mirranet.freex.server.http.WebServer} routes requests, authenticates them and sets
 * the security headers; {@link de.mirranet.freex.server.http.Exchange} wraps one request. The
 * endpoints are grouped by topic: {@code AuthApi} (sign-in, accounts, users), {@code MediaApi}
 * (libraries, browsing, playback), {@code ExtrasApi} (metadata, collections, playlists,
 * watchlist, Watch Together), {@code ProfileApi} (profile picture, preferences, history),
 * {@code SettingsApi}, {@code ActivityApi} and {@code CastApi} (Chromecast).
 * <p>
 * Every endpoint that returns titles checks that the user may see their library: titles of
 * libraries that are not shared answer as if they did not exist.
 */
package de.mirranet.freex.server.http;
