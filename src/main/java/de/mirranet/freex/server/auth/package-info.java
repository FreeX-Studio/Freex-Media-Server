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
 * Building blocks for signing in: password hashing (PBKDF2), time-based one-time codes for
 * two-factor authentication (TOTP, RFC 6238) and the guard that slows down password guessing.
 * The sign-in flow itself is in {@code http.AuthApi}.
 */
package de.mirranet.freex.server.auth;
