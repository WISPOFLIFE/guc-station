//! Screens built on parsed portal data (guc::academic) rather than raw tables:
//! the week schedule, today's classes, exams, grades, attendance and CMS.

use crate::theme::{self, tokens};
use crate::views::{accent_text, banner, ghost, link, loaded, loader, masonry, muted, page_header, panel, secondary, subheading};
use crate::tools;
use crate::{Load, Message, PdfSource, Screen, Workspace};
use chrono::{Datelike, Local, NaiveDate, NaiveTime};
use guc::academic::{Class, CmsWeek, CourseAttendance, CourseGrades, Exam, Quiz, Schedule};
use guc::portal::Section;
use iced::widget::{Column, Row, Space, button, column, container, horizontal_rule, horizontal_space, row, text, text_input, tooltip};
use iced::{Alignment, Element, Length, Theme};

type El<'a> = Element<'a, Message>;

/// GUC's teaching week. Friday is off.
const WEEK: [&str; 6] = ["Saturday", "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday"];
const MIN_SLOT_HEIGHT: f32 = 78.0;
const MAX_SLOT_HEIGHT: f32 = 170.0;
/// Tall enough to also fit the full course name.
const SLOT_WITH_TITLE: f32 = 106.0;

fn portal_actions(ws: &Workspace, s: Section) -> Vec<El<'_>> {
    vec![secondary("Refresh", Message::Refresh), secondary("Open on portal", Message::OpenInBrowser(ws.sections[&s].to_string()))]
}

/// Shows the section page's load state until `parsed` exists.
fn while_loading<'a>(ws: &'a Workspace, s: Section, what: &'static str) -> El<'a> {
    match ws.page(s) {
        Load::Failed(e) => column![banner(e.clone(), true, None), secondary("Try again", Message::Refresh)].spacing(10).into(),
        _ => loader(what),
    }
}

// ---------------------------------------------------------------- schedule

fn kind_label(kind: &str) -> &str {
    match kind {
        "Tut" => "Tutorial",
        "Lab" => "Lab",
        "Lecture" => "Lecture",
        other => other,
    }
}

fn period_bounds(period: &str) -> Option<(NaiveTime, NaiveTime)> {
    let (a, b) = period.split_once('-')?;
    Some((NaiveTime::parse_from_str(a.trim(), "%H:%M").ok()?, NaiveTime::parse_from_str(b.trim(), "%H:%M").ok()?))
}

fn day_slots<'a>(s: &'a Schedule, day: &str) -> Option<&'a Vec<Vec<Class>>> {
    s.days.iter().find(|d| d.name.eq_ignore_ascii_case(day)).map(|d| &d.slots)
}

fn today_name() -> String {
    Local::now().format("%A").to_string()
}

/// Index of the period running now, or else the next one today.
fn live_period(s: &Schedule) -> (Option<usize>, Option<usize>) {
    let now = Local::now().time();
    let mut current = None;
    let mut next = None;
    for (i, p) in s.periods.iter().enumerate() {
        let Some((start, end)) = period_bounds(p) else { continue };
        if start <= now && now < end {
            current = Some(i);
        } else if start > now && next.is_none() {
            next = Some(i);
        }
    }
    (current, next)
}

