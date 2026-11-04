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

package de.mirranet.freex.server.http;

/** Ends a request with the given HTTP status and a message shown in the web interface. */
public final class ApiException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    public final int status;

    public ApiException(int status, String message) {
        super(message);
        this.status = status;
    }

    public static ApiException notFound() {
        return new ApiException(404, "Nicht gefunden");
    }

    public static ApiException badRequest(String message) {
        return new ApiException(400, message);
    }
}
