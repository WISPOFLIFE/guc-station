//! Every screen. Views only read state and emit `Message`s.

use crate::screens;
use crate::theme::{self, AppTheme, tokens};
use crate::tools::{CalcMsg, GpaMsg};
use crate::{App, COMPOSE_TO, Draft, LoginForm, Load, MAIL_PAGE_SIZE, Message, Notice, Screen, Session, Workspace};
use guc::mail::{Compose, Folder};
use guc::page::{Page, Table};
use guc::portal::{self, Section};
use iced::widget::{
    Column, Row, Space, button, center, image, checkbox, column, container, horizontal_rule, horizontal_space, pick_list, text_editor,
    row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Theme};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

type El<'a> = Element<'a, Message>;

// Wide enough for the 198px logo to line up with the nav labels.
pub const SIDEBAR_WIDTH: f32 = 246.0;
pub const PAGE_PADDING_X: f32 = 32.0;
pub const SCROLLBAR_ROOM: f32 = 12.0;
pub const COLUMN_GAP: f32 = 14.0;
/// Below this content width, side-by-side pages stack.
const WIDE_PAGE: f32 = 960.0;
/// Reading width for mail and notification text.
const READING_WIDTH: f32 = 780.0;

pub const LOADER_FRAME_MS: u64 = 140;
const LOADER_SIZE: (f32, f32) = (28.0, 18.0);

/// Shared by every loading indicator, wherever it is in the tree.
static LOADER_FRAME: AtomicUsize = AtomicUsize::new(0);
static LOADER_FRAMES: LazyLock<[image::Handle; 4]> = LazyLock::new(|| {
    [
        image::Handle::from_bytes(include_bytes!("../assets/brand/loading-0.png").as_slice()),
        image::Handle::from_bytes(include_bytes!("../assets/brand/loading-1.png").as_slice()),
        image::Handle::from_bytes(include_bytes!("../assets/brand/loading-2.png").as_slice()),
        image::Handle::from_bytes(include_bytes!("../assets/brand/loading-3.png").as_slice()),
    ]
});

pub fn advance_loader() {
    LOADER_FRAME.fetch_add(1, Ordering::Relaxed);
}

/// The brand's pixel art, scaled without blurring.
fn pixel_art<'a>(handle: image::Handle, width: f32, height: f32) -> Element<'a, Message> {
    image(handle).width(width).height(height).filter_method(image::FilterMethod::Nearest).into()
}

/// The rolling train at `scale` times its 28x18 size.
fn train<'a>(scale: f32) -> El<'a> {
    let frame = LOADER_FRAME.load(Ordering::Relaxed) % LOADER_FRAMES.len();
    pixel_art(LOADER_FRAMES[frame].clone(), LOADER_SIZE.0 * scale, LOADER_SIZE.1 * scale)
}

/// The inline loading indicator: the train and what it's waiting for.
pub(crate) fn loader<'a>(caption: impl text::IntoFragment<'a>) -> El<'a> {
    row![train(2.0), muted(caption).size(14)].spacing(12).align_y(Alignment::Center).into()
}

pub fn root(app: &App) -> El<'_> {
    let body = match &app.session {
        Session::SignedOut(form) => login(form),
        // Asked once, after the first sign-in; the portal loads behind it.
        Session::SignedIn(_) if app.settings.download_dir.is_none() => download_setup(app),
        Session::SignedIn(ws) => shell(app, ws),
    };
    container(body).width(Length::Fill).height(Length::Fill).style(theme::app_bg).into()
}

// ---------------------------------------------------------------- sign in

fn wordmark<'a>() -> El<'a> {
    pixel_art(image::Handle::from_bytes(include_bytes!("../assets/brand/wordmark@4x.png").as_slice()), 320.0, 48.0)
}

fn login(form: &LoginForm) -> El<'_> {
    if form.busy {
        let splash = column![wordmark(), train(4.0), muted("Signing in…").size(14)].spacing(18).align_x(Alignment::Center);
        return center(splash).into();
    }
    let ready = !form.busy && !form.login.trim().is_empty() && !form.password.is_empty();
    let submit = button(
        text(if form.busy { "Signing in…" } else { "Sign in" }).font(theme::MEDIUM).width(Length::Fill).center(),
    )
    .padding([11, 16])
    .width(Length::Fill)
    .style(theme::primary_button)
    .on_press_maybe(ready.then_some(Message::SignIn));

    let mut fields = column![
        wordmark(),
        muted("Sign in with the same account you use for the student portal, CMS and university mail.")
            .size(14),
        Space::with_height(12),
        label("Username"),
        text_input("firstname.lastname", &form.login)
            .on_input(Message::LoginChanged)
            .on_submit(Message::SignIn)
            .padding(11)
            .style(theme::input),
        Space::with_height(4),
        label("Password"),
        text_input("Your GUC password", &form.password)
            .on_input(Message::PasswordChanged)
            .on_submit(Message::SignIn)
            .secure(true)
            .padding(11)
            .style(theme::input),
        Space::with_height(2),
        checkbox("Keep me signed in on this computer", form.remember)
            .on_toggle(Message::RememberToggled)
            .text_size(14)
            .size(16),
        Space::with_height(6),
        submit,
    ]
    .spacing(8)
    .max_width(380);

    if let Some(e) = &form.error {
        fields = fields.push(banner(e.clone(), true, None));
    }
    fields = fields.push(Space::with_height(10)).push(
        muted("Your password goes straight to GUC's servers. If you choose to stay signed in, it's kept in Windows Credential Manager on this computer and nowhere else.")
            .size(12),
    );
    center(fields).padding(24).into()
}

fn download_setup(app: &App) -> El<'_> {
    let suggested = crate::settings::default_download_dir();
    let busy = app.picking_folder;
    let mut card = column![
        wordmark(),
        Space::with_height(4),
        text("Where should downloads go?").size(22).font(theme::SEMIBOLD),
        muted("Course files and mail attachments are saved in a GUC Station folder, sorted by course. You can change this later in Settings.").size(14),
        container(text(suggested.display().to_string()).size(14).font(theme::MEDIUM)).padding([10, 12]).width(Length::Fill).style(theme::panel),
        button(text("Use this folder").font(theme::MEDIUM).width(Length::Fill).center())
            .padding([11, 16])
            .width(Length::Fill)
            .style(theme::primary_button)
            .on_press_maybe((!busy).then(|| Message::UseDownloadFolder(suggested.clone()))),
        button(text(if busy { "Choosing a folder…" } else { "Choose a different location…" }).width(Length::Fill).center())
            .padding([10, 16])
            .width(Length::Fill)
            .style(theme::secondary_button)
            .on_press_maybe((!busy).then_some(Message::ChooseDownloadFolder)),
        muted("If you choose a different location, a GUC Station folder is made inside it.").size(12),
    ]
    .spacing(12)
    .max_width(440);
    if let Some(e) = &app.folder_error {
        card = card.push(banner(e.clone(), true, None));
    }
    center(card).padding(24).into()
}

