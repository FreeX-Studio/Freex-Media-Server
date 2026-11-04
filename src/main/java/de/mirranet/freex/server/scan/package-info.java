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
 * Finding media in the library folders.
 * <p>
 * {@link de.mirranet.freex.server.scan.LibraryScanner} walks the folders, recognises movies,
 * shows, seasons and episodes from folder and file names ({@code NameParser}), reads local
 * {@code .nfo} files and artwork, probes files with ffprobe ({@code MediaProbe}) and replaces the
 * library's index. {@code ThumbnailService} extracts preview frames with ffmpeg.
 */
package de.mirranet.freex.server.scan;
