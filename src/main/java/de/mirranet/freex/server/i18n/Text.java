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

import java.util.Arrays;

/**
 * A message for the user that is translated only when it is sent: a key of the message catalog
 * and its arguments. Arguments may be texts themselves, they are translated into the same language.
 */
public record Text(String key, Object... args) {

    /** Shown when an error has no translation of its own, such as a message from the operating system. */
    public static Text raw(String message) {
        return new Text("error.detail", message == null ? "" : message);
    }

    public String in(String language) {
        return Messages.get(language, this);
    }

    /** The English text, for log messages. */
    @Override
    public String toString() {
        return Messages.get(Messages.DEFAULT, this);
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof Text t && t.key.equals(key) && Arrays.equals(t.args, args);
    }

    @Override
    public int hashCode() {
        return 31 * key.hashCode() + Arrays.hashCode(args);
    }
}