// ---------------------------------------------------------------- shell

fn shell<'a>(app: &'a App, ws: &'a Workspace) -> El<'a> {
    let body: El<'a> = match ws.screen {
        Screen::Home => home(ws),
        Screen::Section(s) => match (&ws.detail, s) {
            (Some(d), _) => detail(d, s),
            (None, Section::Schedule) => screens::schedule(ws),
            (None, Section::Exams) => screens::exams(ws),
            (None, Section::Grades) => screens::grades(ws),
            (None, Section::Attendance) => screens::attendance(ws),
            (None, Section::Transcript) => screens::transcript(ws),
            (None, Section::CourseEvaluation | Section::StaffEvaluation) => screens::evaluations(ws),
            (None, _) => section(ws, s),
        },
        Screen::Cms => screens::cms(ws),
        Screen::Mail => mail(ws),
        Screen::Gpa => gpa(ws),
        Screen::GradeCalc => grade_calc(ws),
        Screen::Settings => settings(app, ws),
    };

    let mut content = Column::new().width(Length::Fill).height(Length::Fill);
    if let Some(n) = &ws.notice {
        let (msg, is_err) = match n {
            Notice::Info(m) => (m.clone(), false),
            Notice::Error(m) => (m.clone(), true),
        };
        content = content.push(container(banner(msg, is_err, Some(Message::DismissNotice))).padding([12, 32]));
    }
    // Mail manages its own scrolling panes.
    let body: El<'a> = if ws.screen == Screen::Mail {
        container(body).padding([24, 32]).height(Length::Fill).into()
    } else {
        scrollable(container(body).padding([28, 32]).width(Length::Fill)).id(crate::content_scroll_id()).height(Length::Fill).into()
    };
    content = content.push(body);

    row![sidebar(ws), content].into()
}

/// Packs items into `columns` equal columns, each item going to the shortest so
/// far (by its `weight`, a rough height), so panels of different sizes leave no gaps.
pub(crate) fn masonry<'a>(items: Vec<(El<'a>, usize)>, columns: usize) -> El<'a> {
    let columns = columns.clamp(1, items.len().max(1));
    let mut stacks: Vec<(usize, Vec<El<'a>>)> = (0..columns).map(|_| (0, Vec::new())).collect();
    for (item, weight) in items {
        let shortest = (0..columns).min_by_key(|&i| stacks[i].0).unwrap_or(0);
        stacks[shortest].0 += weight;
        stacks[shortest].1.push(item);
    }
    Row::with_children(
        stacks.into_iter().map(|(_, stack)| Column::with_children(stack).spacing(COLUMN_GAP).width(Length::FillPortion(1)).into()),
    )
    .spacing(COLUMN_GAP)
    .width(Length::Fill)
    .into()
}

/// The page's main content beside a summary when there's room, stacked otherwise.
fn main_and_side<'a>(ws: &Workspace, main: El<'a>, side: El<'a>) -> El<'a> {
    if ws.content_width() >= WIDE_PAGE {
        row![container(main).width(Length::FillPortion(3)), container(side).width(Length::FillPortion(2))].spacing(COLUMN_GAP).into()
    } else {
        column![side, main].spacing(COLUMN_GAP).into()
    }
}

fn sidebar(ws: &Workspace) -> El<'_> {
    let item = |label: &'static str, screen: Screen| {
        button(text(label).size(14).font(if ws.screen == screen { theme::MEDIUM } else { theme::REGULAR }))
            .width(Length::Fill)
            .padding([7, 12])
            .style(theme::nav_item(ws.screen == screen))
            .on_press(Message::Navigate(screen))
    };
    let group = |name: &'static str| {
        container(muted(name).size(12).font(theme::MEDIUM)).padding(iced::Padding { top: 20.0, bottom: 6.0, left: 12.0, right: 12.0 })
    };

    let first_name = display_name(ws.client.0.username());

    let nav = column![
        container(pixel_art(image::Handle::from_bytes(include_bytes!("../assets/brand/logo-sidebar@2x.png").as_slice()), 198.0, 32.0)).padding([0, 12]),
        Space::with_height(14),
        item("Home", Screen::Home),
        item("Mail", Screen::Mail),
        item("CMS", Screen::Cms),
        group("Academics"),
        item("Class schedule", Screen::Section(Section::Schedule)),
        item("Exam schedule", Screen::Section(Section::Exams)),
        item("Grades", Screen::Section(Section::Grades)),
        item("Transcript", Screen::Section(Section::Transcript)),
        item("Attendance", Screen::Section(Section::Attendance)),
        group("Tools"),
        item("GPA calculator", Screen::Gpa),
        item("Grade calculator", Screen::GradeCalc),
        group("Admin"),
        item("Payments", Screen::Section(Section::Finance)),
        item("Evaluations", Screen::Section(Section::CourseEvaluation)),
        item("Notifications", Screen::Section(Section::Notifications)),
    ]
    .spacing(2);

    let footer = column![
        horizontal_rule(1),
        Space::with_height(8),
        container(column![text(first_name).size(14).font(theme::MEDIUM), muted(ws.client.0.username()).size(12)].spacing(2))
            .padding([0, 12]),
        Space::with_height(6),
        item("Settings", Screen::Settings),
    ]
    .spacing(2);

    container(column![scrollable(nav).height(Length::Fill), footer])
        .width(SIDEBAR_WIDTH)
        .height(Length::Fill)
        .padding([20, 12])
        .style(theme::sidebar)
        .into()
}

// ---------------------------------------------------------------- home

