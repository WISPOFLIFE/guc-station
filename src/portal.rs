//! Where things live on the student portal and how to read GUC grades.
//! Section URLs are discovered from the portal's own menu after sign-in; the
//! hard-coded ones are only a fallback when the menu doesn't mention them.

use crate::page::Page;
use std::collections::HashMap;
use url::Url;

pub const PORTAL_HOME: &str = "https://apps.guc.edu.eg/student_ext/index.aspx";
pub const CMS_HOME: &str = "https://cms.guc.edu.eg/apps/student/HomePageStn.aspx";
pub const MAIL_WEB: &str = "https://mail.guc.edu.eg/owa/";
pub const EWS: &str = "https://mail.guc.edu.eg/EWS/Exchange.asmx";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Notifications,
    Schedule,
    Exams,
    Grades,
    Transcript,
    Attendance,
    Finance,
    CourseEvaluation,
    StaffEvaluation,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::Notifications,
        Section::Schedule,
        Section::Exams,
        Section::Grades,
        Section::Transcript,
        Section::Attendance,
        Section::Finance,
        Section::CourseEvaluation,
        Section::StaffEvaluation,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Notifications => "Notifications",
            Section::Schedule => "Class schedule",
            Section::Exams => "Exam schedule",
            Section::Grades => "Grades",
            Section::Transcript => "Transcript",
            Section::Attendance => "Attendance",
            Section::Finance => "Payments",
            Section::CourseEvaluation => "Evaluate courses",
            Section::StaffEvaluation => "Evaluate staff",
        }
    }

    /// Lower-case fragments that identify this section's menu link (text or URL).
    fn matches(self, text: &str, href: &str) -> bool {
        let has = |s: &str| text.contains(s) || href.contains(s);
        match self {
            Section::Notifications => has("notification"),
            Section::Schedule => href.contains("groupschedule") || (text.contains("schedule") && !has("exam")),
            Section::Exams => has("examseat") || (text.contains("exam") && !has("grade")),
            Section::Grades => href.contains("checkgrade") || (text.contains("grade") && !has("transcript")),
            Section::Transcript => has("transcript"),
            Section::Attendance => has("attendance"),
            Section::Finance => has("financ") || has("balance") || has("payment"),
            Section::CourseEvaluation => has("evaluatecourse") || (text.contains("evaluat") && text.contains("course")),
            Section::StaffEvaluation => has("evaluatestaff") || (text.contains("evaluat") && (text.contains("staff") || text.contains("instructor"))),
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Section::Notifications => "https://apps.guc.edu.eg/student_ext/Main/Notifications.aspx",
            Section::Schedule => "https://apps.guc.edu.eg/student_ext/Scheduling/GroupSchedule.aspx",
            Section::Exams => "https://apps.guc.edu.eg/student_ext/Exam/ViewExamSeat_01.aspx",
            Section::Grades => "https://apps.guc.edu.eg/student_ext/Grade/CheckGrade_01.aspx",
            Section::Transcript => "https://apps.guc.edu.eg/student_ext/Grade/Transcript_001.aspx",
            Section::Attendance => "https://apps.guc.edu.eg/student_ext/Attendance/ClassAttendance_ViewStudentAttendance_001.aspx",
            Section::Finance => "https://apps.guc.edu.eg/student_ext/Financial/BalanceView_001.aspx",
            Section::CourseEvaluation => "https://apps.guc.edu.eg/student_ext/Evaluation/EvaluateCourse.aspx",
            Section::StaffEvaluation => "https://apps.guc.edu.eg/student_ext/Evaluation/EvaluateStaff.aspx",
        }
    }
}