pub fn schedule(ws: &Workspace) -> El<'_> {
    let header = page_header("Class schedule".into(), None, portal_actions(ws, Section::Schedule));
    let Some(s) = &ws.schedule else { return column![header, while_loading(ws, Section::Schedule, "Loading your schedule…")].spacing(20).into() };
    if s.is_empty() {
        return column![header, panel(muted("The portal has no classes on your schedule.").size(14))].spacing(20).into();
    }

    let today = today_name();
    let (current, next) = live_period(s);
    let periods = s.periods.len().max(s.days.iter().map(|d| d.slots.len()).max().unwrap_or(0));
    let ordinal = ["1st", "2nd", "3rd", "4th", "5th", "6th", "7th", "8th"];

    // The week fills the window: six rows share the height left under the header.
    let slot_height = ((ws.size.height - 56.0 - 62.0 - 20.0 - 40.0 - 44.0) / WEEK.len() as f32 - 8.0).clamp(MIN_SLOT_HEIGHT, MAX_SLOT_HEIGHT);

    let mut head = Row::new().spacing(8).push(Space::with_width(96));
    for i in 0..periods {
        let time = s.periods.get(i).map(|p| p.replace('-', "–")).unwrap_or_default();
        head = head.push(
            column![
                text(ordinal.get(i).copied().unwrap_or("")).size(12).font(theme::MEDIUM),
                muted(time).size(12),
            ]
            .spacing(2)
            .width(Length::FillPortion(1)),
        );
    }

    let mut grid = Column::new().spacing(8).push(head);
    for day in WEEK {
        let is_today = day == today;
        let slots = day_slots(s, day);
        let label = if is_today {
            column![accent_text(day).size(14).font(theme::SEMIBOLD), accent_text("Today").size(11)].spacing(2)
        } else {
            column![text(day).size(14).font(theme::MEDIUM)]
        };
        let mut r = Row::new().spacing(8).push(container(label).width(96).height(slot_height).align_y(Alignment::Center));
        for i in 0..periods {
            let classes = slots.and_then(|sl| sl.get(i)).map(Vec::as_slice).unwrap_or(&[]);
            let live = is_today && (current == Some(i) || (current.is_none() && next == Some(i)));
            r = r.push(slot_cell(classes, live, slot_height));
        }
        grid = grid.push(r);
    }

    column![header, panel(grid)].spacing(20).into()
}

fn slot_cell(classes: &[Class], live: bool, height: f32) -> El<'_> {
    if classes.is_empty() {
        return container(Space::new(Length::Fill, Length::Fill))
            .width(Length::FillPortion(1))
            .height(height)
            .style(theme::slot_free)
            .into();
    }
    let lecture = classes.iter().any(|c| c.kind == "Lecture");
    let body = Column::with_children(classes.iter().map(|c| {
        let mut content = column![
            text(c.code.as_str()).size(14).font(theme::SEMIBOLD),
            row![muted(kind_label(&c.kind)).size(12), text(c.room.as_str()).size(12)].spacing(8),
        ]
        .spacing(3);
        if height >= SLOT_WITH_TITLE && !c.title.is_empty() {
            content = content.push(muted(c.title.as_str()).size(12));
        }
        let tip = if c.title.is_empty() { c.group.clone() } else { format!("{} · {}", c.title, c.group) };
        tooltip(content, container(text(tip).size(12)).padding([6, 10]).style(theme::panel), tooltip::Position::Top).into()
    }))
    .spacing(6);
    container(body)
        .padding([9, 11])
        .width(Length::FillPortion(1))
        .height(height)
        .style(theme::slot_class(lecture, live))
        .into()
}