fn home(ws: &Workspace) -> El<'_> {
    let now = chrono::Local::now();
    let greeting = match chrono::Timelike::hour(&now) {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        _ => "Good evening",
    };
    let header = page_header(
        format!("{greeting}, {}", display_name(ws.client.0.username())),
        Some(now.format("%A, %-d %B").to_string()),
        vec![secondary("Refresh", Message::Refresh)],
    );

    // Fill the window: list lengths follow its height when the page has two columns.
    let wide = ws.content_width() >= WIDE_PAGE;
    let page_height = ws.size.height - 56.0 - 62.0 - 20.0;
    let today_height = 92.0 + screens::today_count(ws) as f32 * 55.0;
    let notification_rows = if wide { ((page_height - today_height - 16.0 - 100.0) / 59.0) as usize } else { 6 }.clamp(3, 30);
    let mail_rows = if wide { ((page_height - 100.0) / 66.0) as usize } else { 6 }.clamp(4, 40);

    let notifications = panel(column![
        row![subheading("Notifications"), horizontal_space(), link("See all", Message::Navigate(Screen::Section(Section::Notifications)))]
            .align_y(Alignment::Center),
        loaded(ws.page(Section::Notifications), "Loading notifications…", Message::Refresh, |p| {
            notification_list(p, notification_rows)
        }),
    ]
    .spacing(14));

    let inbox = panel(column![
        row![subheading("Latest mail"), horizontal_space(), link("Open mail", Message::Navigate(Screen::Mail))]
            .align_y(Alignment::Center),
        loaded(&ws.mail.list, "Checking your inbox…", Message::Refresh, |(items, _)| {
            if items.is_empty() {
                return muted("Your inbox is empty.").into();
            }
            let unread = items.iter().filter(|m| !m.is_read).count();
            let mut col = Column::new().spacing(2);
            if unread > 0 {
                col = col.push(muted(format!("{unread} unread in the latest {}", items.len())).size(13));
                col = col.push(Space::with_height(6));
            }
            for m in items.iter().take(mail_rows) {
                col = col.push(mail_row(m, false, true));
            }
            col.into()
        }),
    ]
    .spacing(14));

    let body: El<'_> = if wide {
        row![
            column![screens::today(ws), notifications].spacing(COLUMN_GAP).width(Length::FillPortion(3)),
            container(inbox).width(Length::FillPortion(2)),
        ]
        .spacing(COLUMN_GAP)
        .into()
    } else {
        column![screens::today(ws), notifications, inbox].spacing(COLUMN_GAP).into()
    };
    column![header, body].spacing(20).into()
}

// ---------------------------------------------------------------- portal sections

fn section(ws: &Workspace, s: Section) -> El<'_> {
    let load = ws.page(s);
    let url = ws.sections[&s].to_string();
    let mut actions = Vec::new();
    if ws.history.get(&s).is_some_and(|h| !h.is_empty()) {
        actions.push(ghost("Back", Message::Back(s)));
    }
    actions.push(secondary("Refresh", Message::Refresh));
    actions.push(secondary("Open on portal", Message::OpenInBrowser(url)));
    if load.ready().is_some() {
        actions.push(ghost("Save page", Message::SavePageHtml(s)));
    }
    let header = page_header(s.title().to_string(), None, actions);

    let body = loaded(load, "Loading from the portal…", Message::Refresh, move |p| section_body(s, p));
    column![header, body].spacing(20).into()
}

fn section_body(s: Section, p: &Page) -> El<'_> {
    let mut col = Column::new().spacing(16);

    let pickers: Vec<El<'_>> = p
        .dropdowns
        .iter()
        .filter(|d| d.choices.len() > 1)
        .map(|d| {
            let selected = d.selected.as_ref().and_then(|v| d.choices.iter().find(|c| &c.value == v)).cloned();
            let name = d.name.clone();
            pick_list(d.choices.clone(), selected, move |c| Message::DropdownPicked(s, name.clone(), c))
                .placeholder("Choose…")
                .padding([8, 12])
                .text_size(14)
                .style(theme::select)
                .into()
        })
        .collect();
    if !pickers.is_empty() {
        col = col.push(Row::with_children(pickers).spacing(10));
    }

    match s {
        Section::Transcript => {
            let courses = portal::graded_courses(p);
            if let Some(g) = portal::gpa(&courses) {
                let hours: f32 = courses.iter().map(|c| c.hours).sum();
                col = col.push(panel(
                    row![
                        column![
                            muted("GPA for the courses shown").size(13),
                            row![
                                text(format!("{g:.2}")).size(32).font(theme::SEMIBOLD),
                                accent_text(portal::letter(g)).size(18).font(theme::MEDIUM),
                            ]
                            .spacing(10)
                            .align_y(Alignment::End),
                            muted(format!("{} courses · {hours} credit hours", courses.len())).size(13),
                        ]
                        .spacing(4),
                        horizontal_space(),
                        secondary("Edit in GPA calculator", Message::GpaFromTranscript),
                    ]
                    .align_y(Alignment::Center),
                ));
            }
        }
        Section::Grades => {
            col = col.push(
                row![
                    muted("Want to know what you need on the final?").size(14),
                    link("Open the grade calculator", Message::Navigate(Screen::GradeCalc)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        _ => {}
    }

    let tables: Vec<&Table> = p.tables.iter().filter(|t| t.rows.len() > 1 || t.rows.first().is_some_and(|r| r.len() > 1)).collect();
    if tables.is_empty() {
        let notes: Vec<El<'_>> = p
            .labels
            .iter()
            .filter(|(_, v)| v.len() < 400)
            .take(12)
            .map(|(_, v)| text(v.as_str()).size(14).into())
            .collect();
        col = col.push(panel(if notes.is_empty() {
            column![muted(if pickers_hint(p) {
                "Choose an option above to see its details."
            } else {
                "This page doesn't have any data to show yet."
            })]
        } else {
            Column::with_children(notes).spacing(6)
        }));
    } else if s == Section::Notifications {
        col = col.push(panel(notification_list(p, usize::MAX)));
    } else {
        for t in tables {
            col = col.push(panel(data_table(t, usize::MAX, Some(s))));
        }
    }
    col.into()
}

fn pickers_hint(p: &Page) -> bool {
    p.dropdowns.iter().any(|d| d.choices.len() > 1)
}

/// Columns whose longest value is at most this many characters get a fixed
/// width and never wrap, so IDs, dates and grades stay on one line.
const SHORT_COLUMN: usize = 16;
const CHAR_WIDTH: f32 = 7.4;

/// Draws a portal table. `section` makes linked cells clickable.
fn data_table(t: &Table, limit: usize, section: Option<Section>) -> El<'_> {
    let rows: Vec<usize> = (0..t.rows.len()).filter(|&i| t.rows[i].iter().any(|c| !c.is_empty())).collect();
    let Some(&first) = rows.first() else { return muted("Nothing to show.").into() };
    // Only the header row has text: the portal sent an empty list.
    if rows.len() == 1 && t.rows.len() > 1 {
        return muted("Nothing here right now.").size(14).into();
    }
    let has_header = rows.len() > 1;
    let body = if has_header { &rows[1..] } else { &rows[..] };
    let cols = t.rows.iter().map(Vec::len).max().unwrap_or(1);
    let cell = |i: usize, c: usize| t.rows[i].get(c).map(String::as_str).unwrap_or("");
    let columns: Vec<usize> = (0..cols).filter(|&c| body.iter().any(|&i| !cell(i, c).is_empty())).collect();
    let width = |c: usize| {
        let longest = rows.iter().map(|&i| cell(i, c).chars().count()).max().unwrap_or(1);
        if longest <= SHORT_COLUMN {
            (Length::Fixed(longest.max(2) as f32 * CHAR_WIDTH + 6.0), true)
        } else {
            (Length::FillPortion((longest.min(60) / 6 + 1) as u16), false)
        }
    };

    let draw_row = |i: usize, header: bool| -> El<'_> {
        let r = &t.rows[i];
        let cells: Vec<El<'_>> = if r.len() == 1 && cols > 1 {
            vec![text(r[0].as_str()).size(13).font(theme::MEDIUM).width(Length::Fill).into()]
        } else {
            columns
                .iter()
                .map(|&c| {
                    let (w, short) = width(c);
                    let mut label = text(cell(i, c)).size(13).font(if header { theme::MEDIUM } else { theme::REGULAR });
                    if short {
                        label = label.wrapping(text::Wrapping::None);
                    }
                    let target = t.targets.get(i).and_then(|r| r.get(c)).cloned().flatten();
                    match (target, section) {
                        (Some(target), Some(s)) if !header => button(label.style(|th: &Theme| text::Style { color: Some(tokens(th).accent) }))
                            .padding(0)
                            .style(theme::link_button)
                            .width(w)
                            .on_press(Message::Follow(s, target))
                            .into(),
                        _ => container(label).width(w).into(),
                    }
                })
                .collect()
        };
        let line = container(Row::with_children(cells).spacing(16)).padding([9, 12]).width(Length::Fill);
        if header { line.style(theme::table_header).into() } else { line.style(theme::table_row(false)).into() }
    };

    let mut col = Column::new().spacing(0);
    if has_header {
        col = col.push(draw_row(first, true));
    }
    for (n, &i) in body.iter().take(limit).enumerate() {
        if n > 0 {
            col = col.push(horizontal_rule(1).style(theme::hairline));
        }
        col = col.push(draw_row(i, false));
    }
    if body.len() > limit {
        col = col.push(container(muted(format!("{} more", body.len() - limit)).size(12)).padding([8, 12]));
    }
    col.into()
}

