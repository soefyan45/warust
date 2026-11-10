<p align="center">
  <img src="assets/app/warust.svg" alt="warust logo" width="88" height="88">
</p>

<h1 align="center">warust</h1>

<p align="center"><strong>A WhatsApp gateway on your desktop: many numbers, one inbox.</strong></p>

<p align="center">
  A Rust + egui desktop app that links WhatsApp numbers as companion devices,
  with a local REST API, signed webhooks and automatic triggers. One process, no browser.
</p>

<p align="center"><strong>English</strong> · <a href="README.id.md">Bahasa Indonesia</a></p>

<p align="center">
  <a href="#features"><strong>Features</strong></a> ·
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#link-a-number"><strong>Link a number</strong></a> ·
  <a href="#rest-api"><strong>REST API</strong></a> ·
  <a href="#webhook"><strong>Webhook</strong></a>
</p>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/built_with-Rust-1D5A3F" alt="Built with Rust"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20%C2%B7%20macOS%20%C2%B7%20Windows-24292F" alt="Linux, macOS, Windows">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3E6A12" alt="MIT license"></a>
</p>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/inbox-dark.png">
  <img src="docs/screenshots/inbox.png" alt="warust inbox: three numbers in one chat list, a conversation open with a doodle wallpaper">
</picture>

<table>
  <tr>
    <td width="38%"><img src="docs/screenshots/status.png" alt="Status page with reactions and comments"></td>
    <td width="24%"><img src="docs/screenshots/phone.png" alt="Phone-sized window, Forest and Monochrome templates"></td>
    <td width="38%"><img src="docs/screenshots/webhook.png" alt="Webhook & API page"></td>
  </tr>
</table>

<p align="center"><sub>The combined inbox, the Status page, a phone-sized window (Forest and Monochrome), and Webhook & API.
All screenshots use sample data.</sub></p>

The interface is in English, with Bahasa Indonesia available in Settings.

## Features

<table>
  <tr>
    <td width="50%" valign="top"><strong>📥 Combined inbox</strong><br>Many numbers in one chat list. Replies always go out from the number that received the chat.</td>
    <td width="50%" valign="top"><strong>🟢 Status & reactions</strong><br>See contacts' statuses from all your numbers, and send reactions and comments as quoted replies.</td>
  </tr>
  <tr>
    <td valign="top"><strong>🔌 Local REST API</strong><br>Send text, photos, videos, documents and stickers over HTTP on 127.0.0.1, with a Bearer API key.</td>
    <td valign="top"><strong>🪝 Signed webhooks</strong><br>Incoming messages, delivery status and session status are sent as JSON with HMAC-SHA256 and retries.</td>
  </tr>
  <tr>
    <td valign="top"><strong>⚡ Automatic triggers</strong><br>Text or regex rules that call your API, then reply to the sender after a 2xx response.</td>
    <td valign="top"><strong>🛡️ Anti-ban queue</strong><br>Per-number delay and jitter; the queue is kept when the app is closed.</td>
  </tr>
</table>

Also: color emoji, sending photos/videos/documents/stickers, voice message recording, in-app PDF preview, ✓✓ delivery ticks,
desktop notifications (click to open the chat), two templates (Forest, and Classic dark/light), and a phone layout when the window
is shrunk below 1100 px.

## Install

