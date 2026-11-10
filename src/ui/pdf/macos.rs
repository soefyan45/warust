//! PDF pages through CoreGraphics, macOS's own renderer, so previews need no
//! poppler there.

use std::path::Path;
use std::ptr::NonNull;

use objc2_core_foundation::{CFRetained, CFURL, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGInterpolationQuality, CGPDFBox, CGPDFDocument, CGPDFPage,
};

use super::Info;

/// Tallest picture rendered, so a very long page cannot ask for gigabytes.
const MAX_HEIGHT: u32 = 8192;

/// Opens the file; also PDFs "locked" with an empty password, as Preview does.
fn open(file: &Path) -> Option<CFRetained<CGPDFDocument>> {
    let url = CFURL::from_file_path(file)?;
    let document = CGPDFDocument::with_url(Some(&url))?;
    if !CGPDFDocument::is_unlocked(Some(&document)) {
        // SAFETY: a NUL-terminated string that outlives the call.
        let unlocked = unsafe { CGPDFDocument::unlock_with_password(Some(&document), NonNull::from(c"").cast()) };
        if !unlocked {
            return None;
        }
    }
    Some(document)
}

/// The page's visible size in points as it is shown: its crop box, turned by
/// its `/Rotate`.
fn shown_size(page: &CGPDFPage) -> Option<(f64, f64)> {
    let size = CGPDFPage::box_rect(Some(page), CGPDFBox::CropBox).size;
    if !(size.width > 0.0 && size.height > 0.0) {
        return None;
    }
    let turned = CGPDFPage::rotation_angle(Some(page)).rem_euclid(180) == 90;
    Some(if turned { (size.height, size.width) } else { (size.width, size.height) })
}

pub(super) fn read_info(file: &Path) -> Option<Info> {
    let document = open(file)?;
    let pages = u32::try_from(CGPDFDocument::number_of_pages(Some(&document))).ok().filter(|&n| n > 0)?;
    let first = CGPDFDocument::page(Some(&document), 1)?;
    let (width, height) = shown_size(&first)?;
    Some(Info { pages, aspect: (height / width) as f32 })
}

/// Renders the page (1-based) `width` pixels wide into `target` as a PNG.
pub(super) fn render(file: &Path, page: u32, width: u32, target: &Path) -> bool {
    let done = draw(file, page, width).and_then(|picture| {
        std::fs::create_dir_all(target.parent()?).ok()?;
        // Written beside the target first, so a half-written file is never
        // taken for a finished page.
        let partial = target.with_extension("part");
        picture.save_with_format(&partial, image::ImageFormat::Png).ok()?;
        std::fs::rename(&partial, target).ok()
    });
    if done.is_none() {
        log::warn!("could not render page {page} of {}", file.display());
    }
    done.is_some()
}

fn draw(file: &Path, number: u32, width: u32) -> Option<image::RgbImage> {
    let document = open(file)?;
    let page = CGPDFDocument::page(Some(&document), number as usize)?;
    let (page_width, page_height) = shown_size(&page)?;
    let scale = f64::from(width) / page_width;
    let height = ((page_height * scale).round() as u32).clamp(1, MAX_HEIGHT);
    let space = CGColorSpace::new_device_rgb()?;
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: `pixels` holds height rows of width × 4 bytes and outlives the
    // context, which is dropped before the bytes are read.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            Some(&space),
            CGImageAlphaInfo::NoneSkipLast.0,
        )
    }?;
    let context = Some(&*context);
    // Paper is white; a PDF only paints what is on it.
    CGContext::set_rgb_fill_color(context, 1.0, 1.0, 1.0, 1.0);
    CGContext::fill_rect(context, CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(f64::from(width), f64::from(height))));
    CGContext::set_interpolation_quality(context, CGInterpolationQuality::High);
    CGContext::scale_ctm(context, scale, scale);
    // Puts the crop box, turned by the page's /Rotate, on (0, 0)–(width, height)
    // in points. (It never enlarges, hence the scale above.)
    let shown = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(page_width, page_height));
    CGContext::concat_ctm(context, CGPDFPage::drawing_transform(Some(&page), CGPDFBox::CropBox, shown, 0, true));
    CGContext::draw_pdf_page(context, Some(&page));
    let _ = context;
    // The fourth byte of each pixel is padding.
    let rgba = image::RgbaImage::from_raw(width, height, pixels)?;
    Some(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-page PDF, 200 × 400 pt, the top half of each page red. The
    /// second page is turned a quarter (`/Rotate 90`), so it shows 400 × 200
    /// with the red half on the right.
    fn sample() -> Vec<u8> {
        let content = "1 0 0 rg 0 200 200 200 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 400] /Contents 5 0 R >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 400] /Rotate 90 /Contents 5 0 R >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut pdf = "%PDF-1.4\n".to_owned();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
        for offset in offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
        pdf.into_bytes()
    }

    fn is_red(pixel: &image::Rgb<u8>) -> bool {
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60
    }

    fn is_white(pixel: &image::Rgb<u8>) -> bool {
        pixel.0.iter().all(|&c| c > 240)
    }

    #[test]
    fn reads_and_renders_pages_upright() {
        let dir = std::env::temp_dir().join(format!("warust-cgpdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("contoh.pdf");
        std::fs::write(&file, sample()).unwrap();

        let info = read_info(&file).expect("the sample opens");
        assert_eq!(info.pages, 2);
        assert!((info.aspect - 2.0).abs() < 0.01, "{}", info.aspect);

        // Page 1: portrait, red on top.
        let first = dir.join("out").join("p1-w256.png");
        assert!(render(&file, 1, 256, &first));
        let page = image::open(&first).unwrap().to_rgb8();
        assert_eq!(page.dimensions(), (256, 512));
        assert!(is_red(page.get_pixel(128, 100)), "{:?}", page.get_pixel(128, 100));
        assert!(is_white(page.get_pixel(128, 400)), "{:?}", page.get_pixel(128, 400));

        // Page 2: shown turned, so landscape with the red half on the right.
        let second = dir.join("out").join("p2-w256.png");
        assert!(render(&file, 2, 256, &second));
        let page = image::open(&second).unwrap().to_rgb8();
        assert_eq!(page.dimensions(), (256, 128));
        assert!(is_red(page.get_pixel(200, 64)), "{:?}", page.get_pixel(200, 64));
        assert!(is_white(page.get_pixel(50, 64)), "{:?}", page.get_pixel(50, 64));

        // No such page, and not a PDF.
        assert!(!render(&file, 3, 256, &dir.join("out").join("p3.png")));
        let broken = dir.join("rusak.pdf");
        std::fs::write(&broken, b"bukan pdf").unwrap();
        assert_eq!(read_info(&broken), None);
        assert!(!render(&broken, 1, 256, &dir.join("out").join("rusak.png")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