/// The portal's notifications grid, drawn as a readable list.
fn notification_list(p: &Page, limit: usize) -> El<'_> {
    let Some(t) = p.tables.iter().find(|t| t.rows.first().is_some_and(|h| h.iter().any(|c| c.eq_ignore_ascii_case("title")))) else {
        return match p.tables.iter().max_by_key(|t| t.rows.len()) {
            Some(t) => data_table(t, limit, Some(Section::Notifications)),
            None => muted("No notifications right now.").into(),
        };
    };
    let header: Vec<String> = t.rows[0].iter().map(|c| c.to_lowercase()).collect();
    let find = |name: &str| header.iter().position(|c| c.contains(name));
    let (title, date, staff, importance) = (find("title"), find("date"), find("staff"), find("importance"));
    let get = |r: &'_ Vec<String>, c: Option<usize>| c.and_then(|c| r.get(c)).map(String::as_str).unwrap_or("").to_string();

    let mut col = Column::new().spacing(0);
    let items: Vec<usize> = (1..t.rows.len()).filter(|&i| !get(&t.rows[i], title).is_empty()).collect();
    if items.is_empty() {
        return muted("No notifications right now.").into();
    }
    for (n, &i) in items.iter().take(limit).enumerate() {
        let r = &t.rows[i];
        let (headline, course) = split_notification_title(&get(r, title));
        let mut meta = vec![get(r, staff), short_date(&get(r, date))];
        if let Some(c) = course {
            meta.insert(0, c);
        }
        let meta = meta.into_iter().filter(|m| !m.is_empty()).collect::<Vec<_>>().join("  ·  ");
        let high = get(r, importance).eq_ignore_ascii_case("high");

        let mut top = row![text(headline).size(14).font(theme::MEDIUM).width(Length::Fill)].spacing(12);
        if high {
            top = top.push(accent_text("High priority").size(12).font(theme::MEDIUM));
        }
        let body = column![top, muted(meta).size(12)].spacing(4);
        let target = t.targets.get(i).and_then(|row| row.iter().flatten().next().cloned());
        if n > 0 {
            col = col.push(horizontal_rule(1).style(theme::hairline));
        }
        col = col.push(
            button(body)
                .padding([10, 12])
                .width(Length::Fill)
                .style(theme::list_row(false))
                .on_press_maybe(target.map(|t| Message::Follow(Section::Notifications, t))),
        );
    }
    if items.len() > limit {
        col = col.push(container(muted(format!("{} more", items.len() - limit)).size(12)).padding([8, 12]));
    }
    col.into()
}

/// "Notification System: Midterm Exam [Group: Winter 2026 - DMET 702 - 7DMET1 (Lecture)]"
/// becomes ("Midterm Exam", Some("DMET 702 · Lecture")).
fn split_notification_title(raw: &str) -> (String, Option<String>) {
    // Some titles arrive HTML-escaped twice, so "&amp;" survives one decode.
    let raw = raw.trim().replace("&amp;", "&");
    let raw = raw.strip_prefix("Notification System:").unwrap_or(&raw).trim();
    let Some(open) = raw.rfind("[Group:") else { return (raw.to_string(), None) };
    let headline = raw[..open].trim().to_string();
    let group = raw[open + 7..].trim_end_matches(']').trim();
    let parts: Vec<&str> = group.split(" - ").map(str::trim).collect();
    let kind = group.rsplit_once('(').map(|(_, k)| k.trim_end_matches(')').trim());
    let course = match (parts.get(1), kind) {
        (Some(code), Some(k)) => format!("{code} · {k}"),
        (Some(code), None) => code.to_string(),
        _ => group.to_string(),
    };
    (headline, Some(course))
}

/// "10/28/2026" -> "28 Oct", or the input unchanged.
pub(crate) fn short_date(s: &str) -> String {
    let date = s.split_whitespace().next().unwrap_or("");
    chrono::NaiveDate::parse_from_str(date, "%m/%d/%Y").map(|d| d.format("%-d %b").to_string()).unwrap_or_else(|_| s.to_string())
}

/// One opened notification (or any portal popup).
fn detail(d: &crate::Detail, s: Section) -> El<'_> {
    let (headline, course) = split_notification_title(&d.title);
    let meta = [course.unwrap_or_default(), d.by.clone(), short_date(&d.date)]
        .into_iter()
        .filter(|m| !m.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let back = if s == Section::Notifications { "All notifications" } else { "Back" };
    let reading = column![
        row![button(text(back).size(14)).padding([6, 10]).style(theme::ghost_button).on_press(Message::CloseDetail)],
        column![text(headline).size(24).font(theme::SEMIBOLD), muted(meta).size(13)].spacing(6),
        panel(
            text_editor(&d.body)
                .on_action(Message::DetailBody)
                .size(15)
                .line_height(1.55)
                .padding(0)
                .style(theme::reader),
        ),
    ]
    .spacing(16)
    .max_width(READING_WIDTH + 80.0);
    container(reading).center_x(Length::Fill).into()
}

// ---------------------------------------------------------------- CMS