/// Home's "Today" panel: the day's classes, marking the one on now or next.
pub fn today(ws: &Workspace) -> El<'_> {
    let head = row![subheading("Today"), horizontal_space(), link("Full week", Message::Navigate(Screen::Section(Section::Schedule)))]
        .align_y(Alignment::Center);
    let body: El<'_> = match &ws.schedule {
        None => match ws.page(Section::Schedule) {
            Load::Failed(_) => muted("Couldn't load your schedule.").size(14).into(),
            _ => loader("Loading your schedule…"),
        },
        Some(s) => {
            let today = today_name();
            let (current, next) = live_period(s);
            let classes: Vec<(usize, &Class)> = day_slots(s, &today)
                .map(|sl| sl.iter().enumerate().flat_map(|(i, cs)| cs.iter().map(move |c| (i, c))).collect())
                .unwrap_or_default();
            if classes.is_empty() {
                muted(match next_class_day(s, &today) {
                    Some(day) => format!("No classes today. Your next class day is {day}."),
                    None => "No classes today.".into(),
                })
                .size(14)
                .into()
            } else {
                let mut col = Column::new().spacing(0);
                let mut tagged = false;
                for (n, (i, c)) in classes.iter().enumerate() {
                    let tag = if current == Some(*i) {
                        Some("Now")
                    } else if !tagged && current.is_none() && next.is_some_and(|nx| *i >= nx) {
                        Some("Next")
                    } else {
                        None
                    };
                    tagged |= tag.is_some();
                    let finished = s.periods.get(*i).and_then(|p| period_bounds(p)).is_some_and(|(_, end)| end <= Local::now().time());
                    let time = s.periods.get(*i).map(|p| p.replace('-', "–")).unwrap_or_default();
                    let name = if c.title.is_empty() { c.code.clone() } else { format!("{}  {}", c.code, c.title) };
                    let line = row![
                        muted(time).size(13).width(104),
                        column![
                            text(name).size(14).font(if finished { theme::REGULAR } else { theme::MEDIUM }),
                            muted(format!("{} · {}", kind_label(&c.kind), c.room)).size(12),
                        ]
                        .spacing(2)
                        .width(Length::Fill),
                        accent_text(tag.unwrap_or("")).size(12).font(theme::SEMIBOLD),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center);
                    if n > 0 {
                        col = col.push(horizontal_rule(1).style(theme::hairline));
                    }
                    col = col.push(container(line).padding([9, 0]));
                }
                col.into()
            }
        }
    };
    panel(column![head, body].spacing(12))
}

/// How many classes the Today panel lists, for page layout.
pub fn today_count(ws: &Workspace) -> usize {
    let slots = ws.schedule.as_ref().and_then(|s| day_slots(s, &today_name()));
    slots.map_or(0, |sl| sl.iter().map(Vec::len).sum())
}

fn next_class_day(s: &Schedule, today: &str) -> Option<&'static str> {
    let start = WEEK.iter().position(|d| *d == today).map(|i| i + 1).unwrap_or(0);
    (0..WEEK.len())
        .map(|k| WEEK[(start + k) % WEEK.len()])
        .find(|d| day_slots(s, d).is_some_and(|sl| sl.iter().any(|c| !c.is_empty())))
}

// ---------------------------------------------------------------- exams

pub fn exams(ws: &Workspace) -> El<'_> {
    let header = page_header("Exam schedule".into(), None, portal_actions(ws, Section::Exams));
    let Some(list) = &ws.exams else { return column![header, while_loading(ws, Section::Exams, "Loading your exam seats…")].spacing(20).into() };
    if list.is_empty() {
        return column![header, panel(muted("No exam seats have been published yet.").size(14))].spacing(20).into();
    }
    let today = Local::now().date_naive();
    let mut upcoming: Vec<&Exam> = list.iter().filter(|e| e.date.is_none_or(|d| d >= today)).collect();
    let mut done: Vec<&Exam> = list.iter().filter(|e| e.date.is_some_and(|d| d < today)).collect();
    upcoming.sort_by_key(|e| e.date);
    done.sort_by_key(|e| std::cmp::Reverse(e.date));

    let mut groups = Vec::new();
    if !upcoming.is_empty() {
        groups.push((exam_group("Coming up", &upcoming, today, false), upcoming.len()));
    }
    if !done.is_empty() {
        groups.push((exam_group("Done", &done, today, true), done.len()));
    }
    column![header, masonry(groups, ws.columns(640.0))].spacing(20).into()
}

