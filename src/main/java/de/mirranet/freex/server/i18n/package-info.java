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
 * Translation of the texts the server sends to users. Handlers create {@link de.mirranet.freex.server.i18n.Text}
 * values (a catalog key and arguments); they are translated into the display language of the user, or
 * the language of the browser, when the response is written.
 */

package de.mirranet.freex.server.i18n;
