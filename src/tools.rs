//! Offline calculators and file downloads.

use crate::Client;
use guc::mail::{Attachment, OutFile};
use guc::portal::{self, GradedCourse, number};
use percent_encoding::percent_decode_str;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct GpaRow {
    pub name: String,
    pub hours: String,
    pub grade: String,
}

#[derive(Debug, Clone)]
pub struct GpaCalc {
    pub rows: Vec<GpaRow>,
}

#[derive(Debug, Clone)]
pub enum GpaMsg {
    Name(usize, String),
    Hours(usize, String),
    Grade(usize, String),
    Add,
    Remove(usize),
}

impl Default for GpaCalc {
    fn default() -> Self {
        GpaCalc { rows: vec![GpaRow::default(); 3] }
    }
}

impl GpaCalc {
    pub fn from_courses(courses: &[GradedCourse]) -> Self {
        GpaCalc {
            rows: courses
                .iter()
                .map(|c| GpaRow { name: c.name.clone(), hours: trim_num(c.hours), grade: trim_num(c.grade) })
                .collect(),
        }
    }

    pub fn update(&mut self, m: GpaMsg) {
        match m {
            GpaMsg::Name(i, v) => self.rows[i].name = v,
            GpaMsg::Hours(i, v) => self.rows[i].hours = v,
            GpaMsg::Grade(i, v) => self.rows[i].grade = v,
            GpaMsg::Add => self.rows.push(GpaRow::default()),
            GpaMsg::Remove(i) => {
                self.rows.remove(i);
            }
        }
    }

    pub fn courses(&self) -> Vec<GradedCourse> {
        self.rows
            .iter()
            .filter_map(|r| {
                let (hours, grade) = (number(&r.hours)?, number(&r.grade)?);
                (hours > 0.0 && (0.7..=5.0).contains(&grade)).then(|| GradedCourse { name: r.name.clone(), hours, grade })
            })
            .collect()
    }

