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

## Pesan suara (audio)

Untuk merekam dan meng-encode pesan suara:

- [libopus](https://opus-codec.org) (Xiph.Org dan kontributor), BSD-3-Clause, dikompilasi dari source dan
  **ditautkan statis ke dalam binary warust** lewat [opusic-sys](https://github.com/DoumanAsh/opusic-sys)
  (BSD-3-Clause, teks lisensi yang sama). Teks lengkapnya di bawah.
- [ogg](https://github.com/RustAudio/ogg), BSD-3-Clause, Copyright (c) 2016-2017 est31 dan kontributor,
  Copyright (c) 2002-2015 Xiph.org Foundation. Teks lengkapnya di bawah.
- [opus](https://github.com/SpaceManiac/opus-rs) (binding Rust), MIT OR Apache-2.0.
- [cpal](https://github.com/RustAudio/cpal), Apache-2.0, beserta backend-nya:
  [alsa](https://github.com/diwic/alsa-rs) (Apache-2.0/MIT) dan [alsa-sys](https://github.com/diwic/alsa-sys)
  (MIT) di Linux, [coreaudio-rs](https://github.com/RustAudio/coreaudio-rs) (MIT/Apache-2.0) dan binding
  objc2 CoreAudio (Zlib OR Apache-2.0 OR MIT) di macOS, serta
  [dasp_sample](https://github.com/rustaudio/sample) (MIT OR Apache-2.0),
  [mach2](https://github.com/JohnTitor/mach2) (BSD-2-Clause OR MIT OR Apache-2.0, dipakai di bawah MIT/Apache)
  dan [byteorder](https://github.com/BurntSushi/byteorder) (Unlicense OR MIT).
- [cmake](https://github.com/rust-lang/cmake-rs) (MIT OR Apache-2.0), hanya alat build untuk libopus;
  tidak ikut di dalam binary.
- [ksni](https://github.com/iovxw/ksni), Unlicense: ikon tray di Linux (StatusNotifierItem lewat D-Bus).

### libopus

```
Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic,
                    Jean-Marc Valin, Timothy B. Terriberry,
                    CSIRO, Gregory Maxwell, Mark Borgerding,
                    Erik de Castro Lopo, Mozilla, Amazon

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of Internet Society, IETF or IETF Trust, nor the
names of specific contributors, may be used to endorse or promote
products derived from this software without specific prior written
permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Opus is subject to the royalty-free patent licenses which are
specified at:

Xiph.Org Foundation:
https://datatracker.ietf.org/ipr/1524/

Microsoft Corporation:
https://datatracker.ietf.org/ipr/1914/

Broadcom Corporation:
https://datatracker.ietf.org/ipr/1526/
```

### ogg

```
Copyright (c) 2016-2017 est31 <MTest31@outlook.com> and contributors
Copyright (c) 2002-2015 Xiph.org Foundation

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Inspirasi

[ZapFast](https://github.com/crmne/zapfast) (MIT, Copyright (c) 2026 Carmine Paolino), klien WhatsApp
desktop dengan Rust dan egui, menjadi rujukan pola untuk beberapa bagian: penyiapan bot, QR pairing
yang digambar sebagai vektor, jendela tanpa bilah judul sistem, dan pengiriman media.

## Merek dagang

WhatsApp adalah merek dagang WhatsApp LLC. warust tidak berafiliasi dengan, didukung oleh, atau
terhubung dengan WhatsApp maupun Meta.