fn exam_group<'a>(title: &'a str, list: &[&'a Exam], today: NaiveDate, past: bool) -> El<'a> {
    let mut rows = Column::new().spacing(0);
    for (n, e) in list.iter().enumerate() {
        if n > 0 {
            rows = rows.push(horizontal_rule(1).style(theme::hairline));
        }
        rows = rows.push(exam_row(e, today, past));
    }
    column![subheading(title), panel(rows)].spacing(10).into()
}

fn exam_row(e: &Exam, today: NaiveDate, past: bool) -> El<'_> {
    let (day, month) = e.date.map(|d| (d.day().to_string(), d.format("%b").to_string())).unwrap_or_default();
    let when = match e.date.map(|d| (d - today).num_days()) {
        Some(0) => "Today".to_string(),
        Some(1) => "Tomorrow".to_string(),
        Some(n) if n > 1 => format!("In {n} days"),
        _ => e.day.clone(),
    };
    let fade = move |t: &Theme| {
        let k = tokens(t);
        text::Style { color: Some(if past { k.muted } else { k.text }) }
    };
    let date_block = column![
        text(day).size(24).font(theme::SEMIBOLD).style(fade),
        muted(month).size(12),
    ]
    .align_x(Alignment::Center)
    .width(56);
    let course = column![
        text(e.course.as_str()).size(15).font(theme::SEMIBOLD).style(fade),
        muted(format!("{} · {} · {}", e.code, e.day, e.kind)).size(12),
    ]
    .spacing(3)
    .width(Length::Fill);
    let time = column![
        text(format!("{}–{}", e.start, e.end)).size(14).style(fade),
        if past { muted(when).size(12) } else { accent_text(when).size(12).font(theme::MEDIUM) },
    ]
    .spacing(3)
    .width(120);
    let place = column![text(e.hall.as_str()).size(14).style(fade), muted("Hall").size(12)].spacing(3).width(120);
    let seat = column![text(e.seat.as_str()).size(20).font(theme::SEMIBOLD).style(fade), muted("Seat").size(12)]
        .spacing(1)
        .align_x(Alignment::End)
        .width(64);
    container(row![date_block, course, time, place, seat].spacing(16).align_y(Alignment::Center)).padding([12, 4]).into()
}

// ---------------------------------------------------------------- grades

/// "MET Digital Media Engineering 7th Semester - DMET703 Video and Audio Technology"
/// -> ("DMET 703", "Video and Audio Technology")
fn course_title(label: &str) -> (String, String) {
    let tail = label.rsplit(" - ").next().unwrap_or(label).trim();
    let (code, name) = tail.split_once(' ').unwrap_or((tail, ""));
    let code = match code.find(|c: char| c.is_ascii_digit()) {
        Some(i) if i > 0 => format!("{} {}", &code[..i], &code[i..]),
        _ => code.to_string(),
    };
    (code, name.to_string())
}

pub fn grades(ws: &Workspace) -> El<'_> {
    let mut actions = portal_actions(ws, Section::Grades);
    actions.push(ghost("Grade calculator", Message::Navigate(Screen::GradeCalc)));
    let header = page_header("Grades".into(), Some("Quiz, assignment and midterm marks published so far".into()), actions);
    let body = loaded(&ws.grades, "Collecting grades from each of your courses…", Message::Refresh, |courses| {
        if courses.is_empty() {
            return panel(muted("No courses this semester.").size(14));
        }
        let items = courses.iter().map(|c| (course_grades(c), 2 + c.quizzes.len() + usize::from(c.midterm.is_some()))).collect();
        masonry(items, ws.columns(420.0))
    });
    column![header, body].spacing(20).into()
}

