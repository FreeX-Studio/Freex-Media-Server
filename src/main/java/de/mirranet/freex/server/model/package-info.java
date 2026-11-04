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
 * The domain model: libraries, items (movies, shows, seasons, episodes) with their media files
 * and tracks, users and their watch state. The classes are plain data holders; persistence
 * lives in {@code store}, the API views are built by their {@code toPublic()} methods.
 */
package de.mirranet.freex.server.model;