// ---------------------------------------------------------------- mail

fn mail(ws: &Workspace) -> El<'_> {
    let m = &ws.mail;
    let tab = |label: &'static str, f: Folder| {
        button(text(label).size(13).font(theme::MEDIUM))
            .padding([6, 12])
            .style(theme::nav_item(m.folder == f))
            .on_press(Message::MailFolder(f))
    };
    let header = row![
        text("Mail").size(26).font(theme::SEMIBOLD),
        Space::with_width(16),
        tab("Inbox", Folder::Inbox),
        tab("Sent", Folder::Sent),
        horizontal_space(),
        secondary("Refresh", Message::Refresh),
        secondary("Open Outlook on the web", Message::OpenInBrowser(portal::MAIL_WEB.into())),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let header = if m.composing {
        header
    } else {
        let (label, msg) = if m.draft.is_some() { ("Continue draft", Message::ComposeResume) } else { ("New message", Message::ComposeStart(Compose::New)) };
        header.push(button(text(label).size(14).font(theme::MEDIUM)).padding([8, 14]).style(theme::primary_button).on_press(msg))
    };

    let mut search = row![
        text_input("Search mail", &m.query)
            .on_input(Message::MailQuery)
            .on_submit(Message::MailSearch)
            .padding(9)
            .size(14)
            .style(theme::input)
            .width(Length::Fill),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if !m.query.is_empty() || !m.active_query.is_empty() {
        search = search.push(ghost("Clear", Message::MailClearSearch));
    }

    let list = loaded(&m.list, "Loading messages…", Message::Refresh, |(items, total)| {
        let mut col = Column::new().spacing(2);
        if items.is_empty() {
            col = col.push(
                container(muted(if m.active_query.is_empty() {
                    "No messages here.".to_string()
                } else {
                    format!("No messages match \"{}\".", m.active_query)
                }))
                .padding(12),
            );
        }
        for item in items {
            col = col.push(mail_row(item, m.open_id.as_deref() == Some(item.id.as_str()), false));
        }
        let shown_to = (m.offset + items.len() as u32).min(*total);
        let pager = row![
            muted(format!("{}–{} of {total}", if items.is_empty() { 0 } else { m.offset + 1 }, shown_to)).size(12),
            horizontal_space(),
            button(text("Newer").size(13))
                .padding([5, 10])
                .style(theme::ghost_button)
                .on_press_maybe((m.offset > 0).then(|| Message::MailPage(m.offset.saturating_sub(MAIL_PAGE_SIZE)))),
            button(text("Older").size(13))
                .padding([5, 10])
                .style(theme::ghost_button)
                .on_press_maybe((shown_to < *total).then(|| Message::MailPage(m.offset + MAIL_PAGE_SIZE))),
        ]
        .align_y(Alignment::Center)
        .padding([6, 8]);
        // The right padding keeps the dates clear of the scrollbar.
        let col = col.padding(iced::Padding { right: 14.0, ..Default::default() });
        column![scrollable(col).height(Length::Fill), horizontal_rule(1), pager].into()
    });

    let list_width = (ws.content_width() * 0.32).clamp(340.0, 480.0);
    // Beside the sender when there's room, under it when the pane is narrow.
    let narrow_reader = ws.content_width() - list_width - 16.0 < 620.0;
    let reader: El<'_> = match &m.open {
        _ if m.composing && m.draft.is_some() => compose(m.draft.as_ref().unwrap()),
        Load::Idle => center(muted("Choose a message to read it here.")).into(),
        Load::Loading => center(loader("Opening message…")).into(),
        Load::Failed(e) => container(banner(e.clone(), true, None)).padding(20).into(),
        Load::Ready(msg) => {
            let images = m.images.ready().map(Vec::as_slice).unwrap_or(&[]);
            // The right padding keeps content clear of the scrollbar.
            let mut content = Column::new().spacing(16).padding(iced::Padding { right: 14.0, ..Default::default() });
            let files: Vec<_> = msg.attachments.iter().filter(|a| a.is_listed()).collect();
            if !files.is_empty() {
                let mut list = Column::new().spacing(2);
                for a in files {
                    list = list.push(
                        button(
                            row![
                                text(a.name.as_str()).size(14).width(Length::Fill),
                                muted(file_size(a.size)).size(12),
                                accent_text("Download").size(13).font(theme::MEDIUM),
                            ]
                            .spacing(12)
                            .align_y(Alignment::Center),
                        )
                        .padding([7, 8])
                        .width(Length::Fill)
                        .style(theme::list_row(false))
                        .on_press(Message::SaveAttachment(a.clone())),
                    );
                }
                content = content.push(column![muted("Attachments").size(12).font(theme::MEDIUM), list].spacing(6));
            }
            for shown in images.iter().filter(|i| i.is_large()) {
                content = content.push(mail_image(shown));
            }
            if !msg.body.is_empty() {
                // Read-only: edits are dropped in update, so this only selects and copies.
                content = content.push(
                    text_editor(&m.body).on_action(Message::MailBody).size(15).line_height(1.55).padding(0).style(theme::reader),
                );
            }
            for shown in images.iter().filter(|i| !i.is_large()) {
                content = content.push(mail_image(shown));
            }
            if m.images.is_loading() {
                content = content.push(loader("Loading images…"));
            }
            if m.images_failed > 0 {
                content = content.push(
                    muted(if m.images_failed == 1 { "1 image couldn't be shown.".to_string() } else { format!("{} images couldn't be shown.", m.images_failed) })
                        .size(13),
                );
            }
            if !msg.links.is_empty() {
                let links = Column::with_children(msg.links.iter().map(|(label, href)| {
                    button(
                        row![
                            accent_text(label.as_str()).size(13).font(theme::MEDIUM).wrapping(text::Wrapping::None),
                            muted(href.as_str()).size(12).wrapping(text::Wrapping::None),
                        ]
                        .spacing(10),
                    )
                    .padding([4, 6])
                    .style(theme::ghost_button)
                    .on_press(Message::OpenInBrowser(href.clone()))
                    .into()
                }))
                .spacing(2);
                content = content.push(horizontal_rule(1).style(theme::hairline)).push(
                    column![muted(format!("Links in this message ({})", msg.links.len())).size(12).font(theme::MEDIUM), links].spacing(6),
                );
            }
            let reader = column![
                text(msg.subject.as_str()).size(21).font(theme::SEMIBOLD),
                {
                    let meta = {
                        let mut meta = column![
                            text(msg.from.as_str()).size(14).font(theme::MEDIUM),
                            muted(if msg.to.is_empty() { "To undisclosed recipients".to_string() } else { format!("To {}", msg.to) }).size(13),
                        ]
                        .spacing(3)
                        .width(Length::Fill);
                        if !msg.cc_list.is_empty() {
                            meta = meta.push(muted(format!("Cc {}", msg.cc_list.iter().map(guc::mail::Mailbox::label).collect::<Vec<_>>().join(", "))).size(13));
                        }
                        meta.push(muted(format_date(&msg.received, true)).size(13))
                    };
                    let actions = row![
                        secondary("Reply", Message::ComposeStart(Compose::Reply)),
                        secondary("Reply all", Message::ComposeStart(Compose::ReplyAll)),
                        secondary("Forward", Message::ComposeStart(Compose::Forward)),
                    ]
                    .spacing(6);
                    let top: El<'_> = if narrow_reader { column![meta, actions].spacing(12).into() } else { row![meta, actions].spacing(12).into() };
                    top
                },
                horizontal_rule(1).style(theme::hairline),
                scrollable(content).height(Length::Fill),
            ]
            .spacing(16)
            .padding(24)
            .height(Length::Fill)
            .max_width(READING_WIDTH);
            container(reader).center_x(Length::Fill).height(Length::Fill).into()
        }
    };

    // Writing in a narrow window gets the whole width; the list comes back after.
    let mut panes = Row::new().spacing(16).height(Length::Fill);
    if !(m.composing && m.draft.is_some() && narrow_reader) {
        panes = panes.push(container(column![search, list].spacing(6)).width(list_width).height(Length::Fill).padding(6).style(theme::panel));
    }
    panes = panes.push(container(reader).width(Length::Fill).height(Length::Fill).style(theme::panel));
    column![header, panes]
    .spacing(20)
    .into()
}

