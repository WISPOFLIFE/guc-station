//! Structured data from specific portal and CMS pages: the weekly schedule,
//! exam seats, quiz grades, attendance and CMS course content. Each parser
//! targets the markup GUC actually serves and returns an empty result rather
//! than failing when a page has nothing to show.

use crate::client::{GucClient, Result};
use crate::page::{self, Page, Table, clean};
use scraper::{ElementRef, Html, Selector};
use url::Url;

// ---------------------------------------------------------------- schedule

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub group: String,
    pub room: String,
    /// e.g. "CSEN 701"
    pub code: String,
    /// "Lecture", "Tut", "Lab", ...
    pub kind: String,
    /// Full course name from the cell's tooltip, when there is one.
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub name: String,
    /// One entry per period; empty means a free slot.
    pub slots: Vec<Vec<Class>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schedule {
    pub days: Vec<Day>,
    /// "8:15-9:45" etc., one per period.
    pub periods: Vec<String>,
}

impl Schedule {
    pub fn is_empty(&self) -> bool {
        self.days.iter().all(|d| d.slots.iter().all(Vec::is_empty))
    }
}

pub fn schedule(p: &Page) -> Schedule {
    let doc = Html::parse_document(&p.html);
    let mut days = Vec::new();
    for row in doc.select(&sel(r#"tr[id*="Xrw"]"#)) {
        let mut cells = row.children().filter_map(ElementRef::wrap).filter(|c| c.value().name() == "td");
        let Some(day) = cells.next() else { continue };
        let slots = cells
            .map(|cell| {
                cell.select(&sel("table tr"))
                    .filter_map(|tr| {
                        let tds: Vec<ElementRef> =
                            tr.children().filter_map(ElementRef::wrap).filter(|c| c.value().name() == "td").collect();
                        if tds.len() < 3 {
                            return None; // the "Free" label table has a single cell
                        }
                        let course = clean(&tds[2]);
                        let (code, kind) = split_course_kind(&course);
                        Some(Class {
                            group: clean(&tds[0]),
                            room: clean(&tds[1]),
                            code,
                            kind,
                            title: tds[2].value().attr("title").unwrap_or_default().trim().to_string(),
                        })
                    })
                    .collect()
            })
            .collect();
        days.push(Day { name: clean(&day), slots });
    }

    // The page lists period times twice: other faculties first, then
    // Engineering / Management / BI. The second set is the common one.
    let text: String = doc.root_element().text().collect::<Vec<_>>().join(" ");
    let times: Vec<String> = text
        .split_whitespace()
        .filter(|w| {
            let mut parts = w.split('-');
            matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if is_clock(a) && is_clock(b))
        })
        .map(str::to_string)
        .collect();
    let per_day = days.iter().map(|d| d.slots.len()).max().unwrap_or(0);
    let periods = if times.len() >= per_day * 2 && per_day > 0 {
        times[per_day..per_day * 2].to_vec()
    } else {
        times.into_iter().take(per_day).collect()
    };
    Schedule { days, periods }
}

/// "CSEN 701 Lecture" -> ("CSEN 701", "Lecture")
fn split_course_kind(s: &str) -> (String, String) {
    match s.trim().rsplit_once(' ') {
        Some((code, kind)) if code.chars().any(|c| c.is_ascii_digit()) => (code.trim().to_string(), kind.to_string()),
        _ => (s.trim().to_string(), String::new()),
    }
}

fn is_clock(s: &str) -> bool {
    s.split_once(':').is_some_and(|(h, m)| {
        !h.is_empty() && h.len() <= 2 && m.len() == 2 && h.chars().chain(m.chars()).all(|c| c.is_ascii_digit())
    })
}

// ---------------------------------------------------------------- exams

#[derive(Debug, Clone, PartialEq)]
pub struct Exam {
    pub code: String,
    pub course: String,
    pub day: String,
    pub date: Option<chrono::NaiveDate>,
    pub date_text: String,
    pub start: String,
    pub end: String,
    pub hall: String,
    pub seat: String,
    pub kind: String,
}