fn course_grades(c: &CourseGrades) -> El<'_> {
    let (code, name) = course_title(&c.course);
    let mut col = Column::new().spacing(10).push(
        row![text(name).size(16).font(theme::SEMIBOLD).width(Length::Fill), muted(code).size(13)].align_y(Alignment::Center),
    );
    if let Some(m) = &c.midterm {
        col = col.push(row![text("Midterm").size(14).width(Length::Fill), text(format!("{}%", m.trim().parse().map(trim).unwrap_or_else(|_: std::num::ParseFloatError| m.clone()))).size(14).font(theme::SEMIBOLD)]);
    }
    // Group question rows under their quiz or assignment.
    let mut groups: Vec<(&str, Vec<&Quiz>)> = Vec::new();
    for q in &c.quizzes {
        match groups.iter_mut().find(|(n, _)| *n == q.name) {
            Some((_, v)) => v.push(q),
            None => groups.push((q.name.as_str(), vec![q])),
        }
    }
    if groups.is_empty() && c.midterm.is_none() {
        col = col.push(muted("Nothing published yet.").size(14));
    }
    for (name, parts) in groups {
        let score: f32 = parts.iter().filter_map(|q| q.score).sum();
        let max: f32 = parts.iter().filter_map(|q| q.max).sum();
        // An unmarked item has no score at all, which is different from zero.
        let graded = parts.iter().any(|q| q.score.is_some());
        let pct = (graded && max > 0.0).then(|| score / max * 100.0);
        let shown = if graded { format!("{} / {}", trim(score), trim(max)) } else { format!("Not marked / {}", trim(max)) };
        let staff = parts.first().map(|q| q.staff.as_str()).unwrap_or("");
        col = col.push(horizontal_rule(1).style(theme::hairline));
        col = col.push(
            row![
                column![text(name).size(14).font(theme::MEDIUM), muted(staff).size(12)].spacing(2).width(Length::Fill),
                muted(pct.map(|p| format!("{p:.0}%")).unwrap_or_default()).size(13).width(56),
                text(shown).size(15).font(theme::SEMIBOLD),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        );
        if parts.len() > 1 {
            for q in parts {
                col = col.push(
                    row![
                        muted(q.element.as_str()).size(12).width(Length::Fill),
                        muted(format!("{} / {}", q.score.map(trim).unwrap_or("–".into()), q.max.map(trim).unwrap_or("–".into()))).size(12),
                    ]
                    .padding([0, 0]),
                );
            }
        }
    }
    panel(col)
}

fn trim(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------------------------------------------------------------- attendance

pub fn attendance(ws: &Workspace) -> El<'_> {
    let header = page_header(
        "Attendance".into(),
        Some("Sessions your instructors have recorded. Missing more than 25% of a course drops it.".into()),
        portal_actions(ws, Section::Attendance),
    );
    let body = loaded(&ws.attendance, "Collecting attendance from each of your courses…", Message::Refresh, |courses| {
        if courses.is_empty() {
            return panel(muted("No courses this semester.").size(14));
        }
        let items = courses.iter().map(|c| (panel(course_attendance(c)), 2 + usize::from(c.absences() > 0) + usize::from(c.warning.is_some()))).collect();
        masonry(items, ws.columns(420.0))
    });
    column![header, body].spacing(20).into()
}

fn course_attendance(c: &CourseAttendance) -> El<'_> {
    let total = c.sessions.len();
    let missed = c.absences();
    let (code, name) = c.course.split_once(" - ").unwrap_or(("", c.course.as_str()));
    let missed_list = c
        .sessions
        .iter()
        .filter(|s| !s.attended)
        .map(|s| {
            let d = s.date.map(|d| d.format("%-d %b").to_string()).unwrap_or_default();
            format!("{d} {}", s.kind).trim().to_string()
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut info = column![
        row![text(name).size(15).font(theme::SEMIBOLD), muted(code).size(12)].spacing(8).align_y(Alignment::Center),
        muted(if total == 0 { "No sessions recorded yet".to_string() } else { format!("Attended {} of {total} recorded sessions", total - missed) }).size(13),
    ]
    .spacing(3)
    .width(Length::Fill);
    if !missed_list.is_empty() {
        info = info.push(muted(format!("Missed: {missed_list}")).size(12));
    }
    if let Some(w) = &c.warning {
        info = info.push(banner(format!("Portal warning: {w}"), true, None));
    }
    let count = column![
        text(missed.to_string()).size(24).font(theme::SEMIBOLD).style(move |t: &Theme| {
            let k = tokens(t);
            text::Style { color: Some(if missed == 0 { k.muted } else { k.text }) }
        }),
        muted(if missed == 1 { "absence" } else { "absences" }).size(12),
    ]
    .align_x(Alignment::End)
    .width(80);
    row![info, count].spacing(16).align_y(Alignment::Center).into()
}

// ---------------------------------------------------------------- CMS

pub fn cms(ws: &Workspace) -> El<'_> {
    match &ws.cms.course {
        Some((title, load)) => cms_course(ws, title, load),
        None => cms_courses(ws),
    }
}

