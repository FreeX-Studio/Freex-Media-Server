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

import de.mirranet.freex.server.i18n.MessageException;
import de.mirranet.freex.server.i18n.Text;
import de.mirranet.freex.server.i18n.Translatable;

/**
 * Ends a request with the given HTTP status and a message for the user, translated into the
 * user's display language when the response is written.
 */
public final class ApiException extends MessageException {

    private static final long serialVersionUID = 1L;

    public final int status;

    public ApiException(int status, String key, Object... args) {
        this(status, new Text(key, args));
    }

    public ApiException(int status, Text text) {
        super(text);
        this.status = status;
    }

    /** Passes on the message of another exception, translated if it has a text of its own. */
    public static ApiException of(int status, Throwable cause) {
        return new ApiException(status, Translatable.of(cause));
    }

    public static ApiException notFound() {
        return new ApiException(404, "error.notFound");
    }

    public static ApiException badRequest(String key, Object... args) {
        return new ApiException(400, key, args);
    }
}
