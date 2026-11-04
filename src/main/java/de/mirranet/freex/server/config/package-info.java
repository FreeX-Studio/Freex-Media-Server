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
 * The server configuration in {@code config.yml}.
 * <p>
 * {@link de.mirranet.freex.server.config.Configuration} reads the file (a small YAML subset)
 * at start; a running server keeps those values until the next (in-process) restart. {@link de.mirranet.freex.server.config.Settings}
 * describes which settings administrators may change in the web interface, validates them and
 * writes changed values back into the file without touching comments or order.
 */
package de.mirranet.freex.server.config;
