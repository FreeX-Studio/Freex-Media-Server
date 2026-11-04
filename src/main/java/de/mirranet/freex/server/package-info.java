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
 * FreeX Media Server: a self-hosted media server for movies and TV shows.
 * <p>
 * {@link de.mirranet.freex.server.FreeXMediaServer} is the entry point. It reads the
 * configuration, wires the services of the sub-packages together and starts the web server;
 * after a settings change it restarts all of them inside the same process.
 * <p>
 * Overview of the packages:
 * <ul>
 *   <li>{@code config} – {@code config.yml}, reading and writing</li>
 *   <li>{@code store} – SQLite database and the library index</li>
 *   <li>{@code model} – libraries, items, media files, users, watch state</li>
 *   <li>{@code scan} – finding and recognising media files</li>
 *   <li>{@code metadata} – online (TMDB) and local metadata</li>
 *   <li>{@code transcode} – how a file is played, ffmpeg conversion to HLS</li>
 *   <li>{@code markers} – detection of intros and end credits</li>
 *   <li>{@code together} – Watch Together rooms</li>
 *   <li>{@code auth} – passwords, two-factor codes, brute-force protection</li>
 *   <li>{@code http} – the web server and the JSON API</li>
 *   <li>{@code json}, {@code util} – small helpers</li>
 * </ul>
 */
package de.mirranet.freex.server;
