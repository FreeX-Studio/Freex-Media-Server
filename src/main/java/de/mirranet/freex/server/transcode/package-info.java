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
 * Playback: deciding how a file is played and converting it when necessary.
 * <p>
 * {@link de.mirranet.freex.server.transcode.PlaybackPlanner} compares the file with what the
 * client can decode and picks direct play, direct stream or transcoding;
 * {@link de.mirranet.freex.server.transcode.TranscodeManager} keeps one background worker per
 * conversion, which offers a playlist over the whole title ({@code Segments}) and runs ffmpeg
 * wherever the player asks for a segment ({@code TranscodeWorker}); {@code SubtitleService}
 * delivers subtitles as WebVTT.
 */
package de.mirranet.freex.server.transcode;
