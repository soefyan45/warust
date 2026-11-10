# Pihak ketiga / Third-party notices

Kode warust berlisensi [MIT](LICENSE). Bagian di bawah ini milik pihak lain dan
memakai lisensinya masing-masing.

## Disertakan di repo ini

| Bagian | Lokasi | Lisensi | Pemilik |
|---|---|---|---|
| Inter (font) | `assets/fonts/Inter-*.ttf` | OFL-1.1, [teks](assets/fonts/Inter-LICENSE.txt) | The Inter Project Authors |
| JetBrains Mono (font) | `assets/fonts/JetBrainsMono-Regular.ttf` | OFL-1.1, [teks](assets/fonts/JetBrainsMono-LICENSE.txt) | The JetBrains Mono Project Authors |
| Noto Color Emoji (font) | `assets/fonts/NotoColorEmoji.ttf` | OFL-1.1, [teks](assets/fonts/NotoColorEmoji-LICENSE.txt) | Google LLC |
| Lucide (ikon) | `assets/icons/*.svg` | ISC, [teks](assets/icons/LICENSE.txt) | Lucide Icons and Contributors |
| Wallpaper chat | `assets/wallpaper/*.webp` | bagian dari warust (MIT) | dibuat untuk warust dengan bantuan ChatGPT |
| Logo dan ikon aplikasi | `assets/app/*` | bagian dari warust (MIT) | Soefyan Syah |

Bentuk emoji adalah gaya Noto (Google), bukan emoji WhatsApp.

## Library (lewat Cargo, tidak disalin ke repo)

- [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust), MIT,
  Copyright (c) 2025 João Lucas de Oliveira Lopes. Dipakai lewat fork
  [crmne/whatsapp-rust](https://github.com/crmne/whatsapp-rust) pada commit yang dipatok di `Cargo.toml`.
- [egui / eframe](https://github.com/emilk/egui), MIT OR Apache-2.0.
- [axum](https://github.com/tokio-rs/axum), [tokio](https://tokio.rs), [ureq](https://github.com/algesten/ureq),
  [image](https://github.com/image-rs/image), [regex](https://github.com/rust-lang/regex),
  [qrcode](https://github.com/kennytm/qrcode-rust) dan crate lain di `Cargo.lock`: semuanya berlisensi
  permisif (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0 dan sejenisnya). Crate
  dengan lisensi ganda yang juga menawarkan GPL/LGPL dipakai di bawah pilihan MIT/Apache-nya.

Daftar lengkap bisa dibuat dengan `cargo metadata` atau alat seperti `cargo about`.

## Inspirasi

[ZapFast](https://github.com/crmne/zapfast) (MIT, Copyright (c) 2026 Carmine Paolino), klien WhatsApp
desktop dengan Rust dan egui, menjadi rujukan pola untuk beberapa bagian: penyiapan bot, QR pairing
yang digambar sebagai vektor, jendela tanpa bilah judul sistem, dan pengiriman media.

## Merek dagang

WhatsApp adalah merek dagang WhatsApp LLC. warust tidak berafiliasi dengan, didukung oleh, atau
terhubung dengan WhatsApp maupun Meta.