pub fn exams(p: &Page) -> Vec<Exam> {
    let Some((t, h)) = find_table(p, &["seat"]) else { return Vec::new() };
    let col = |name: &str| h.iter().position(|c| c.contains(name));
    let (course, day, date, start, end, hall, seat, kind) =
        (col("course"), col("day"), col("date"), col("start"), col("end"), col("hall"), col("seat"), col("type"));
    rows_after_header(t)
        .map(|r| {
            let get = |c: Option<usize>| c.and_then(|c| r.get(c)).cloned().unwrap_or_default();
            // "MET Digital Media Engineering 7th Semester - DMET702 Visualization and Animation - Winter 2026"
            let full = get(course);
            let parts: Vec<&str> = full.split(" - ").map(str::trim).collect();
            let middle = if parts.len() >= 3 { parts[parts.len() - 2] } else { full.as_str() };
            let (code, name) = middle.split_once(' ').unwrap_or((middle, ""));
            let date_text = get(date);
            let compact: String = date_text.split_whitespace().filter(|w| *w != "-").collect::<Vec<_>>().join(" ");
            Exam {
                code: spaced_code(code),
                course: name.to_string(),
                day: get(day),
                date: chrono::NaiveDate::parse_from_str(&compact, "%d %B %Y").ok(),
                date_text,
                start: short_time(&get(start)),
                end: short_time(&get(end)),
                hall: get(hall),
                seat: get(seat),
                kind: get(kind),
            }
        })
        .collect()
}

/// "DMET702" -> "DMET 702", to match how the schedule writes codes.
fn spaced_code(code: &str) -> String {
    match code.find(|c: char| c.is_ascii_digit()) {
        Some(i) if i > 0 && !code[..i].ends_with(' ') => format!("{} {}", &code[..i], &code[i..]),
        _ => code.to_string(),
    }
}

/// "11:00:00 AM" -> "11:00"; "1:00:00 PM" -> "13:00".
fn short_time(s: &str) -> String {
    chrono::NaiveTime::parse_from_str(s.trim(), "%I:%M:%S %p")
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|_| s.trim().to_string())
}

// ---------------------------------------------------------------- grades