    pub fn result(&self) -> Option<(f32, f32)> {
        let c = self.courses();
        Some((portal::gpa(&c)?, c.iter().map(|c| c.hours).sum()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Component {
    pub name: String,
    pub weight: String,
    pub score: String,
    pub out_of: String,
}

#[derive(Debug, Clone)]
pub struct GradeCalc {
    pub parts: Vec<Component>,
}

#[derive(Debug, Clone)]
pub enum CalcMsg {
    Name(usize, String),
    Weight(usize, String),
    Score(usize, String),
    OutOf(usize, String),
    Add,
    Remove(usize),
}

impl Default for GradeCalc {
    fn default() -> Self {
        let part = |name: &str, weight: &str| Component { name: name.into(), weight: weight.into(), ..Default::default() };
        GradeCalc { parts: vec![part("Quizzes", "15"), part("Assignments", "10"), part("Midterm", "25"), part("Final", "50")] }
    }
}

pub struct CalcSummary {
    /// Percentage points of the course total already earned.
    pub earned: f32,
    /// Weight of components that have a score.
    pub graded_weight: f32,
    /// Weight still open.
    pub remaining: f32,
    pub total_weight: f32,
}

impl CalcSummary {
    pub fn best_possible(&self) -> f32 {
        self.earned + self.remaining
    }

    /// Average so far on what has been graded, as a percentage.
    pub fn standing(&self) -> Option<f32> {
        (self.graded_weight > 0.0).then(|| self.earned / self.graded_weight * 100.0)
    }
}

impl GradeCalc {
    pub fn update(&mut self, m: CalcMsg) {
        match m {
            CalcMsg::Name(i, v) => self.parts[i].name = v,
            CalcMsg::Weight(i, v) => self.parts[i].weight = v,
            CalcMsg::Score(i, v) => self.parts[i].score = v,
            CalcMsg::OutOf(i, v) => self.parts[i].out_of = v,
            CalcMsg::Add => self.parts.push(Component::default()),
            CalcMsg::Remove(i) => {
                self.parts.remove(i);
            }
        }
    }

    pub fn summary(&self) -> CalcSummary {
        let mut s = CalcSummary { earned: 0.0, graded_weight: 0.0, remaining: 0.0, total_weight: 0.0 };
        for p in &self.parts {
            let Some(w) = number(&p.weight) else { continue };
            s.total_weight += w;
            match (number(&p.score), number(&p.out_of)) {
                (Some(score), Some(max)) if max > 0.0 => {
                    s.earned += w * (score / max).min(1.0);
                    s.graded_weight += w;
                }
                _ => s.remaining += w,
            }
        }
        s
    }
}

fn trim_num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

const FILE_EXTENSIONS: [&str; 16] =
    ["pdf", "ppt", "pptx", "doc", "docx", "xls", "xlsx", "zip", "rar", "7z", "txt", "mp4", "mkv", "java", "py", "c"];

pub fn is_file_link(url: &Url) -> bool {
    let path = url.path().to_lowercase();
    path.contains("/uploads/")
        || path.rsplit_once('.').is_some_and(|(_, ext)| FILE_EXTENSIONS.contains(&ext))
}

pub fn file_name(url: &Url) -> String {
    let last = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("download");
    let decoded = percent_decode_str(last).decode_utf8_lossy().into_owned();
    sanitize(if decoded.is_empty() { "download" } else { &decoded })
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect::<String>().trim().to_string()
}

/// The name a CMS item is saved under: its title, without the CMS's "7 - "
/// numbering, plus the extension of the file behind it.
/// "7 - Volume_Visualization 1" with a .pdf URL -> "Volume Visualization 1.pdf".
pub fn saved_name(title: &str, url: &Url) -> String {
    let fallback = file_name(url);
    let title = title.trim();
    let title = match title.split_once(" - ") {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => title,
    };
    let stem = sanitize(&title.replace('_', " "));
    let stem = stem.trim_end_matches(['.', ' ']);
    if stem.is_empty() {
        return fallback;
    }
    match fallback.rsplit_once('.') {
        Some((_, ext)) if !stem.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) => format!("{stem}.{ext}"),
        _ => stem.to_string(),
    }
}

/// Where to save `bytes` as `name`: the plain name if it's free or already holds
/// this same file, otherwise "name (2).ext", so two different files that share a
/// title never overwrite each other.
async fn save_path(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(candidate);
        match tokio::fs::read(&path).await {
            Ok(existing) if existing != bytes => n += 1,
            _ => return path,
        }
    }
}

/// Downloads a CMS file with the signed-in session into `<downloads>/<course>/`,
/// named after its title on the CMS.
pub async fn download(client: Client, url: Url, title: String, folder: String, downloads: PathBuf) -> Result<PathBuf, String> {
    let resp = client.0.get(url.as_str()).await.map_err(|e| e.to_string())?;
    let dir = downloads.join(sanitize(&folder));
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let path = save_path(&dir, &saved_name(&title, &resp.url), &resp.bytes).await;
    tokio::fs::write(&path, &resp.bytes).await.map_err(|e| e.to_string())?;
    Ok(path)
}

/// A safe local name for a mail attachment. The name comes from the sender, so
/// it can't be allowed to leave the folder or hit a reserved Windows name.
pub fn attachment_file_name(name: &str) -> String {
    let clean = sanitize(name);
    let clean = clean.trim_end_matches(['.', ' ']).trim_start();
    if clean.chars().all(|c| c == '.') {
        return "attachment".to_string();
    }
    let stem = clean.split('.').next().unwrap_or("").to_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved { format!("_{clean}") } else { clean.to_string() }
}

/// Document and image types that are fine to open straight away. Everything
/// else from a mail (programs, scripts, macro files, shortcuts) is only saved.
pub fn safe_to_open(path: &Path) -> bool {
    const SAFE: [&str; 15] =
        ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "ics", "jpg", "jpeg", "png", "gif", "webp", "bmp"];
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| SAFE.contains(&e.to_lowercase().as_str()))
}

/// Downloads a mail attachment into `<downloads>/Mail/`.
pub async fn save_attachment(client: Client, attachment: Attachment, downloads: PathBuf) -> Result<PathBuf, String> {
    let bytes = guc::mail::attachment_bytes(&client.0, &attachment.id).await.map_err(|e| e.to_string())?;
    // The server does this for some attachments; a 0-byte file would just fail to open.
    if bytes.is_empty() {
        return Err(format!("the mail server has no content for {}, so there's nothing to save.", attachment.name));
    }
    let dir = downloads.join("Mail");
    write_attachment(&dir, &attachment_file_name(&attachment.name), &bytes).await
}

async fn write_attachment(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    let path = save_path(dir, name, bytes).await;
    // An existing file here is this same one, possibly open in another app.
    if !path.exists() {
        tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    }
    mark_from_internet(&path).await;
    Ok(path)
}

/// Sets Windows' "came from the internet" mark, as browsers do, so SmartScreen
/// and Office's Protected View still apply to mail attachments.
async fn mark_from_internet(path: &Path) {
    if cfg!(windows) {
        let mark = format!("{}:Zone.Identifier", path.display());
        let _ = tokio::fs::write(mark, "[ZoneTransfer]\r\nZoneId=3\r\n").await;
    }
}