fn cms_courses(ws: &Workspace) -> El<'_> {
    let header = page_header("CMS".into(), None, vec![secondary("Refresh", Message::Refresh)]);
    let body = loaded(&ws.cms.home, "Loading your courses from the CMS…", Message::Refresh, |_| {
        if ws.cms.courses.is_empty() {
            return panel(muted("No courses were found on your CMS home page.").size(14));
        }
        let mut list = Vec::new();
        for c in &ws.cms.courses {
            list.push((
                button(
                    row![
                        text(c.code.as_str()).size(14).font(theme::SEMIBOLD).width(96),
                        text(c.name.as_str()).size(14).width(Length::Fill),
                        muted(c.season.as_str()).size(12),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([12, 12])
                .width(Length::Fill)
                .style(theme::list_row(false))
                .on_press(Message::OpenCourse(c.name.clone(), c.url.clone()))
                .into(),
                1,
            ));
        }
        panel(masonry(list, ws.columns(460.0)))
    });
    column![header, body].spacing(20).into()
}

fn cms_course<'a>(ws: &'a Workspace, title: &'a str, load: &'a Load<guc::page::Page>) -> El<'a> {
    let mut actions = vec![ghost("All courses", Message::CloseCourse), secondary("Refresh", Message::Refresh)];
    if let Load::Ready(p) = load {
        if let Some(pz) = p.links.iter().find(|l| l.href.host_str().is_some_and(|h| h.contains("piazza"))) {
            actions.push(secondary("Piazza", Message::OpenInBrowser(pz.href.to_string())));
        }
    }
    let header = page_header(title.to_string(), None, actions);
    let body = loaded(load, "Loading course content…", Message::Refresh, |_| {
        let weeks = &ws.cms.weeks;
        if weeks.is_empty() {
            return panel(muted("Nothing has been posted to this course yet.").size(14));
        }
        let mut kinds: Vec<&str> = weeks.iter().flat_map(|w| w.items.iter().map(|i| i.kind.as_str())).filter(|k| !k.is_empty()).collect();
        kinds.sort();
        kinds.dedup();
        let chip = |label: String, value: Option<String>| {
            let active = ws.cms.kind == value;
            button(text(label).size(13))
                .padding([6, 12])
                .style(theme::nav_item(active))
                .on_press(Message::CmsKind(value))
        };
        let mut chips = Row::new().spacing(4).push(chip("All".into(), None));
        for k in kinds {
            chips = chips.push(chip(k.to_string(), Some(k.to_string())));
        }
        let search = text_input("Search files", &ws.cms.filter).on_input(Message::CmsFilter).padding(9).size(14).style(theme::input).width(260);

        let mut col = Column::new().spacing(18).push(row![chips, horizontal_space(), search].align_y(Alignment::Center));
        let shown: Vec<_> = weeks.iter().filter_map(|w| cms_week(ws, w).map(|block| (block, w.items.len() + 2))).collect();
        col = col.push(if shown.is_empty() { panel(muted("No files match.").size(14)) } else { masonry(shown, ws.columns(480.0)) });
        col.into()
    });
    column![header, body].spacing(20).into()
}

