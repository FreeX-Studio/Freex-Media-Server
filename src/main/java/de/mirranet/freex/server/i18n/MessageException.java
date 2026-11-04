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

package de.mirranet.freex.server.i18n;

/** A rejected input or a refused action, with a message for the user. */
public class MessageException extends RuntimeException implements Translatable {

    private static final long serialVersionUID = 1L;

    private final transient Text text;

    public MessageException(String key, Object... args) {
        this(new Text(key, args));
    }

    public MessageException(Text text) {
        super(text.toString());
        this.text = text;
    }

    @Override
    public Text text() {
        return text;
    }
}