/// Pages the app doesn't show yet but needs for the schedule and CMS screens.
const EXTRA_PAGES: [(&str, &str); 5] = [
    ("portal_home", guc::portal::PORTAL_HOME),
    ("registered_courses", "https://apps.guc.edu.eg/student_ext/Registration/CourseRegistrationView_001.aspx"),
    ("academic_schedule", "https://apps.guc.edu.eg/student_ext/Scheduling/SearchAcademicScheduled_001.aspx"),
    ("general_group_schedule", "https://apps.guc.edu.eg/student_ext/Scheduling/GeneralGroupSchedule.aspx"),
    ("previous_grades", "https://apps.guc.edu.eg/student_ext/Grade/CheckGradePerviousSemester_01.aspx"),
];

/// Saves every page the app reads (plus a few it will) to the local pages
/// folder. Pages that fail are skipped; the count says how many made it.
pub async fn save_diagnostics(
    client: Client,
    sections: std::collections::HashMap<guc::portal::Section, Url>,
) -> Result<(usize, PathBuf), String> {
    use guc::page;
    let mut saved = 0;
    let mut dir = None;
    let mut keep = |name: &str, html: &str| {
        if let Ok(path) = crate::settings::save_page(name, html) {
            saved += 1;
            dir = path.parent().map(PathBuf::from);
        }
    };

    for (section, url) in &sections {
        let Ok(p) = page::fetch(&client.0, url.as_str()).await else { continue };
        keep(&format!("{section:?}"), &p.html);
        // These pages only show data once something is picked in their dropdown.
        use guc::portal::Section::{Attendance, Grades, Transcript};
        let picks = match section {
            Grades => 1,
            Transcript => 4,
            Attendance => 2,
            _ => 0,
        };
        if let Some(d) = p.dropdowns.first() {
            let real = d.choices.iter().filter(|c| !c.value.is_empty() && c.value != "0");
            for (n, choice) in real.take(picks).enumerate() {
                if let Ok(picked) = p.post_back(&client.0, &d.name, &[(d.name.clone(), choice.value.clone())]).await {
                    keep(&format!("{section:?}_picked_{n}"), &picked.html);
                }
            }
        }
    }
    for (name, url) in EXTRA_PAGES {
        if let Ok(p) = page::fetch(&client.0, url).await {
            keep(name, &p.html);
        }
    }
    if let Ok(cms) = page::fetch(&client.0, guc::portal::CMS_HOME).await {
        keep("cms_home", &cms.html);
        if let Some(course) = cms.links.iter().find(|l| l.href.path().to_lowercase().contains("courseview")) {
            if let Ok(c) = page::fetch(&client.0, course.href.as_str()).await {
                keep("cms_course", &c.html);
            }
        }
    }
    match dir {
        Some(d) if saved > 0 => Ok((saved, d)),
        _ => Err("no page could be loaded".into()),
    }
}


/// Asks for files to attach and reads them. Cancelling the dialog attaches nothing.
pub async fn pick_attachments() -> Result<Vec<OutFile>, String> {
    let Some(files) = rfd::AsyncFileDialog::new().set_title("Attach files").pick_files().await else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut total = 0u64;
    for f in files {
        let name = f.file_name();
        // Checked before reading, so picking a huge file doesn't fill memory.
        total += tokio::fs::metadata(f.path()).await.map(|m| m.len()).unwrap_or(0);
        if total > crate::MAX_ATTACHMENTS_BYTES as u64 {
            return Err("Attachments can add up to 10 MB at most, or GUC's mail server turns the message away.".into());
        }
        let bytes = tokio::fs::read(f.path()).await.map_err(|e| format!("Couldn't read {name}: {e}"))?;
        out.push(OutFile { name, bytes });
    }
    Ok(out)
}

/// Asks for a location for the downloads folder and creates GUC Station
/// inside it. None if the dialog was cancelled.
pub async fn pick_download_folder(start: PathBuf) -> Option<Result<PathBuf, String>> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose where to put the GUC Station folder");
    if let Some(parent) = start.parent().filter(|p| p.exists()) {
        dialog = dialog.set_directory(parent);
    }
    let picked = dialog.pick_folder().await?;
    Some(create_download_folder(crate::settings::station_folder(picked.path())).await)
}