#[derive(Debug, Clone, PartialEq)]
pub struct Quiz {
    /// "Quiz 1", "Assignment 2"
    pub name: String,
    /// "Question1"
    pub element: String,
    pub score: Option<f32>,
    pub max: Option<f32>,
    pub staff: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CourseGrades {
    pub course: String,
    pub quizzes: Vec<Quiz>,
    /// Midterm percentage, when the portal has published one.
    pub midterm: Option<String>,
}

pub fn quizzes(p: &Page) -> Vec<Quiz> {
    let Some((t, h)) = find_table(p, &["element name"]) else { return Vec::new() };
    let col = |name: &str| h.iter().position(|c| c.contains(name));
    let (name, element, grade, staff) = (col("quiz"), col("element"), col("grade"), col("prof"));
    let mut last_name = String::new();
    rows_after_header(t)
        .map(|r| {
            let get = |c: Option<usize>| c.and_then(|c| r.get(c)).cloned().unwrap_or_default();
            // Later questions of the same quiz leave the name cell empty.
            let n = get(name);
            if !n.is_empty() {
                last_name = n;
            }
            let g = get(grade);
            let (score, max) = match g.split_once('/') {
                Some((a, b)) => (a.trim().parse().ok(), b.trim().parse().ok()),
                None => (g.trim().parse().ok(), None),
            };
            Quiz { name: last_name.clone(), element: get(element), score, max, staff: get(staff) }
        })
        .collect()
}

/// Rows of the "Mid-Term Results" table: (course, percentage).
pub fn midterms(p: &Page) -> Vec<(String, String)> {
    p.tables
        .iter()
        .filter(|t| t.id.as_deref().is_some_and(|id| id.ends_with("midDg")))
        .flat_map(|t| t.rows.iter().skip(1))
        .filter(|r| r.len() >= 2 && !r[0].is_empty())
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect()
}

/// Picks every course in the grades dropdown in turn and collects its grades.
pub async fn all_grades(client: &GucClient, url: &str) -> Result<Vec<CourseGrades>> {
    let first = page::fetch(client, url).await?;
    let mids = midterms(&first);
    let mut out = Vec::new();
    if let Some((name, choices)) = courses_in_dropdown(&first) {
        for (value, label) in choices {
            let picked = first.post_back(client, &name, &[(name.clone(), value)]).await?;
            let mut mids_here = midterms(&picked);
            mids_here.extend(mids.iter().cloned());
            let midterm = mids_here.into_iter().find(|(c, _)| course_matches(c, &label)).map(|(_, pct)| pct);
            out.push(CourseGrades { course: label, quizzes: quizzes(&picked), midterm });
        }
    }
    Ok(out)
}

/// True when two course labels refer to the same course code.
fn course_matches(a: &str, b: &str) -> bool {
    let code = |s: &str| -> String { s.split_whitespace().find(|w| w.chars().any(|c| c.is_ascii_digit()) && w.len() >= 6).unwrap_or("").to_string() };
    let a = code(a).replace(' ', "");
    !a.is_empty() && b.replace(' ', "").contains(&a)
}

// ---------------------------------------------------------------- attendance

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub attended: bool,
    /// "Tutorial", "Lecture", "Lab"
    pub kind: String,
    pub date: Option<chrono::NaiveDate>,
    pub slot: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CourseAttendance {
    pub course: String,
    pub sessions: Vec<Session>,
    /// The portal's warning level for this course, if any.
    pub warning: Option<String>,
}

impl CourseAttendance {
    pub fn absences(&self) -> usize {
        self.sessions.iter().filter(|s| !s.attended).count()
    }
}

pub fn sessions(p: &Page) -> Vec<Session> {
    let Some((t, h)) = find_table(p, &["sessiondsc"]) else { return Vec::new() };
    let col = |name: &str| h.iter().position(|c| c.contains(name));
    let (status, desc) = (col("attendance"), col("sessiondsc"));
    rows_after_header(t)
        .map(|r| {
            let get = |c: Option<usize>| c.and_then(|c| r.get(c)).cloned().unwrap_or_default();
            // "W26 - CSEN 701 - Embedded System Architecture - 7DMET T029 Tutorial @2026.09.17 - Regular - Slot2 - 2h"
            let d = get(desc);
            let (before, after) = d.split_once('@').unwrap_or((d.as_str(), ""));
            let kind = before.trim().rsplit(' ').next().unwrap_or("").to_string();
            let date_part = after.split_whitespace().next().unwrap_or("");
            let slot = after.split(" - ").find(|s| s.trim().starts_with("Slot")).unwrap_or("").trim().to_string();
            Session {
                attended: !get(status).to_lowercase().contains("absent"),
                kind,
                date: chrono::NaiveDate::parse_from_str(date_part, "%Y.%m.%d").ok(),
                slot,
            }
        })
        .collect()
}

/// Rows of the absence-warning table: (course, level).
pub fn absence_warnings(p: &Page) -> Vec<(String, String)> {
    let Some((t, h)) = find_table(p, &["absencelevel"]) else { return Vec::new() };
    let col = |name: &str| h.iter().position(|c| c.contains(name));
    let (code, name, level) = (col("code"), col("name"), col("absencelevel"));
    rows_after_header(t)
        .map(|r| {
            let get = |c: Option<usize>| c.and_then(|c| r.get(c)).cloned().unwrap_or_default();
            (format!("{} {}", get(code), get(name)).trim().to_string(), get(level))
        })
        .filter(|(c, l)| !c.is_empty() && !l.is_empty())
        .collect()
}

pub async fn all_attendance(client: &GucClient, url: &str) -> Result<Vec<CourseAttendance>> {
    let first = page::fetch(client, url).await?;
    let warnings = absence_warnings(&first);
    let mut out = Vec::new();
    if let Some((name, choices)) = courses_in_dropdown(&first) {
        for (value, label) in choices {
            let picked = first.post_back(client, &name, &[(name.clone(), value)]).await?;
            // "Winter 2026  - CSEN 701 - Embedded System Architecture" -> "CSEN 701 - Embedded System Architecture"
            let course = label.split_once(" - ").map(|(_, rest)| rest.trim().to_string()).unwrap_or(label.clone());
            let warning = warnings.iter().find(|(c, _)| course_matches(c, &course)).map(|(_, l)| l.clone());
            out.push(CourseAttendance { course, sessions: sessions(&picked), warning });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- CMS

#[derive(Debug, Clone, PartialEq)]
pub struct CmsCourse {
    /// "CSEN701"
    pub code: String,
    pub name: String,
    pub season: String,
    pub url: Url,
}

pub fn cms_courses(p: &Page) -> Vec<CmsCourse> {
    let Some((t, h)) = find_table(p, &["seasonid"]) else { return Vec::new() };
    let col = |name: &str| h.iter().position(|c| c == name);
    let (Some(id), Some(sid)) = (col("id"), col("seasonid")) else { return Vec::new() };
    let season = col("season");
    // The name column has no header text; it's the one with "(|CODE|)".
    rows_after_header(t)
        .filter_map(|r| {
            let raw = r.iter().find(|c| c.starts_with("(|"))?;
            let rest = raw.strip_prefix("(|")?;
            let (code, name) = rest.split_once("|)")?;
            // Drop the trailing "(313)" id.
            let name = name.trim();
            let name = name.rsplit_once(" (").map(|(n, _)| n).unwrap_or(name).trim();
            let url = Url::parse(&format!(
                "https://cms.guc.edu.eg/apps/student/CourseViewStn.aspx?id={}&sid={}",
                r.get(id)?,
                r.get(sid)?
            ))
            .ok()?;
            Some(CmsCourse {
                code: code.trim().to_string(),
                name: name.to_string(),
                season: season.and_then(|s| r.get(s)).cloned().unwrap_or_default(),
                url,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CmsItem {
    /// "1 - CSEN701_Lecture00_Course Policy"
    pub title: String,
    /// "Lecture slides", "Tutorial notes", "Project", ...
    pub kind: String,
    pub url: Url,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CmsWeek {
    /// "2026-9-5", the week's start date as the CMS writes it.
    pub week: String,
    pub items: Vec<CmsItem>,
}

pub fn cms_weeks(p: &Page) -> Vec<CmsWeek> {
    let doc = Html::parse_document(&p.html);
    doc.select(&sel("div.weeksdata"))
        .map(|w| {
            let week = w
                .select(&sel("h2"))
                .next()
                .map(|h| clean(&h))
                .map(|t| t.trim_start_matches("Week:").trim().to_string())
                .unwrap_or_default();
            let items = w
                .select(&sel("div.card-body"))
                .filter_map(|body| {
                    let head = body.select(&sel(r#"div[id^="content"]"#)).next()?;
                    let title = head.select(&sel("strong")).next().map(|s| clean(&s)).unwrap_or_default();
                    let all = clean(&head);
                    let kind = all
                        .strip_prefix(title.as_str())
                        .map(|k| k.trim().trim_start_matches('(').trim_end_matches(')').trim().to_string())
                        .unwrap_or_default();
                    let href = body.select(&sel("a[href]")).find_map(|a| {
                        let h = a.value().attr("href")?;
                        (!h.starts_with("javascript") && !h.starts_with('#')).then(|| h.to_string())
                    })?;
                    Some(CmsItem { title, kind, url: p.url.join(&href).ok()? })
                })
                .collect();
            CmsWeek { week, items }
        })
        .filter(|w: &CmsWeek| !w.items.is_empty())
        .collect()
}

// ---------------------------------------------------------------- evaluations

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    /// Course or staff member, as the dropdown lists it.
    pub name: String,
    pub done: bool,
}

/// Picks each entry of an evaluation page's dropdown and reads whether the
/// portal says it was "already evaluated". Picking only loads the form; it
/// never submits anything.
pub async fn evaluation_status(client: &GucClient, url: &str) -> Result<Vec<Evaluation>> {
    let first = page::fetch(client, url).await?;
    let mut out = Vec::new();
    if let Some((name, choices)) = courses_in_dropdown(&first) {
        for (value, label) in choices {
            let picked = first.post_back(client, &name, &[(name.clone(), value)]).await?;
            out.push(Evaluation { name: label, done: is_already_evaluated(&picked.html) });
        }
    }
    Ok(out)
}

pub fn is_already_evaluated(html: &str) -> bool {
    html.contains("already evaluated")
}

// ---------------------------------------------------------------- transcript

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptCourse {
    /// The study-group semester code, e.g. "DME05".
    pub semester: String,
    pub name: String,
    /// GUC numeric grade (0.7 best, 5.0 fail), when graded.
    pub numeric: Option<f32>,
    pub letter: String,
    pub hours: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Season {
    /// "Winter 2025", "Spring 2026 February Makeup", ...
    pub name: String,
    pub courses: Vec<TranscriptCourse>,
    pub gpa: Option<f32>,
    pub hours: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transcript {
    /// Study year shown, e.g. "2025-2026"; empty before one is picked.
    pub year: String,
    /// Seasons that have at least one course.
    pub seasons: Vec<Season>,
    /// e.g. ("Engineering including German Language", 2.6)
    pub cumulative: Option<(String, f32)>,
}

pub fn transcript(p: &Page) -> Transcript {
    let year = p
        .dropdowns
        .first()
        .and_then(|d| {
            let v = d.selected.as_ref()?;
            d.choices.iter().find(|c| &c.value == v && !c.value.is_empty()).map(|c| c.label.clone())
        })
        .unwrap_or_default();

    let mut seasons = Vec::new();
    for t in &p.tables {
        let Some(h) = t.rows.iter().position(|r| {
            let l: Vec<String> = r.iter().map(|c| c.to_lowercase()).collect();
            l.iter().any(|c| c == "course name") && l.iter().any(|c| c == "numeric")
        }) else {
            continue;
        };
        let header: Vec<String> = t.rows[h].iter().map(|c| c.to_lowercase()).collect();
        let col = |name: &str| header.iter().position(|c| c == name);
        let (sem, name, numeric, grade, hours) = (col("semester"), col("course name"), col("numeric"), col("grade"), col("hours"));
        let season_name = if h > 0 { t.rows[0].iter().find(|c| !c.is_empty()).cloned().unwrap_or_default() } else { String::new() };
        let mut season = Season { name: season_name, courses: Vec::new(), gpa: None, hours: None };
        for r in &t.rows[h + 1..] {
            // "" | "Semester GPA in Current Study Group" | 2.41 | "" | 28
            if let Some(at) = r.iter().position(|c| c.to_lowercase().contains("gpa")) {
                let nums: Vec<f32> = r.iter().skip(at + 1).filter_map(|c| c.trim().parse().ok()).collect();
                season.gpa = nums.first().copied().filter(|g| *g > 0.0);
                season.hours = nums.get(1).copied();
                continue;
            }
            let get = |c: Option<usize>| c.and_then(|c| r.get(c)).cloned().unwrap_or_default();
            let course = TranscriptCourse {
                semester: get(sem),
                name: get(name),
                numeric: get(numeric).trim().parse().ok(),
                letter: get(grade),
                hours: get(hours).trim().parse().ok(),
            };
            if !course.name.is_empty() {
                season.courses.push(course);
            }
        }
        if !season.courses.is_empty() {
            seasons.push(season);
        }
    }

    Transcript { year, seasons, cumulative: cumulative_gpa(&p.html) }
}

/// "Current Cumulative GPA for / Engineering / including German Language / 2.6"
fn cumulative_gpa(html: &str) -> Option<(String, f32)> {
    let doc = Html::parse_document(html);
    let pieces: Vec<String> = doc
        .root_element()
        .text()
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .collect();
    let start = pieces.iter().position(|p| p.starts_with("Current Cumulative GPA"))?;
    let mut label = Vec::new();
    for p in &pieces[start + 1..] {
        match p.parse::<f32>() {
            Ok(v) => return Some((label.join(" "), v)),
            Err(_) if label.len() < 4 => label.push(p.clone()),
            Err(_) => return None,
        }
    }
    None
}

/// The study year a date falls in, as the transcript dropdown labels it:
/// September starts a new year, so 28 Oct 2026 is "2026-2027".
pub fn study_year(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let start = if date.month() >= 9 { date.year() } else { date.year() - 1 };
    format!("{start}-{}", start + 1)
}

// ---------------------------------------------------------------- rate limit

/// The transcript page refuses quick repeat requests with "...temporarily
/// paused due to server overload. Please refresh the page again after
/// 00:00:59 Seconds". Returns how many seconds it asks us to wait.
pub fn pause_seconds(html: &str) -> Option<u64> {
    if html.len() > 4096 || !html.contains("temporarily paused") {
        return None;
    }
    let after = &html[html.find("again after")? + "again after".len()..];
    let clock: String = after.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == ':').collect();
    let secs = clock.split(':').try_fold(0u64, |acc, part| part.parse::<u64>().ok().map(|v| acc * 60 + v))?;
    Some(secs.max(1))
}

// ---------------------------------------------------------------- helpers

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

/// The first table whose header row mentions every one of `needles`
/// (lower-case), with its lower-cased header.
fn find_table<'a>(p: &'a Page, needles: &[&str]) -> Option<(&'a Table, Vec<String>)> {
    p.tables.iter().find_map(|t| {
        let header: Vec<String> = t.rows.first()?.iter().map(|c| c.to_lowercase()).collect();
        needles.iter().all(|n| header.iter().any(|c| c.contains(n))).then_some((t, header))
    })
}

fn rows_after_header(t: &Table) -> impl Iterator<Item = &Vec<String>> {
    t.rows.iter().skip(1).filter(|r| r.iter().any(|c| !c.is_empty()))
}

/// Real choices of the page's course dropdown: (field name, [(value, label)]).
fn courses_in_dropdown(p: &Page) -> Option<(String, Vec<(String, String)>)> {
    let d = p.dropdowns.iter().find(|d| d.choices.len() > 1)?;
    let choices = d
        .choices
        .iter()
        .filter(|c| !c.value.is_empty() && c.value != "0")
        .map(|c| (c.value.clone(), c.label.clone()))
        .collect();
    Some((d.name.clone(), choices))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(html: &str, url: &str) -> Page {
        Page::parse(Url::parse(url).unwrap(), html.to_string())
    }

    #[test]
    fn reads_transcript_year() {
        let html = r#"<select name="yr"><option value="">Choose a study year</option><option selected="selected" value="23">2025-2026</option></select>
          <table id="Table4"><tr><td colspan="2"><strong>Winter 2025 </strong></td><td colspan="4"><input type="hidden" name="h" value="67" /></td></tr>
          <tr><td><strong>Semester</strong></td><td><strong>Course Name</strong></td><td><strong>Numeric</strong></td><td><strong>Grade</strong></td><td><strong>Hours</strong></td></tr>
          <tr><td>DME05</td><td>Computer Graphics</td><td>2.3</td><td>B-</td><td>6</td></tr>
          <tr><td>DME05</td><td>Data Base I</td><td>2</td><td>B</td><td>4</td></tr>
          <tr><td></td><td>Semester GPA in Current Study Group</td><td>2.18</td><td></td><td>10</td></tr></table>
          <table id="Table5"><tr><td colspan="2"><strong>Summer 2026 Round 1</strong></td></tr>
          <tr><td>Semester</td><td>Course Name</td><td>Numeric</td><td>Grade</td><td>Hours</td></tr>
          <tr><td></td><td>Semester GPA in Current Study Group</td><td>0</td></tr></table>
          <p>Current Cumulative GPA for <b>Engineering</b> <span>including German Language</span> <b>2.6</b></p>"#;
        let t = transcript(&page(html, "https://apps.guc.edu.eg/student_ext/Grade/Transcript_001.aspx"));
        assert_eq!(t.year, "2025-2026");
        assert_eq!(t.seasons.len(), 1, "empty seasons are dropped");
        let s = &t.seasons[0];
        assert_eq!(s.name, "Winter 2025");
        assert_eq!(s.courses.len(), 2);
        assert_eq!((s.courses[0].numeric, s.courses[0].letter.as_str(), s.courses[0].hours), (Some(2.3), "B-", Some(6.0)));
        assert_eq!((s.gpa, s.hours), (Some(2.18), Some(10.0)));
        assert_eq!(t.cumulative, Some(("Engineering including German Language".into(), 2.6)));
    }

    #[test]
    fn study_year_turns_in_september() {
        let d = |y, m, day| chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(study_year(d(2026, 10, 28)), "2026-2027");
        assert_eq!(study_year(d(2026, 8, 31)), "2025-2026");
    }

    #[test]
    fn reads_pause_message() {
        let html = "We're sorry! Your request to refresh the transcript page has been temporarily paused due to server overload. Please refresh the page again after 00:00:59 Seconds";
        assert_eq!(pause_seconds(html), Some(59));
        assert_eq!(pause_seconds("<html>Transcript</html>"), None);
    }

    #[test]
    fn reads_schedule_grid() {
        let html = r#"<p>8:15-9:45 10:00-11:30</p><p>8:15-9:45 10:00-11:30</p>
        <table id="scdTbl"><tr><td></td><td>First Period</td><td>Second Period</td></tr>
        <tr id="x_Xrw1"><td><strong>Saturday</strong></td>
          <td><table id="Table3"></table><table id="x_XaltTblR1C1"><tr><td><span id="x_XlblR1C1">Free</span></td></tr></table></td>
          <td><table id="Table4"><tr><td>7DMET L001</td><td>H16</td><td background='half.gif' title='Embedded System Architecture'>CSEN 701 Lecture </td></tr></table></td>
        </tr></table>"#;
        let s = schedule(&page(html, "https://apps.guc.edu.eg/student_ext/Scheduling/GroupSchedule.aspx"));
        assert_eq!(s.periods, vec!["8:15-9:45", "10:00-11:30"]);
        assert_eq!(s.days.len(), 1);
        assert_eq!(s.days[0].name, "Saturday");
        assert!(s.days[0].slots[0].is_empty());
        let c = &s.days[0].slots[1][0];
        assert_eq!((c.code.as_str(), c.kind.as_str(), c.room.as_str()), ("CSEN 701", "Lecture", "H16"));
        assert_eq!(c.title, "Embedded System Architecture");
    }

    #[test]
    fn reads_exam_seats() {
        let html = r#"<TABLE id="Table2"><TR><TD><b>Course Name - Season</b></TD><TD><b> Exam Day</b></TD><TD><b> Date</b></TD>
          <TD><b> Start Time</b></TD><TD><b> End Time</b></TD><TD><b>Hall</b></TD><TD><b>Seat</b></TD><TD><b>Exam Type</b></TD></TR>
          <TR><TD>MET Digital Media Engineering 7th Semester - DMET702 Visualization and Animation - Winter 2026 </TD><TD>Sunday</TD>
          <TD>25 - October - 2026</TD><TD>11:00:00 AM</TD><TD>1:00:00 PM</TD><TD>Exam hall 3</TD><TD>H12</TD><TD>Normal</TD></TR></TABLE>"#;
        let e = exams(&page(html, "https://apps.guc.edu.eg/student_ext/Exam/ViewExamSeat_01.aspx"));
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].code.as_str(), e[0].course.as_str()), ("DMET 702", "Visualization and Animation"));
        assert_eq!(e[0].date, chrono::NaiveDate::from_ymd_opt(2026, 10, 25));
        assert_eq!((e[0].start.as_str(), e[0].end.as_str()), ("11:00", "13:00"));
        assert_eq!((e[0].hall.as_str(), e[0].seat.as_str()), ("Exam hall 3", "H12"));
    }

    #[test]
    fn reads_quiz_grades() {
        let html = r#"<TABLE><TR><TD><STRONG>Quiz/Assignment</STRONG></TD><td><strong>Element Name</strong></td><TD><STRONG>Grade</STRONG></TD><td><strong>Prof./Lecturer/TA</strong></td></TR>
          <tr><TD><span>Quiz 1</span></TD><td> Question1 </td><TD> 13.5 / 15 </TD><td> Dina Shalaby </td></tr>
          <tr><TD></TD><td> Question2 </td><TD> 4 / 5 </TD><td> Dina Shalaby </td></tr></TABLE>"#;
        let q = quizzes(&page(html, "https://apps.guc.edu.eg/student_ext/Grade/CheckGrade_01.aspx"));
        assert_eq!(q.len(), 2);
        assert_eq!((q[0].name.as_str(), q[0].score, q[0].max), ("Quiz 1", Some(13.5), Some(15.0)));
        assert_eq!(q[1].name, "Quiz 1", "continuation rows inherit the quiz name");
    }

    #[test]
    fn reads_attendance_sessions() {
        let html = r#"<table><tr><th>RowNumber</th><th>Attendance</th><th>SessionDsc</th></tr>
          <tr><td>1</td><td>Attended</td><td>W26 - CSEN 701 - Embedded System Architecture - 7DMET T029 Tutorial @2026.09.17 - Regular - Slot2 - 2h</td></tr>
          <tr><td>2</td><td>Absent</td><td>W26 - CSEN 701 - Embedded System Architecture - 7DMET T029 Tutorial @2026.09.24 - Regular - Slot2 - 2h</td></tr></table>"#;
        let s = sessions(&page(html, "https://apps.guc.edu.eg/student_ext/Attendance/x.aspx"));
        assert_eq!(s.len(), 2);
        assert!(s[0].attended && !s[1].attended);
        assert_eq!(s[1].kind, "Tutorial");
        assert_eq!(s[1].date, chrono::NaiveDate::from_ymd_opt(2026, 9, 24));
        assert_eq!(s[1].slot, "Slot2");
    }

    #[test]
    fn reads_cms_courses_and_files() {
        let home = r#"<table><tr><th></th><th scope="col">Name</th><th scope="col">Active</th><th scope="col">Season</th><th scope="col">ID</th><th scope="col">SeasonId</th></tr>
          <tr><td><input type="submit" name="go" value="View Course" /></td><td>(|CSEN701|) Embedded System Architecture (313)</td><td>Active</td><td>Winter 2026</td><td>313</td><td>70</td></tr></table>"#;
        let c = cms_courses(&page(home, "https://cms.guc.edu.eg/apps/student/HomePageStn.aspx"));
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].code.as_str(), c[0].name.as_str()), ("CSEN701", "Embedded System Architecture"));
        assert_eq!(c[0].url.as_str(), "https://cms.guc.edu.eg/apps/student/CourseViewStn.aspx?id=313&sid=70");

        let course = r#"<div class="card mb-5 weeksdata"><div class="card-header"><h2 class="text-big">Week: 2026-9-5</h2></div>
          <div class="card mb-4"><div class="card-body"><div id="content271723"><strong>1 - CSEN701_Lecture00_Course Policy</strong> (Lecture slides)</div>
          <div><a href='/Uploads/70/313/x.pdf' id="download" class="btn">Download Content</a></div></div></div></div>"#;
        let w = cms_weeks(&page(course, "https://cms.guc.edu.eg/apps/student/CourseViewStn.aspx?id=313&sid=70"));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].week, "2026-9-5");
        assert_eq!(w[0].items[0].title, "1 - CSEN701_Lecture00_Course Policy");
        assert_eq!(w[0].items[0].kind, "Lecture slides");
        assert_eq!(w[0].items[0].url.as_str(), "https://cms.guc.edu.eg/Uploads/70/313/x.pdf");
    }

    /// Runs every parser over the pages saved by "Save pages for troubleshooting".
    /// Ignored by default because those pages only exist on a signed-in machine:
    /// `cargo test --lib real_pages -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_pages() {
        let dir = dirs::data_local_dir().unwrap().join("GUC Station").join("pages");
        let load = |name: &str, url: &str| page(&std::fs::read_to_string(dir.join(format!("{name}.html"))).unwrap(), url);
        let s = schedule(&load("schedule", "https://apps.guc.edu.eg/student_ext/Scheduling/GroupSchedule.aspx"));
        println!("schedule: {} days, periods {:?}", s.days.len(), s.periods);
        for d in &s.days {
            let classes: Vec<String> = d.slots.iter().map(|sl| sl.iter().map(|c| format!("{} {} @{}", c.code, c.kind, c.room)).collect::<Vec<_>>().join("+")).collect();
            println!("  {:<10} {:?}", d.name, classes);
        }
        assert!(!s.is_empty());
        let e = exams(&load("exams", "https://apps.guc.edu.eg/student_ext/Exam/ViewExamSeat_01.aspx"));
        println!("exams: {}", e.len());
        for x in &e { println!("  {} {} {:?} {}-{} {} {}", x.code, x.course, x.date, x.start, x.end, x.hall, x.seat); }
        assert!(!e.is_empty());
        let q = quizzes(&load("grades_course_picked", "https://apps.guc.edu.eg/student_ext/Grade/CheckGrade_01.aspx"));
        println!("quizzes: {q:?}");
        let c = cms_courses(&load("cms_home", "https://cms.guc.edu.eg/apps/student/HomePageStn.aspx"));
        println!("cms courses: {:?}", c.iter().map(|c| format!("{} {}", c.code, c.name)).collect::<Vec<_>>());
        assert!(!c.is_empty());
        let w = cms_weeks(&load("cms_course", "https://cms.guc.edu.eg/apps/student/CourseViewStn.aspx?id=313&sid=70"));
        println!("cms weeks: {} items: {}", w.len(), w.iter().map(|w| w.items.len()).sum::<usize>());
        for wk in w.iter().take(2) { for i in &wk.items { println!("  [{}] {} ({})", wk.week, i.title, i.kind); } }
        assert!(!w.is_empty());
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        for y in ["2024-2025", "2025-2026", "2026-2027", "2027-2028"] {
            let Ok(html) = std::fs::read_to_string(fixtures.join(format!("transcript_{y}.html"))) else { continue };
            let t = transcript(&page(&html, "https://apps.guc.edu.eg/student_ext/Grade/Transcript_001.aspx"));
            println!("transcript {y}: year={:?} cumulative={:?}", t.year, t.cumulative);
            for s in &t.seasons {
                println!("  {} gpa={:?} hours={:?} courses={}", s.name, s.gpa, s.hours, s.courses.len());
            }
        }
    }
}