fn cms_week<'a>(ws: &'a Workspace, w: &'a CmsWeek) -> Option<El<'a>> {
    let needle = ws.cms.filter.to_lowercase();
    let items: Vec<_> = w
        .items
        .iter()
        .filter(|i| ws.cms.kind.as_deref().is_none_or(|k| i.kind == k))
        .filter(|i| needle.is_empty() || i.title.to_lowercase().contains(&needle))
        .collect();
    if items.is_empty() {
        return None;
    }
    // Some courses date every group the same, which says nothing; name it by content then.
    let heading = if ws.cms.weeks.len() > 1 && ws.cms.weeks.iter().all(|other| other.week == w.week) {
        most_common_kind(w)
    } else {
        NaiveDate::parse_from_str(&w.week, "%Y-%m-%d").map(|d| format!("Week of {}", d.format("%-d %B"))).unwrap_or_else(|_| w.week.clone())
    };
    let mut list = Column::new().spacing(0);
    for (n, i) in items.into_iter().enumerate() {
        if n > 0 {
            list = list.push(horizontal_rule(1).style(theme::hairline));
        }
        // "3 - CSEN701_Lecture02_Overview of Processors" reads better with underscores as spaces.
        let title = i.title.replace('_', " ");
        let mut line = row![column![text(title).size(14), muted(i.kind.as_str()).size(12)].spacing(2).width(Length::Fill)]
            .spacing(12)
            .align_y(Alignment::Center);
        if tools::is_pdf_name(i.url.path()) {
            let folder = ws.cms.course.as_ref().map(|(t, _)| t.clone()).unwrap_or_else(|| "Files".into());
            line = line.push(
                button(text("View").size(13).font(theme::MEDIUM))
                    .padding([4, 10])
                    .style(theme::secondary_button)
                    .on_press(Message::ViewPdf(PdfSource::Cms { url: i.url.clone(), title: i.title.clone(), folder })),
            );
        }
        line = line.push(accent_text("Download").size(13).font(theme::MEDIUM));
        list = list.push(
            button(line)
            .padding([10, 10])
            .width(Length::Fill)
            .style(theme::list_row(false))
            .on_press(Message::OpenLink(i.url.clone(), i.title.clone())),
        );
    }
    Some(column![subheading_owned(heading), panel(list)].spacing(10).into())
}