fn compose(d: &Draft) -> El<'_> {
    let editable = !d.sending;
    let field = |placeholder: &'static str, value: &str, on_input: fn(String) -> Message| -> iced::widget::TextInput<'static, Message> {
        text_input(placeholder, value).on_input_maybe(editable.then_some(on_input)).padding(9).size(14).style(theme::input).width(Length::Fill)
    };
    let line = |name: &'static str, input: El<'static>| -> El<'static> {
        row![text(name).size(13).font(theme::MEDIUM).width(58), input].spacing(8).align_y(Alignment::Center).into()
    };
    let title = match d.kind {
        Compose::New => "New message",
        Compose::Reply => "Reply",
        Compose::ReplyAll => "Reply all",
        Compose::Forward => "Forward",
    };
    let mut heading = column![text(title).size(21).font(theme::SEMIBOLD)].spacing(3);
    if !d.about.is_empty() {
        heading = heading.push(muted(d.about.as_str()).size(13));
    }

    let mut to = row![field("name@guc.edu.eg", &d.to, Message::ComposeTo).id(COMPOSE_TO)].spacing(8).align_y(Alignment::Center);
    if !d.show_cc {
        to = to.push(button(text("Cc").size(13).font(theme::MEDIUM)).padding([8, 10]).style(theme::ghost_button).on_press_maybe(editable.then_some(Message::ComposeShowCc)));
    }
    let mut fields = column![line("To", to.into())].spacing(8);
    if d.show_cc {
        fields = fields.push(line("Cc", field("", &d.cc, Message::ComposeCc).into()));
    }
    fields = fields.push(line("Subject", field("", &d.subject, Message::ComposeSubject).into()));

    let mut content = column![heading].spacing(14);
    if let Some(e) = &d.error {
        content = content.push(banner(e.clone(), true, None));
    }
    content = content.push(fields).push(
        text_editor(&d.body)
            .placeholder("Write your message")
            .on_action(Message::ComposeBody)
            .height(Length::Fill)
            .padding(12)
            .size(15)
            .line_height(1.5)
            .style(theme::editor),
    );
    let note = match d.kind {
        Compose::New => None,
        Compose::Reply | Compose::ReplyAll => Some("The message you're replying to is quoted below your text when it's sent."),
        Compose::Forward => Some("The original message and its attachments are added below your text when it's sent."),
    };
    if let Some(note) = note {
        content = content.push(muted(note).size(12));
    }
    if !d.attachments.is_empty() {
        let mut files = Column::new().spacing(2);
        for (i, f) in d.attachments.iter().enumerate() {
            files = files.push(
                row![
                    text(f.name.as_str()).size(14).width(Length::Fill),
                    muted(file_size(f.bytes.len() as u64)).size(12),
                    button(text("Remove").size(13)).padding([4, 8]).style(theme::ghost_button).on_press_maybe(editable.then_some(Message::ComposeRemove(i))),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .padding([2, 8]),
            );
        }
        content = content.push(column![muted(format!("Attachments ({})", file_size(d.attachments_size() as u64))).size(12).font(theme::MEDIUM), files].spacing(6));
    }
    let discard = if d.confirm_discard { "Click again to discard" } else { "Discard" };
    let actions = row![
        button(text("Attach files").size(14)).padding([8, 14]).style(theme::secondary_button).on_press_maybe(editable.then_some(Message::ComposeAttach)),
        horizontal_space(),
        button(text(discard).size(14)).padding([8, 12]).style(theme::ghost_button).on_press_maybe(editable.then_some(Message::ComposeDiscard)),
        button(text(if d.sending { "Sending…" } else { "Send" }).size(14).font(theme::MEDIUM))
            .padding([8, 20])
            .style(theme::primary_button)
            .on_press_maybe(editable.then_some(Message::ComposeSend)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let content = content.push(actions).padding(24).height(Length::Fill).max_width(READING_WIDTH);
    container(content).center_x(Length::Fill).height(Length::Fill).into()
}

fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// A mail image at its natural size, shrunk to fit when the reader is narrower.
fn mail_image(shown: &crate::ShownImage) -> El<'_> {
    container(image(shown.handle.clone()).width(Length::Fill)).max_width(shown.width as f32).into()
}

fn mail_row(m: &guc::mail::Summary, selected: bool, compact: bool) -> El<'_> {
    let weight = if m.is_read { theme::REGULAR } else { theme::SEMIBOLD };
    let mut top = row![text(m.from.as_str()).size(14).font(weight).width(Length::Fill)].spacing(8);
    if m.has_attachments {
        top = top.push(muted("Attachment").size(11));
    }
    top = top.push(muted(format_date(&m.received, false)).size(12));
    let subject = if m.subject.is_empty() { "(no subject)" } else { m.subject.as_str() };
    let body = column![top, text(subject).size(13).font(if m.is_read { theme::REGULAR } else { theme::MEDIUM })].spacing(2);
    let msg = if compact { Message::Navigate(Screen::Mail) } else { Message::MailOpen(m.id.clone()) };
    button(body).padding([8, 10]).width(Length::Fill).style(theme::list_row(selected)).on_press(msg).into()
}

fn format_date(iso: &str, long: bool) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(iso) else { return iso.to_string() };
    let local = t.with_timezone(&chrono::Local);
    if long {
        return local.format("%A, %-d %B %Y at %H:%M").to_string();
    }
    if local.date_naive() == chrono::Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%-d %b").to_string()
    }
}

