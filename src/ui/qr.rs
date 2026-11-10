//! Pairing QR painted as vectors, sharp at any zoom.

use egui::{Color32, Rect, Ui, pos2, vec2};

/// Quiet zone around the code, in modules. Scanners need it.
const QUIET: usize = 4;

pub struct Qr {
    pub payload: String,
    width: usize,
    dark: Vec<bool>,
}

impl Qr {
    pub fn encode(payload: &str) -> Option<Self> {
        let code = qrcode::QrCode::new(payload.as_bytes()).ok()?;
        let dark = code
            .to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect();
        Some(Self { payload: payload.to_owned(), width: code.width(), dark })
    }

    /// Paints the code centered in `rect`, with a light background and quiet zone.
    pub fn paint(&self, ui: &Ui, rect: Rect, dark: Color32, light: Color32) {
        let total = (self.width + 2 * QUIET) as f32;
        let side = rect.width().min(rect.height());
        // Whole pixels per module keep the edges crisp.
        let module = (side / total).floor().max(1.0);
        let size = module * total;
        let origin = rect.center() - vec2(size, size) / 2.0;
        let painter = ui.painter();
        painter.rect_filled(Rect::from_min_size(origin, vec2(size, size)), 6.0, light);
        for y in 0..self.width {
            for x in 0..self.width {
                if self.dark[y * self.width + x] {
                    let min = pos2(
                        origin.x + (x + QUIET) as f32 * module,
                        origin.y + (y + QUIET) as f32 * module,
                    );
                    painter.rect_filled(Rect::from_min_size(min, vec2(module, module)), 0.0, dark);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Qr;

    #[test]
    fn encodes_a_linking_string() {
        let qr = Qr::encode("2@abc,def,ghi,7").expect("encodes");
        assert!(qr.width >= 21);
        assert_eq!(qr.dark.len(), qr.width * qr.width);
    }
}
