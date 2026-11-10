//! PDF pages drawn by Windows' own PDF engine (Windows.Data.Pdf, the one
//! built into Windows 10 and 11), so viewing needs no extra files. Everything
//! stays in memory; nothing is written to disk.

use std::sync::{Mutex, OnceLock};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions, PdfPageRotation};
use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};

/// An open PDF. Pages are drawn one at a time on request.
pub struct Document {
    inner: PdfDocument,
    /// Each page's width and height in points, as it appears (rotation applied).
    pub pages: Vec<(f32, f32)>,
    /// Pages are drawn one after another rather than at once.
    lock: Mutex<()>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Document({} pages)", self.pages.len())
    }
}

/// Lets WinRT run on any background thread, which the drawing tasks use.
fn init_com() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        // Keeps a process-wide multithreaded apartment alive; the cookie is never released.
        let _ = windows::Win32::System::Com::CoIncrementMTAUsage();
    });
}

/// Whether bytes look like a PDF; files sometimes have a little junk before the header.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

pub fn open(bytes: &[u8]) -> Result<Document, String> {
    if !is_pdf(bytes) {
        return Err("This file isn't a PDF.".into());
    }
    init_com();
    let load = || -> windows::core::Result<Document> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(bytes)?;
        writer.StoreAsync()?.get()?;
        writer.FlushAsync()?.get()?;
        writer.DetachStream()?;
        stream.Seek(0)?;
        let inner = PdfDocument::LoadFromStreamAsync(&stream)?.get()?;
        let mut pages = Vec::new();
        for i in 0..inner.PageCount()? {
            let page = inner.GetPage(i)?;
            let size = page.Size()?;
            let turned = matches!(page.Rotation()?, PdfPageRotation::Rotate90 | PdfPageRotation::Rotate270);
            pages.push(if turned { (size.Height, size.Width) } else { (size.Width, size.Height) });
        }
        Ok(Document { inner, pages, lock: Mutex::new(()) })
    };
    let doc = load().map_err(|e| {
        // Windows reports a password-protected file as a wrong password.
        if e.code().0 as u32 == 0x8007_052B { "This PDF is password-protected, so it can't be shown here.".to_string() } else { format!("Couldn't open this PDF ({}).", code(&e)) }
    })?;
    if doc.pages.is_empty() {
        return Err("This PDF has no pages.".into());
    }
    Ok(doc)
}

/// The error's code. `windows` 0.52's `message()` reads a null buffer when
/// Windows has no text for an error, so it's not used.
fn code(e: &windows::core::Error) -> String {
    format!("error {:#010X}", e.code().0 as u32)
}

/// Draws one page as a PNG `width` pixels wide.
pub fn render(doc: &Document, index: usize, width: u32) -> Result<Vec<u8>, String> {
    init_com();
    let _drawing = doc.lock.lock().unwrap_or_else(|e| e.into_inner());
    let draw = || -> windows::core::Result<Vec<u8>> {
        let page = doc.inner.GetPage(index as u32)?;
        let options = PdfPageRenderOptions::new()?;
        options.SetDestinationWidth(width)?;
        let out = InMemoryRandomAccessStream::new()?;
        page.RenderWithOptionsToStreamAsync(&out, &options)?.get()?;
        let size = out.Size()? as u32;
        let reader = DataReader::CreateDataReader(&out.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut png = vec![0u8; size as usize];
        reader.ReadBytes(&mut png)?;
        Ok(png)
    };
    draw().map_err(|e| format!("Couldn't draw page {} ({}).", index + 1, code(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-page PDF built by hand, the second page landscape.
    fn sample_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 792 612] /Contents 5 0 R >>",
            "<< /Length 44 >>\nstream\n0 0 1 rg 72 72 200 200 re f 1 0 0 rg 300 300 50 50 re f\nendstream",
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    #[test]
    fn opens_and_draws_pages() {
        let doc = open(&sample_pdf()).unwrap();
        assert_eq!(doc.pages.len(), 2);
        let (w, h) = doc.pages[0];
        assert!(h > w, "first page is portrait: {w}x{h}");
        let (w, h) = doc.pages[1];
        assert!(w > h, "second page is landscape: {w}x{h}");

        // Drawn on another thread, as the app does.
        let doc = std::sync::Arc::new(doc);
        let png = std::thread::spawn({
            let doc = doc.clone();
            move || render(&doc, 0, 600)
        })
        .join()
        .unwrap()
        .unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!(image.width(), 600);
        assert!((image.height() as i32 - 776).abs() <= 2, "keeps the page's shape: {}", image.height());
        assert!(render(&doc, 5, 600).is_err(), "no such page");
    }

    #[test]
    fn refuses_files_that_arent_pdfs() {
        assert!(!is_pdf(b"PK\x03\x04 a zip"));
        assert!(is_pdf(b"\xef\xbb\xbf%PDF-1.7 ..."));
        assert_eq!(open(b"<html>not found</html>").unwrap_err(), "This file isn't a PDF.");
        assert!(open(b"%PDF-1.4 broken").is_err());
    }
}