// ---------------------------------------------------------------- calculators

fn gpa(ws: &Workspace) -> El<'_> {
    let transcript_ready = ws.transcript_year.as_ref().and_then(|y| ws.transcripts.get(y)).is_some_and(|t| !t.seasons.is_empty());
    let header = page_header(
        "GPA calculator".into(),
        Some("GUC grades run from 0.7 (best) to 5.0 (fail). Lower is better.".into()),
        vec![button(text("Fill from transcript").size(14))
            .padding([8, 14])
            .style(theme::secondary_button)
            .on_press_maybe(transcript_ready.then_some(Message::GpaFromTranscript))
            .into()],
    );

    let head = row![
        muted("Course").size(12).width(Length::Fill),
        muted("Credit hours").size(12).width(110),
        muted("Grade").size(12).width(110),
        Space::with_width(40 + 80),
    ]
    .spacing(10);
    let mut rows = Column::new().spacing(8).push(head);
    for (i, r) in ws.gpa.rows.iter().enumerate() {
        let letter = portal::number(&r.grade).filter(|g| (0.7..=5.0).contains(g)).map(portal::letter).unwrap_or("");
        rows = rows.push(
            row![
                field("e.g. Databases I", &r.name, move |v| Message::Gpa(GpaMsg::Name(i, v))).width(Length::Fill),
                field("6", &r.hours, move |v| Message::Gpa(GpaMsg::Hours(i, v))).width(110),
                field("1.3", &r.grade, move |v| Message::Gpa(GpaMsg::Grade(i, v))).width(110),
                accent_text(letter).size(14).font(theme::MEDIUM).width(40),
                button(text("Remove").size(13)).padding([6, 10]).style(theme::ghost_button).width(80).on_press(Message::Gpa(GpaMsg::Remove(i))),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
    }
    rows = rows.push(row![secondary("Add course", Message::Gpa(GpaMsg::Add))]);

    let result = match ws.gpa.result() {
        Some((g, hours)) => column![
            muted("Your GPA").size(13),
            row![text(format!("{g:.2}")).size(40).font(theme::SEMIBOLD), accent_text(portal::letter(g)).size(20).font(theme::MEDIUM)]
                .spacing(12)
                .align_y(Alignment::End),
            muted(format!("Across {hours} credit hours")).size(13),
        ]
        .spacing(4),
        None => column![muted("Enter credit hours and a grade between 0.7 and 5.0 to see your GPA.").size(14)],
    };

    column![header, main_and_side(ws, panel(rows), panel(result))].spacing(20).into()
}

fn grade_calc(ws: &Workspace) -> El<'_> {
    let header = page_header(
        "Grade calculator".into(),
        Some("Enter each part of the course with its weight. Leave the score empty for anything not graded yet.".into()),
        vec![],
    );
    let head = row![
        muted("Component").size(12).width(Length::Fill),
        muted("Weight %").size(12).width(100),
        muted("Score").size(12).width(100),
        muted("Out of").size(12).width(100),
        Space::with_width(80),
    ]
    .spacing(10);
    let mut rows = Column::new().spacing(8).push(head);
    for (i, p) in ws.calc.parts.iter().enumerate() {
        rows = rows.push(
            row![
                field("e.g. Quizzes", &p.name, move |v| Message::Calc(CalcMsg::Name(i, v))).width(Length::Fill),
                field("15", &p.weight, move |v| Message::Calc(CalcMsg::Weight(i, v))).width(100),
                field("—", &p.score, move |v| Message::Calc(CalcMsg::Score(i, v))).width(100),
                field("—", &p.out_of, move |v| Message::Calc(CalcMsg::OutOf(i, v))).width(100),
                button(text("Remove").size(13)).padding([6, 10]).style(theme::ghost_button).width(80).on_press(Message::Calc(CalcMsg::Remove(i))),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
    }
    rows = rows.push(row![secondary("Add component", Message::Calc(CalcMsg::Add))]);

    let s = ws.calc.summary();
    let mut summary = column![].spacing(10);
    if (s.total_weight - 100.0).abs() > 0.01 {
        summary = summary.push(banner(format!("Weights add up to {:.0}%, not 100%.", s.total_weight), true, None));
    }
    let pair = |a: El<'static>, b: El<'static>| {
        row![container(a).width(Length::FillPortion(1)), container(b).width(Length::FillPortion(1))].spacing(16)
    };
    summary = summary
        .push(pair(stat("Secured so far", format!("{:.1}%", s.earned)), stat("Still open", format!("{:.1}%", s.remaining))))
        .push(pair(
            stat("Best possible", format!("{:.1}%", s.best_possible().min(s.total_weight))),
            stat("Average on graded work", s.standing().map(|v| format!("{v:.1}%")).unwrap_or_else(|| "—".into())),
        ));
    if s.graded_weight > 0.0 && s.remaining > 0.0 {
        summary = summary.push(muted(format!(
            "Keeping your current average on the remaining {:.0}% would finish the course at about {:.1}%.",
            s.remaining,
            s.earned + s.remaining * s.standing().unwrap_or(0.0) / 100.0
        )).size(14));
    }

    column![header, main_and_side(ws, panel(rows), panel(summary))].spacing(20).into()
}

// ---------------------------------------------------------------- settings

fn settings<'a>(app: &'a App, ws: &'a Workspace) -> El<'a> {
    let mut picker = Column::new().spacing(8);
    let mut cards = AppTheme::ALL.into_iter().map(|choice| theme_choice(choice, app.settings.theme == choice));
    loop {
        let line: Vec<El<'a>> = cards.by_ref().take(if ws.content_width() >= 1120.0 { 8 } else { 4 }).collect();
        if line.is_empty() {
            break;
        }
        picker = picker.push(Row::with_children(line).spacing(8));
    }

    let account = column![
            subheading("Account"),
            text(format!("Signed in as {}", ws.client.0.username())).size(14),
            muted("Signing out also removes the saved password from this computer.").size(13),
            row![secondary("Sign out", Message::SignOut)],
        ].spacing(10);
    let mut downloads = column![
            subheading("Downloads"),
            muted("Course files and mail attachments are saved here.").size(13),
            text(app.settings.downloads().display().to_string()).size(14).font(theme::MEDIUM),
            row![
                button(text(if app.picking_folder { "Choosing…" } else { "Change folder…" }).size(14))
                    .padding([8, 14])
                    .style(theme::secondary_button)
                    .on_press_maybe((!app.picking_folder).then_some(Message::ChooseDownloadFolder)),
                ghost("Open folder", Message::OpenDownloadFolder),
            ]
            .spacing(8),
        ].spacing(10);
    if let Some(e) = &app.folder_error {
        downloads = downloads.push(banner(e.clone(), true, None));
    }
    let privacy = column![
            subheading("Privacy"),
            muted("GUC Station talks only to apps.guc.edu.eg, cms.guc.edu.eg and mail.guc.edu.eg. Your grades, mail and password never pass through any other server.").size(14),
        ].spacing(10);
    let troubleshooting = column![
            subheading("Troubleshooting"),
            muted("Saves copies of the portal and CMS pages this app reads, so a screen that looks wrong can be fixed against the real page. They include your personal details and stay on this computer unless you share them.").size(14),
            row![button(text(if ws.saving_pages { "Saving pages…" } else { "Save pages for troubleshooting" }).size(14))
                .padding([8, 14])
                .style(theme::secondary_button)
                .on_press_maybe((!ws.saving_pages).then_some(Message::SaveDiagnostics))],
        ].spacing(10);

    column![
        page_header("Settings".into(), None, vec![]),
        panel(column![subheading("Theme"), picker].spacing(12)),
        card_grid(vec![account.into(), downloads.into(), privacy.into(), troubleshooting.into()], ws.columns(340.0)),
    ]
    .spacing(COLUMN_GAP)
    .into()
}

/// A picker card: a tiny drawing of the app in that theme's own colours.
fn theme_choice<'a>(choice: AppTheme, selected: bool) -> El<'a> {
    let t = tokens(&choice.build());
    let bar = |w: f32, h: f32, color| container(Space::new(w, h)).style(theme::swatch(color, 2.0, None));
    let preview = container(row![
        container(column![bar(16.0, 4.0, t.muted), bar(16.0, 4.0, t.accent), bar(16.0, 4.0, t.muted)].spacing(5))
            .padding([10, 7])
            .height(Length::Fill)
            .style(theme::swatch(t.sidebar, 0.0, None)),
        container(column![bar(52.0, 6.0, t.text), bar(34.0, 4.0, t.muted), bar(24.0, 8.0, t.accent)].spacing(6))
            .padding(10)
            .width(Length::Fill),
    ])
    .width(Length::Fill)
    .height(64)
    .clip(true)
    .style(theme::swatch(t.bg, 6.0, Some(t.border)));
    button(column![preview, text(choice.name()).size(13).font(if selected { theme::SEMIBOLD } else { theme::REGULAR })].spacing(7))
        .padding(8)
        .width(Length::FillPortion(1))
        .style(theme::theme_card(selected))
        .on_press(Message::ThemePicked(choice))
        .into()
}

// ---------------------------------------------------------------- pieces

pub(crate) fn page_header<'a>(title: String, subtitle: Option<String>, actions: Vec<El<'a>>) -> El<'a> {
    let mut titles = column![text(title).size(26).font(theme::SEMIBOLD)].spacing(4);
    if let Some(s) = subtitle {
        titles = titles.push(muted(s).size(14));
    }
    row![titles.width(Length::Fill), Row::with_children(actions).spacing(8)].align_y(Alignment::Center).spacing(16).into()
}

