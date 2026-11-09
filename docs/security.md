<p><img src="brand/freex-logo.svg" alt="FreeX" width="56"></p>

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

## Library sharing

Admins decide per user which libraries they may see: all of them (including libraries added later, the default) or
selected ones. The server enforces this on every request, not only in the interface: titles, artwork, streams,
subtitles, search, home screen, collections, playlists, watchlist, watch history, Watch Together and Chromecast all
behave as if libraries that are not shared did not exist. Admins always see everything.

## Parental controls

Admins can give a user an age limit (*Edit → Parental controls*: up to 6, 12, 16 or 18 years). Titles rated above it
are treated like titles of a library that is not shared, in every place listed above. The rating comes from the
online metadata (FSK in Germany, MPAA and TV ratings in the US, the plain ages of other countries) or from the
`<mpaa>`/`<certification>` of a `.nfo` file; seasons, episodes and extras count with their show or movie. Titles
without a rating are hidden unless the admin allows them for that user (useful for home videos). Admins have no
age limit.

## Switching users with a PIN

On a screen that is shared (the living-room TV), *Switch user* in the account menu changes the account without
typing a password: every user except admins can set a PIN of 4 to 8 digits under *Account* (with their password).
Switching only works from a device on which someone is already signed in, so a PIN never opens the server from
outside; wrong PINs count towards the same lockout as wrong passwords and show in the login history. Admins sign in
with their password only, and an account that becomes admin loses its PIN.

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
  state can only be changed in `config.toml`, not in the browser. Secrets are never sent to the browser.
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
