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

package de.mirranet.freex.server.util;

import de.mirranet.freex.server.json.Json;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.logging.Logger;

/** Reading and atomically writing the JSON data files. */
public final class JsonFiles {

    private static final Logger LOG = Logger.getLogger(JsonFiles.class.getName());

    private JsonFiles() {
    }

    /** Parsed content, or the parsed fallback if the file is missing. A broken file is kept as a copy. */
    public static Object read(Path p, String fallback) throws IOException {
        if (!Files.exists(p)) return Json.parse(fallback);
        try {
            return Json.parse(Files.readString(p, StandardCharsets.UTF_8));
        } catch (Json.JsonException e) {
            Path backup = p.resolveSibling(p.getFileName() + ".broken-" + System.currentTimeMillis());
            Files.copy(p, backup);
            LOG.severe("Could not parse " + p + " (" + e.getMessage() + "), saved a copy as " + backup);
            return Json.parse(fallback);
        }
    }

    /** Writes via a temp file and rename, readable for the server user only (POSIX). */
    public static void write(Path target, Object value) throws IOException {
        Files.createDirectories(target.getParent());
        Path tmp = target.resolveSibling(target.getFileName() + ".tmp");
        Files.writeString(tmp, Json.write(value), StandardCharsets.UTF_8);
        try {
            Files.setPosixFilePermissions(tmp, PosixFilePermissions.fromString("rw-------"));
        } catch (UnsupportedOperationException | IOException ignored) {
            // Windows or a file system without POSIX permissions
        }
        try {
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (AtomicMoveNotSupportedException e) {
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING);
        }
    }
}
