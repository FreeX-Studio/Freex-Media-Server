# Security

FreeX Media Server is meant to be reachable from the internet. It is designed to be secure without extra setup.

## Sign-in

- **Passwords** are stored as PBKDF2-HMAC-SHA256 hashes with 210 000 iterations. The minimum length is configurable.
- **Two-factor sign-in** with TOTP, compatible with Aegis, 2FAS, Google/Microsoft Authenticator, 1Password and
  others, set up with a QR code. Ten one-time recovery codes are issued; codes cannot be replayed.
- **Brute-force protection:** after 20 failures from one address within 10 minutes that address has to wait. After
  five consecutive failures for one account, every further attempt waits twice as long, up to 15 minutes. Known and
  unknown account names get the same answer in the same time.
- **Sessions** per device with address and last use. Users can end single devices or all others; changing the
  password or switching on two-factor sign-in signs out all other devices. Admins can reset passwords and two-factor
  sign-in of other users.
- **Login history** with failed attempts per account.

## Requests

- Session tokens are 256-bit random values; only their SHA-256 hash is stored. The cookie is `HttpOnly`,
  `SameSite=Strict` and `Secure` over HTTPS, and is only accepted for `GET` requests (images, video, subtitles).
- Everything that changes state needs the `X-FreeX-Token` header and a matching `Origin`, which blocks cross-site
  request forgery.
- Strict Content-Security-Policy (only the server's own scripts, plus Google's Cast SDK), `X-Frame-Options: DENY`,
  `X-Content-Type-Options: nosniff`, same-origin resource policy, HSTS over HTTPS.
- Versions of server and web client are only reported to signed-in users.

## Administration

- Saving server settings requires the current password. Paths through which the server runs programs or keeps its
  state can only be changed in `config.yml`, not in the browser. Secrets are never sent to the browser.
- Profile pictures are decoded and re-encoded by the server; metadata and anything else hidden in an uploaded file
  is dropped, and image dimensions are checked before decoding.
- Chromecast receives a token per title that expires; it unlocks nothing else.

## Files and service

- The database and configuration are readable only by the server user.
- The Debian package runs the server as the unprivileged user `freex` in a hardened systemd service
  (`ProtectSystem=strict`, `ProtectHome=read-only`, `NoNewPrivileges`, private `/tmp`).

## Reporting a vulnerability

Please report security problems privately to niklas.linz@mirranet.de and do not open a public issue. You will get an
answer as soon as possible.