/// Maps each section to a URL, preferring links found on the portal home page.
pub fn discover(home: Option<&Page>) -> HashMap<Section, Url> {
    Section::ALL
        .into_iter()
        .map(|s| {
            let known_path = Url::parse(s.fallback()).unwrap().path().to_lowercase();
            let on_portal = |l: &&crate::page::Link| l.href.host_str() == Some("apps.guc.edu.eg");
            // The known address wins; loose text matching would pick e.g.
            // "Final Exam Excuse" for the exam schedule.
            let found = home.and_then(|p| {
                p.links
                    .iter()
                    .filter(on_portal)
                    .find(|l| l.href.path().to_lowercase() == known_path)
                    .or_else(|| {
                        p.links.iter().filter(on_portal).find(|l| s.matches(&l.text.to_lowercase(), &l.href.path().to_lowercase()))
                    })
            });
            let url = found.map(|l| l.href.clone()).unwrap_or_else(|| Url::parse(s.fallback()).unwrap());
            (s, url)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradedCourse {
    pub name: String,
    pub hours: f32,
    /// GUC numeric grade: 0.7 is best, 5.0 is a fail.
    pub grade: f32,
}

/// Finds rows that look like transcript entries: a table whose header names an
/// hours column and a numeric grade column.
pub fn graded_courses(page: &Page) -> Vec<GradedCourse> {
    let mut out = Vec::new();
    for table in &page.tables {
        let Some(header_at) = table.rows.iter().position(|r| {
            let lower: Vec<String> = r.iter().map(|c| c.to_lowercase()).collect();
            lower.iter().any(|c| c.contains("hour")) && lower.iter().any(|c| c.contains("numeric") || c.contains("grade"))
        }) else {
            continue;
        };
        let header: Vec<String> = table.rows[header_at].iter().map(|c| c.to_lowercase()).collect();
        let col = |pred: &dyn Fn(&str) -> bool| header.iter().position(|c| pred(c));
        let hours_col = col(&|c| c.contains("hour"));
        let grade_col = col(&|c| c.contains("numeric")).or_else(|| col(&|c| c.contains("grade") && !c.contains("letter")));
        let name_col = col(&|c| c.contains("course") || c.contains("name")).unwrap_or(0);
        let (Some(hc), Some(gc)) = (hours_col, grade_col) else { continue };
        for row in &table.rows[header_at + 1..] {
            let (Some(h), Some(g)) = (row.get(hc).and_then(|c| number(c)), row.get(gc).and_then(|c| number(c))) else {
                continue;
            };
            if (0.7..=6.0).contains(&g) && h > 0.0 {
                out.push(GradedCourse { name: row.get(name_col).cloned().unwrap_or_default(), hours: h, grade: g });
            }
        }
    }
    out
}

pub fn gpa(courses: &[GradedCourse]) -> Option<f32> {
    let hours: f32 = courses.iter().map(|c| c.hours).sum();
    (hours > 0.0).then(|| courses.iter().map(|c| c.hours * c.grade).sum::<f32>() / hours)
}

/// GUC's letter for a numeric grade (German scale).
pub fn letter(grade: f32) -> &'static str {
    const SCALE: [(f32, &str); 12] = [
        (0.7, "A+"), (1.0, "A"), (1.3, "A-"), (1.7, "B+"), (2.0, "B"), (2.3, "B-"),
        (2.7, "C+"), (3.0, "C"), (3.3, "C-"), (3.7, "D+"), (4.0, "D"), (5.0, "F"),
    ];
    SCALE.iter().find(|(g, _)| grade <= *g + 0.001).map(|(_, l)| *l).unwrap_or("F")
}

pub fn number(s: &str) -> Option<f32> {
    let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    t.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_transcript_table_and_gpa() {
        let html = r#"<table><tr><th>Course Name</th><th>Numeric</th><th>Grade</th><th>Credit Hours</th></tr>
            <tr><td>Math</td><td>1.0</td><td>A</td><td>6</td></tr>
            <tr><td>Physics</td><td>2.0</td><td>B</td><td>2</td></tr>
            <tr><td>Semester GPA</td><td></td><td></td><td></td></tr></table>"#;
        let p = Page::parse(Url::parse(PORTAL_HOME).unwrap(), html.into());
        let c = graded_courses(&p);
        assert_eq!(c.len(), 2);
        assert_eq!(gpa(&c), Some(1.25));
        assert_eq!(letter(1.25), "A-");
        assert_eq!(letter(0.7), "A+");
    }

    #[test]
    fn discovers_menu_links() {
        let html = r#"<a href="Grade/Transcript_001.aspx">My Transcript</a><a href="Grade/CheckGrade_01.aspx">Grades</a>"#;
        let p = Page::parse(Url::parse(PORTAL_HOME).unwrap(), html.into());
        let map = discover(Some(&p));
        assert_eq!(map[&Section::Transcript].path(), "/student_ext/Grade/Transcript_001.aspx");
        assert_eq!(map[&Section::Grades].path(), "/student_ext/Grade/CheckGrade_01.aspx");
    }

    #[test]
    fn exam_seats_beat_exam_excuses() {
        // Menu order on the real portal: the excuse page comes first.
        let html = r#"<a href="/student_ext/Exam/ExamExcuseFinal.aspx">Final Exam Excuse</a>
            <a href="/student_ext/Exam/ViewExamSeat_01.aspx">Exam Seats</a>"#;
        let p = Page::parse(Url::parse(PORTAL_HOME).unwrap(), html.into());
        assert_eq!(discover(Some(&p))[&Section::Exams].path(), "/student_ext/Exam/ViewExamSeat_01.aspx");
    }

    #[test]
    fn follows_script_redirect() {
        let html = "<script>eval(function(p,a,c,k,e,d){}('',0,0,''.split('|'),0,{}))</script><script>sTo('MJG663596')</script>";
        let url = Url::parse("https://apps.guc.edu.eg/student_ext/Grade/Transcript_001.aspx").unwrap();
        let next = crate::page::script_redirect(&url, html).unwrap();
        assert_eq!(next.as_str(), "https://apps.guc.edu.eg/student_ext/Grade/Transcript_001.aspx?v=MJG663596");
        assert!(crate::page::script_redirect(&url, "<html>normal page</html>").is_none());
    }
}