Download from [Releases](https://github.com/soefyan45/warust/releases/latest):

| OS | File | How to install |
|---|---|---|
| Linux (Debian/Ubuntu, x86_64) | `warust_<version>_amd64.deb` | `sudo apt install ./warust_<version>_amd64.deb`, then open **warust** from the application menu |
| Windows 10/11 (x86_64) | `warust-<version>-windows-x86_64.zip` | Extract, run `warust.exe` |
| macOS 11+ (Apple Silicon and Intel) | `warust-<version>-macos-universal.dmg` | Open it, drag **warust** to Applications |

The macOS build is signed with a Developer ID and notarized by Apple, so it opens right away.
The Windows build is not signed yet: in SmartScreen click **More info** → **Run anyway**.

Data (session keys, chats, media, settings, API key) is stored in a per-user folder:
`~/.local/share/warust` (Linux), `~/Library/Application Support/warust` (macOS), or
`%APPDATA%\warust` (Windows). Do not share that folder: it contains enough to sign in to the linked
WhatsApp account. You can pick another folder with the `WARUST_DATA` environment variable.
Recorded voice messages are stored in the `voice/` subfolder (Ogg Opus, about 4 KB per second) so they
can be reopened from the chat; warust never deletes them itself, so you can empty that folder if you
need the space.

Some features use tools built into the OS. On Linux, the .deb package suggests `zenity` (file dialogs),
`poppler-utils` (PDF preview), `libnotify-bin` and `gnome-session-canberra` (notifications and sound),
and `ffmpeg` (video thumbnails).
PDF preview on macOS uses the system's own renderer. On Windows it needs poppler (through Scoop); without it, PDFs can
still be opened in another app. Without ffmpeg, videos are still sent and can be played, just without a
thumbnail (on macOS, Quick Look is used instead).

### From source

You need [Rust](https://rustup.rs) (the version is pinned in `rust-toolchain.toml`), a C compiler for SQLite, and
CMake for libopus (voice messages). On Linux you also need the ALSA headers for the microphone (`sudo apt install cmake libasound2-dev`).

```bash
git clone https://github.com/soefyan45/warust && cd warust
cargo run --release
scripts/install-desktop.sh   # Linux, optional: dock and menu icon
```

When run from a project folder that already has a `data/` folder, that folder is used.

## Link a number

1. Open **Numbers → Add number**.
2. On your phone: **WhatsApp → Linked devices → Link a device**, then scan the QR code. You can also use
   a pairing code from the phone number.
3. The number connects. WhatsApp sends the chat history during pairing; other older messages can be
   loaded from the phone with "Load older messages from phone".

One number can be linked to several devices at once, so WhatsApp on your phone keeps working as
usual.

## REST API

Enable it in **Webhook & API → REST API**. The server only listens on `127.0.0.1` (default port 8787), and
every request needs the API key as an `Authorization: Bearer <key>` header.

```bash
curl -X POST http://127.0.0.1:8787/sessions/<id>/messages/text \
  -H 'Authorization: Bearer <API key>' \
  -H 'Content-Type: application/json' \
  -d '{"to": "628123456789", "text": "Hello from warust"}'
```

| Endpoint | Purpose |
|---|---|
| `GET /healthz` | Check that the server is alive (no API key) |
| `GET /sessions` · `POST /sessions` | List numbers and their status · add a new number |
| `GET /sessions/{id}/qr` · `POST /sessions/{id}/pair-code` | QR code or pairing code |
| `POST /sessions/{id}/messages/text` | Send text through the anti-ban queue |
| `POST /sessions/{id}/messages/media` | Photo, video (MP4/MOV), document or sticker from a `path` or `url` |
| `GET /sessions/{id}/messages/{msg_id}` | Delivery status of one message |
| `POST /sessions/{id}/messages/{msg_id}/forward` | Forward a text message to 1-5 recipients (`{"to": ["628123456789", ...]}`), through the anti-ban queue. Media is rejected with `400 not_forwardable` |
| `POST /sessions/{id}/messages/{msg_id}/pin` | Pin or unpin a message, body `{"pinned": true, "duration": "24h"\|"7d"\|"30d"}` (`duration` defaults to `7d`, ignored when unpinning) |
| `GET /sessions/{id}/chats` · `GET /sessions/{id}/statuses` | List of chats and statuses from the last 24 hours |
| `POST /sessions/{id}/chats/{jid}/read` | Mark a chat as read: sends the blue ticks and syncs to the phone. Optional body `{"read": false}` to mark it unread |
| `POST /sessions/{id}/chats/{jid}/archive` | Archive or unarchive: body `{"archived": true}` or `{"archived": false}` |
| `PUT /sessions/{id}/webhook` | Webhook URL specific to this number |
| `POST /sessions/{id}/logout` · `DELETE /sessions/{id}` | Unlink · delete the number |

`{jid}` is the chat JID from `GET /sessions/{id}/chats` (e.g. `628123456789@s.whatsapp.net` or
`120363...@g.us`); the chat list response includes `unread` and `archived`. Both endpoints reply `202`,
and need a connected number (`409 not_connected`) and a chat that already exists (`404 chat_not_found`).
The blue ticks are only sent when **Send read receipts** in Settings is on (default: off).

To reply to (quote) a message, add `"reply_to": "<message id>"` to the `messages/text` body. The id
comes from `message.received`, `GET /sessions/{id}/chats`, or the id returned by an earlier send; the
message must exist in the `to` chat, otherwise the response is `404 reply_not_found`; a message that is
still in the queue (not yet sent) cannot be replied to yet: `409 reply_not_sent`.

Pins are sent to WhatsApp and answered with `202`; a new pin only shows up in the chat once WhatsApp has accepted it.
Only messages that have already been sent (not ones still in the queue) can be pinned.

A message to a number that has never chatted with your number is rejected (`403 cold_contact`) unless
the request includes `"allow_cold": true`. This is deliberate, because messaging cold contacts is a
common reason numbers get banned.

## Webhook

Set the URL in **Webhook & API → Webhook**: one global URL, and another URL per number if you need it. Events:

| Event | When |
|---|---|
| `message.received` | Incoming message |
| `message.status` | Outgoing message sent, delivered, read, or failed |
| `message.edited` | A message was edited |
| `message.deleted` | A message was deleted for everyone |
| `status.received` | A contact posts a status |
| `session.status` | Number connected, disconnected, needs pairing, and so on |

Every request contains `{"event", "session", "delivery_id", "timestamp", "data"}` with the headers
`X-Warust-Event`, `X-Warust-Delivery`, and `X-Warust-Signature: sha256=<hex>`. The signature is the
HMAC-SHA256 of the raw body with your webhook secret:

```python
import hashlib, hmac

def valid(secret: str, body: bytes, signature: str) -> bool:
    expected = "sha256=" + hmac.new(secret.encode(), body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, signature)
```

Non-2xx responses are retried after 5 seconds, 30 seconds, 2 minutes, and 10 minutes.

### Triggers

In the **Triggers** tab, create rules such as *text starts with "REGISTER"* → `POST` to your API with a JSON body
built from a template (`{{teks}}`, `{{pengirim.nomor}}`, `{{pengirim.nama}}`, `{{perangkat.nomor}}`, …).
If the API answers 2xx, warust can reply to the sender, for example *"Thank you {{nama}}, we are
processing your message."*, from the number that received the message. The editor has a test run that shows the body and
reply without sending anything.

> [!WARNING]
> warust is an unofficial client and is not affiliated with WhatsApp or Meta. Unofficial clients
> can violate WhatsApp's terms of service, and numbers can be restricted or banned. Do not use it
> for spam or bulk messages to people who do not know you. Use at your own risk.

## Credits & license

warust is built on [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) (MIT) and
[egui](https://www.egui.rs) (MIT/Apache-2.0), and inspired by
[ZapFast](https://github.com/crmne/zapfast) (MIT). Inter, JetBrains Mono and Noto Color Emoji fonts
(OFL-1.1), [Lucide](https://lucide.dev) icons (ISC). Details are in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

warust's code is licensed under [MIT](LICENSE). WhatsApp is a trademark of WhatsApp LLC.
