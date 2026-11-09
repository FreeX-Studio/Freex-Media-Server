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

//! HTTPS: the server's certificate and key from PEM files (as Let's Encrypt and most tools
//! write them). The files are read once at start; a renewed certificate takes effect with a
//! restart from the settings page.

use axum_server::tls_rustls::RustlsConfig;
use std::path::Path;
use std::sync::Arc;

/// Reads and checks the files, so a broken certificate stops the start with a clear message.
pub fn load(certificate: &Path, key: &Path) -> Result<RustlsConfig, String> {
    Ok(RustlsConfig::from_config(Arc::new(server_config(certificate, key)?)))
}

pub fn server_config(certificate: &Path, key: &Path) -> Result<rustls::ServerConfig, String> {
    let certs = rustls_pemfile::certs(&mut reader(certificate)?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", certificate.display()))?;
    if certs.is_empty() {
        return Err(format!("{}: no certificate found (expected PEM)", certificate.display()));
    }
    let key = rustls_pemfile::private_key(&mut reader(key)?)
        .map_err(|e| format!("{}: {e}", key.display()))?
        .ok_or_else(|| format!("{}: no private key found (expected PEM, unencrypted)", key.display()))?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("certificate and key do not fit together: {e}"))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn reader(p: &Path) -> Result<std::io::BufReader<std::fs::File>, String> {
    std::fs::File::open(p).map(std::io::BufReader::new).map_err(|e| format!("cannot read {}: {e}", p.display()))
}