/// Makes sure the folder exists and can be written to before it's used.
pub async fn create_download_folder(dir: PathBuf) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let probe = dir.join(".guc-station-write-check");
    tokio::fs::write(&probe, b"").await.map_err(|e| format!("Can't save files in {}: {e}", dir.display()))?;
    let _ = tokio::fs::remove_file(&probe).await;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_calc_best_case() {
        let mut c = GradeCalc::default();
        c.update(CalcMsg::Score(0, "12".into()));
        c.update(CalcMsg::OutOf(0, "15".into()));
        c.update(CalcMsg::Score(2, "20".into()));
        c.update(CalcMsg::OutOf(2, "25".into()));
        let s = c.summary();
        assert!((s.earned - 32.0).abs() < 1e-4);
        assert!((s.remaining - 60.0).abs() < 1e-4);
        assert!((s.best_possible() - 92.0).abs() < 1e-4);
        assert!((s.standing().unwrap() - 80.0).abs() < 1e-4);
    }

    #[test]
    fn saves_under_the_shown_title() {
        let u = Url::parse("https://cms.guc.edu.eg/Uploads/70/GUC_318_70_2026-09-06T08_51_18.pdf").unwrap();
        assert_eq!(saved_name("Volume Visualization 1", &u), "Volume Visualization 1.pdf");
        assert_eq!(saved_name("7 - Volume Visualization 1", &u), "Volume Visualization 1.pdf");
        assert_eq!(saved_name("3 - CSEN701_Lecture02_Overview: Processors", &u), "CSEN701 Lecture02 Overview_ Processors.pdf");
        assert_eq!(saved_name("Notes.PDF", &u), "Notes.PDF");
        assert_eq!(saved_name("Part 1 - Intro", &u), "Part 1 - Intro.pdf");
        assert_eq!(saved_name("  ", &u), "GUC_318_70_2026-09-06T08_51_18.pdf");
    }

    #[test]
    fn attachment_names_stay_in_their_folder() {
        assert_eq!(attachment_file_name("Report Q1.xlsx"), "Report Q1.xlsx");
        assert_eq!(attachment_file_name("../../evil.exe"), ".._.._evil.exe");
        assert_eq!(attachment_file_name("C:\\Windows\\x.dll"), "C__Windows_x.dll");
        assert_eq!(attachment_file_name(".."), "attachment");
        assert_eq!(attachment_file_name("   "), "attachment");
        assert_eq!(attachment_file_name("trailing. "), "trailing");
        assert_eq!(attachment_file_name("con.txt"), "_con.txt");
        assert_eq!(attachment_file_name("COM1"), "_COM1");
        assert_eq!(attachment_file_name("Computer.pdf"), "Computer.pdf");
    }

    #[test]
    fn only_documents_open_automatically() {
        for ok in ["a.pdf", "b.DOCX", "c.xlsx", "d.jpg", "invite.ics"] {
            assert!(safe_to_open(Path::new(ok)), "{ok}");
        }
        for risky in ["a.exe", "invoice.pdf.exe", "b.js", "c.bat", "d.docm", "e.xlsm", "f.lnk", "g.html", "noextension"] {
            assert!(!safe_to_open(Path::new(risky)), "{risky}");
        }
    }

    #[tokio::test]
    async fn attachments_are_saved_with_the_internet_mark() {
        let dir = std::env::temp_dir().join(format!("guc-station-attach-{}", std::process::id()));
        let first = write_attachment(&dir, "File.pdf", b"one").await.unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(write_attachment(&dir, "File.pdf", b"one").await.unwrap(), first, "same file is reused");
        assert_eq!(write_attachment(&dir, "File.pdf", b"two").await.unwrap(), dir.join("File (2).pdf"));
        if cfg!(windows) {
            let mark = std::fs::read_to_string(format!("{}:Zone.Identifier", first.display())).unwrap();
            assert!(mark.contains("ZoneId=3"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn same_title_never_overwrites_a_different_file() {
        let dir = std::env::temp_dir().join(format!("guc-station-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Quiz.pdf"), b"first").unwrap();
        assert_eq!(save_path(&dir, "Quiz.pdf", b"first").await, dir.join("Quiz.pdf"));
        assert_eq!(save_path(&dir, "Quiz.pdf", b"second").await, dir.join("Quiz (2).pdf"));
        assert_eq!(save_path(&dir, "New.pdf", b"x").await, dir.join("New.pdf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_links() {
        let u = Url::parse("https://cms.guc.edu.eg/Uploads/x/Lecture%201.pdf").unwrap();
        assert!(is_file_link(&u));
        assert_eq!(file_name(&u), "Lecture 1.pdf");
        assert!(!is_file_link(&Url::parse("https://cms.guc.edu.eg/apps/student/CourseViewStn.aspx?id=1").unwrap()));
    }
}