fn most_common_kind(w: &CmsWeek) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for i in &w.items {
        match counts.iter_mut().find(|(kind, _)| *kind == i.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((i.kind.as_str(), 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map_or_else(String::new, |(kind, _)| kind.to_string())
}

fn subheading_owned<'a>(s: String) -> iced::widget::Text<'a> {
    text(s).size(15).font(theme::SEMIBOLD)
}


// ---------------------------------------------------------------- transcript

pub fn transcript(ws: &Workspace) -> El<'_> {
    let header = page_header("Transcript".into(), None, portal_actions(ws, Section::Transcript));
    if let Load::Failed(e) = ws.page(Section::Transcript) {
        return column![header, banner(e.clone(), true, None), secondary("Try again", Message::Refresh)].spacing(20).into();
    }

    // Cumulative GPA and the year picker.
    let mut summary = Row::new().spacing(16).align_y(Alignment::Center);
    match &ws.cumulative {
        Some((label, gpa)) => {
            summary = summary.push(
                column![
                    muted(if label.is_empty() { "Cumulative GPA".to_string() } else { format!("Cumulative GPA · {label}") }).size(13),
                    row![
                        text(format!("{gpa:.2}")).size(34).font(theme::SEMIBOLD),
                        text(guc::portal::letter(*gpa)).size(18).font(theme::SEMIBOLD).style(theme::grade_text(guc::portal::letter(*gpa))),
                    ]
                    .spacing(10)
                    .align_y(Alignment::End),
                ]
                .spacing(4)
                .width(Length::Fill),
            )
        }
        None => summary = summary.push(container(loader("Loading your transcript…")).width(Length::Fill)),
    }
    let years = ws.transcript_years();
    if !years.is_empty() {
        let shown = ws.transcript_loading.clone().or_else(|| ws.transcript_year.clone());
        summary = summary.push(
            iced::widget::pick_list(years, shown, Message::TranscriptYear)
                .placeholder("Study year")
                .padding([8, 12])
                .text_size(14)
                .style(theme::select),
        );
    }
    let year = ws.transcript_year.as_ref().and_then(|y| ws.transcripts.get(y));
    if year.is_some_and(|t| !t.seasons.is_empty()) {
        summary = summary.push(secondary("Use in GPA calculator", Message::GpaFromTranscript));
    }

    let mut col = Column::new().spacing(16).push(header).push(panel(summary));
    if let Some(y) = &ws.transcript_loading {
        col = col.push(loader(format!("Loading {y}… The portal can make the transcript wait up to a minute between requests.")));
    } else if let Some(t) = year {
        if t.seasons.is_empty() {
            col = col.push(panel(muted(format!("No grades recorded for {} yet.", t.year)).size(14)));
        }
        let items = t.seasons.iter().map(|s| (season(s), s.courses.len() + 2)).collect();
        col = col.push(masonry(items, ws.columns(440.0)));
    }
    col.into()
}

fn season(s: &guc::academic::Season) -> El<'_> {
    let mut meta = Vec::new();
    if let Some(g) = s.gpa {
        meta.push(format!("GPA {g:.2}"));
    }
    if let Some(h) = s.hours {
        meta.push(format!("{} credit hours", trim(h)));
    }
    let mut col = Column::new().spacing(0).push(
        row![subheading_owned(s.name.clone()).width(Length::Fill), muted(meta.join("  ·  ")).size(13)]
            .align_y(Alignment::Center)
            .padding(iced::Padding { bottom: 8.0, ..Default::default() }),
    );
    for c in &s.courses {
        col = col.push(horizontal_rule(1).style(theme::hairline));
        col = col.push(
            row![
                text(c.name.as_str()).size(14).width(Length::Fill),
                text(c.letter.as_str()).size(14).font(theme::SEMIBOLD).style(theme::grade_text(&c.letter)).width(36),
                muted(c.numeric.map(trim).unwrap_or_default()).size(13).width(44),
                muted(c.hours.map(|h| format!("{} h", trim(h))).unwrap_or_default()).size(13).width(44),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .padding([9, 0]),
        );
    }
    panel(col)
}

// ---------------------------------------------------------------- evaluations

pub fn evaluations(ws: &Workspace) -> El<'_> {
    let header = page_header(
        "Evaluations".into(),
        Some("Course and staff evaluations open on the portal for this season".into()),
        vec![
            secondary("Refresh", Message::Refresh),
            secondary("Open on portal", Message::OpenInBrowser(ws.sections[&Section::CourseEvaluation].to_string())),
        ],
    );
    let body = loaded(&ws.evaluations, "Checking each course and staff member…", Message::Refresh, |(courses, staff)| {
        let pending = courses.iter().chain(staff).filter(|e| !e.done).count();
        let total = courses.len() + staff.len();
        let summary: El<'_> = if total == 0 {
            muted("There's nothing to evaluate right now.").size(14).into()
        } else if pending == 0 {
            text(format!("All {total} evaluations are done.")).size(15).font(theme::MEDIUM).into()
        } else {
            row![
                text(format!("{pending} of {total} still to do.")).size(15).font(theme::MEDIUM),
                muted("Open them on the portal to fill them in.").size(14),
            ]
            .spacing(8)
            .into()
        };
        let mut groups = Vec::new();
        if !courses.is_empty() {
            groups.push((eval_group("Courses", courses), courses.len()));
        }
        if !staff.is_empty() {
            groups.push((eval_group("Staff", staff), staff.len()));
        }
        column![panel(summary), masonry(groups, ws.columns(420.0))].spacing(16).into()
    });
    column![header, body].spacing(20).into()
}

fn eval_group<'a>(title: &'a str, list: &'a [guc::academic::Evaluation]) -> El<'a> {
    let mut rows = Column::new().spacing(0);
    for (n, e) in list.iter().enumerate() {
        if n > 0 {
            rows = rows.push(horizontal_rule(1).style(theme::hairline));
        }
        let status: El<'_> = if e.done {
            muted("Evaluated").size(13).into()
        } else {
            accent_text("Not evaluated yet").size(13).font(theme::MEDIUM).into()
        };
        rows = rows.push(
            row![text(e.name.as_str()).size(14).width(Length::Fill), status].align_y(Alignment::Center).padding([10, 0]),
        );
    }
    column![subheading(title), panel(rows)].spacing(10).into()
}