/// Renders a `Load` state; `f` draws the loaded value.
pub(crate) fn loaded<'a, T>(load: &'a Load<T>, loading: &'static str, retry: Message, f: impl FnOnce(&'a T) -> El<'a>) -> El<'a> {
    match load {
        Load::Idle | Load::Loading => container(loader(loading)).padding([8, 0]).into(),
        Load::Failed(e) => column![banner(e.clone(), true, None), row![secondary("Try again", retry)]].spacing(10).into(),
        Load::Ready(v) => f(v),
    }
}

/// Cards in an even grid: equal widths, and every card in a row as tall as
/// the tallest. Four cards use four or two columns, never three with one
/// left over.
fn card_grid<'a>(cards: Vec<El<'a>>, columns: usize) -> El<'a> {
    let columns = match columns.min(cards.len()) {
        n if n >= 4 => 4,
        n if n >= 2 => 2,
        _ => 1,
    };
    let mut grid = Column::new().spacing(COLUMN_GAP).width(Length::Fill);
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let line: Vec<El<'a>> = cards
            .by_ref()
            .take(columns)
            .map(|card| container(card).padding(20).width(Length::Fill).style(theme::panel).into())
            .collect();
        grid = grid.push(crate::equal_row::EqualRow::new(line, columns, COLUMN_GAP));
    }
    grid.into()
}

pub(crate) fn panel<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content).padding(20).width(Length::Fill).style(theme::panel).into()
}

pub(crate) fn banner<'a>(msg: String, error: bool, dismiss: Option<Message>) -> El<'a> {
    let mut r = row![text(msg).size(14).width(Length::Fill)].spacing(12).align_y(Alignment::Center);
    if let Some(m) = dismiss {
        r = r.push(button(text("Dismiss").size(13)).padding([4, 10]).style(theme::ghost_button).on_press(m));
    }
    container(r)
        .padding([10, 14])
        .width(Length::Fill)
        .style(if error { theme::banner_error } else { theme::banner_info })
        .into()
}

pub(crate) fn stat<'a>(label: &'a str, value: String) -> El<'a> {
    column![muted(label).size(13), text(value).size(24).font(theme::SEMIBOLD)].spacing(4).into()
}

fn field<'a>(placeholder: &'a str, value: &'a str, on_input: impl Fn(String) -> Message + 'a) -> iced::widget::TextInput<'a, Message> {
    text_input(placeholder, value).on_input(on_input).padding(9).size(14).style(theme::input)
}

pub(crate) fn secondary<'a>(label: &'a str, msg: Message) -> El<'a> {
    button(text(label).size(14)).padding([8, 14]).style(theme::secondary_button).on_press(msg).into()
}

pub(crate) fn ghost<'a>(label: &'a str, msg: Message) -> El<'a> {
    button(text(label).size(14)).padding([8, 12]).style(theme::ghost_button).on_press(msg).into()
}

pub(crate) fn link<'a>(label: &'a str, msg: Message) -> El<'a> {
    button(text(label).size(14).font(theme::MEDIUM)).padding(0).style(theme::link_button).on_press(msg).into()
}

pub(crate) fn subheading<'a>(s: &'a str) -> iced::widget::Text<'a> {
    text(s).size(16).font(theme::SEMIBOLD)
}

fn label<'a>(s: &'a str) -> iced::widget::Text<'a> {
    text(s).size(13).font(theme::MEDIUM)
}

pub(crate) fn muted<'a>(s: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(s).style(|t: &Theme| text::Style { color: Some(tokens(t).muted) })
}

pub(crate) fn accent_text<'a>(s: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(s).style(|t: &Theme| text::Style { color: Some(tokens(t).accent) })
}

/// "ahmed.ali" -> "Ahmed".
fn display_name(username: &str) -> String {
    let first = username.split(['.', '_', ' ']).next().unwrap_or(username);
    let mut c = first.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

