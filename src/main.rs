#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod equal_row;
mod screens;
mod settings;
mod theme;
mod tools;
mod views;

use guc::academic::{self, CourseAttendance, CourseGrades, Evaluation};
use guc::client::{Credentials, GucClient};
use guc::mail::{self, Folder};
use guc::page::{self, Choice, Page, Target};
use guc::portal::{self, Section};
use guc::creds;
use iced::widget::{scrollable, text_editor};
use iced::{Size, Subscription, Task, Theme, window};
use settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use url::Url;

pub const MAIL_PAGE_SIZE: u32 = 40;

fn initial_size() -> Size {
    Size::new(1280.0, 820.0)
}

fn main() -> iced::Result {
    iced::application("GUC Station", App::update, App::view)
        .theme(App::theme)
        .font(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice())
        .default_font(theme::REGULAR)
        .window(window::Settings {
            size: initial_size(),
            min_size: Some(Size::new(960.0, 640.0)),
            icon: window::icon::from_file_data(include_bytes!("../assets/brand/app-icon-256.png"), None).ok(),
            ..Default::default()
        })
        .subscription(App::subscription)
        .antialiasing(true)
        .run_with(App::new)
}

/// Shared handle to the signed-in HTTP client.
#[derive(Clone)]
pub struct Client(Arc<GucClient>);

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client({})", self.0.username())
    }
}

#[derive(Debug, Clone, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }

    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

impl<T> From<Result<T, String>> for Load<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Section(Section),
    Cms,
    Mail,
    Gpa,
    GradeCalc,
    Settings,
}

#[derive(Default)]
pub struct LoginForm {
    pub login: String,
    pub password: String,
    pub remember: bool,
    pub busy: bool,
    pub error: Option<String>,
}

pub struct CmsState {
    pub home: Load<Page>,
    pub courses: Vec<academic::CmsCourse>,
    pub course: Option<(String, Load<Page>)>,
    pub weeks: Vec<academic::CmsWeek>,
    pub filter: String,
    /// Content type filter, e.g. "Lecture slides"; None shows everything.
    pub kind: Option<String>,
}

pub struct MailState {
    pub folder: Folder,
    pub offset: u32,
    pub list: Load<(Vec<mail::Summary>, u32)>,
    pub open_id: Option<String>,
    pub open: Load<mail::Message>,
    /// The open message's body, in a read-only editor so it can be selected and copied.
    pub body: text_editor::Content,
    pub images: Load<Vec<ShownImage>>,
    /// Images in the open message that couldn't be downloaded or decoded.
    pub images_failed: usize,
    /// What's typed in the search box, and the search the list currently shows.
    pub query: String,
    pub active_query: String,
    /// A message being written. It's kept while the user reads other mail,
    /// until it's sent or discarded; `composing` says whether it's on screen.
    pub draft: Option<Draft>,
    pub composing: bool,
}

pub struct Draft {
    pub kind: mail::Compose,
    /// The message being answered or forwarded: its id and change key.
    pub original: Option<(String, String)>,
    /// What the reply or forward is about, shown under the heading.
    pub about: String,
    pub to: String,
    pub cc: String,
    pub show_cc: bool,
    pub subject: String,
    pub body: text_editor::Content,
    pub attachments: Vec<mail::OutFile>,
    pub sending: bool,
    pub error: Option<String>,
    /// Discarding something typed takes a second click.
    pub confirm_discard: bool,
}

/// GUC's mail server turns away bigger messages, so the app stops before that.
pub const MAX_ATTACHMENTS_BYTES: usize = 10 * 1024 * 1024;

impl Draft {
    /// A new message, or a reply or forward with its recipients and subject filled in.
    pub fn new(kind: mail::Compose, original: Option<&mail::Message>, username: &str) -> Draft {
        use mail::Compose;
        let me = |m: &mail::Mailbox| mail::is_me(m, username);
        let (mut to, mut cc) = (Vec::new(), Vec::new());
        let (mut subject, mut about) = (String::new(), String::new());
        if let Some(m) = original {
            let from_me = m.sender.as_ref().is_some_and(me);
            match kind {
                Compose::Reply | Compose::ReplyAll => {
                    // Answering your own sent mail goes to the people it went to.
                    if from_me {
                        to.extend(m.to_list.iter().filter(|r| !me(r)).cloned());
                    } else {
                        to.extend(m.sender.clone());
                    }
                    if kind == Compose::ReplyAll {
                        if !from_me {
                            to.extend(m.to_list.iter().filter(|r| !me(r)).cloned());
                        }
                        cc.extend(m.cc_list.iter().filter(|r| !me(r)).cloned());
                    }
                    subject = prefixed("RE:", &[], &m.subject);
                    about = format!("Replying to {}", if m.from.is_empty() { "this message" } else { m.from.as_str() });
                }
                Compose::Forward => {
                    subject = prefixed("FW:", &["fwd:"], &m.subject);
                    about = format!("Forwarding \"{}\"", if m.subject.is_empty() { "(no subject)" } else { m.subject.as_str() });
                }
                Compose::New => {}
            }
        }
        // No one twice, and only addresses that can be typed back in.
        let mut seen = std::collections::HashSet::new();
        let mut keep = |list: Vec<mail::Mailbox>| -> String {
            list.into_iter()
                .filter(|r| r.is_smtp() && seen.insert(r.address.to_lowercase()))
                .map(|r| mail::address_text(&r))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let to = keep(to);
        let cc = keep(cc);
        Draft {
            kind,
            original: original.filter(|_| kind != Compose::New).map(|m| (m.id.clone(), m.change_key.clone())),
            about,
            to,
            show_cc: !cc.is_empty(),
            cc,
            subject,
            body: text_editor::Content::new(),
            attachments: Vec::new(),
            sending: false,
            error: None,
            confirm_discard: false,
        }
    }

    /// Whether the user has written or attached anything that discarding would lose.
    pub fn has_content(&self) -> bool {
        !self.body.text().trim().is_empty() || !self.attachments.is_empty() || (self.kind == mail::Compose::New && (!self.to.trim().is_empty() || !self.subject.trim().is_empty()))
    }

    pub fn attachments_size(&self) -> usize {
        self.attachments.iter().map(|f| f.bytes.len()).sum()
    }
}

/// "RE: Subject", without stacking prefixes on a thread that already has one.
fn prefixed(prefix: &str, also: &[&str], subject: &str) -> String {
    let lower = subject.trim_start().to_lowercase();
    if lower.starts_with(&prefix.to_lowercase()) || also.iter().any(|a| lower.starts_with(a)) {
        subject.to_string()
    } else if subject.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} {subject}")
    }
}

pub const COMPOSE_TO: &str = "compose-to";

/// A mail image, with its handle made once so the renderer can cache it.
pub struct ShownImage {
    pub handle: iced::widget::image::Handle,
    pub width: u32,
    pub height: u32,
}

impl ShownImage {
    /// Posters and flyers go above the text; signature logos stay below it.
    pub fn is_large(&self) -> bool {
        self.width >= 400 && self.height >= 300
    }
}

pub struct Workspace {
    /// Where downloads are saved; the app keeps it in step with Settings.
    pub downloads: PathBuf,
    pub client: Client,
    pub screen: Screen,
    pub sections: HashMap<Section, Url>,
    pub pages: HashMap<Section, Load<Page>>,
    /// Pages left by following a link, for the Back button.
    pub history: HashMap<Section, Vec<Page>>,
    pub cms: CmsState,
    pub mail: MailState,
    pub gpa: tools::GpaCalc,
    pub calc: tools::GradeCalc,
    pub notice: Option<Notice>,
    /// Parsed from the schedule and exam pages when they load.
    pub schedule: Option<academic::Schedule>,
    pub exams: Option<Vec<academic::Exam>>,
    /// Every course's grades / attendance, gathered by picking each course in turn.
    pub grades: Load<Vec<CourseGrades>>,
    pub attendance: Load<Vec<CourseAttendance>>,
    /// (courses, staff) evaluation status.
    pub evaluations: Load<(Vec<Evaluation>, Vec<Evaluation>)>,
    /// Transcript years fetched so far, by label ("2025-2026"). The portal
    /// throttles this page, so each year is fetched once per session.
    pub transcripts: HashMap<String, academic::Transcript>,
    pub transcript_year: Option<String>,
    /// The year being fetched right now.
    pub transcript_loading: Option<String>,
    /// Cumulative GPA, which every transcript page repeats.
    pub cumulative: Option<(String, f32)>,
    /// Set once the opening year turned out empty and we stepped back a year.
    pub transcript_stepped_back: bool,
    /// A popup opened from a portal page, e.g. one notification.
    pub detail: Option<Detail>,
    pub saving_pages: bool,
    pub size: Size,
}

#[derive(Debug, Clone)]
pub enum Notice {
    Info(String),
    Error(String),
}

pub struct Detail {
    pub title: String,
    pub by: String,
    pub date: String,
    pub body: text_editor::Content,
}

impl Detail {
    /// Builds a detail view from a popup's data-* attributes. The keys are the
    /// ones GUC's notification popup uses, with generic fallbacks.
    fn from_data(data: &[(String, String)]) -> Self {
        let get = |keys: &[&str]| {
            keys.iter().find_map(|k| data.iter().find(|(dk, _)| dk == k)).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        let body = get(&["body_text", "body", "content", "description"]);
        let body = if body.contains('<') { mail::html_to_text(&body) } else { body };
        Detail {
            title: get(&["subject_text", "subject", "title"]),
            by: get(&["staff_full_name", "staff", "by"]),
            date: get(&["email_time", "date", "time"]),
            body: text_editor::Content::with_text(body.trim()),
        }
    }
}

pub fn content_scroll_id() -> scrollable::Id {
    scrollable::Id::new("content")
}

fn scroll_to_top() -> Task<Message> {
    scrollable::snap_to(content_scroll_id(), scrollable::RelativeOffset::START)
}

pub enum Session {
    SignedOut(LoginForm),
    SignedIn(Box<Workspace>),
}

pub struct App {
    /// A problem creating the chosen downloads folder, shown where it was chosen.
    pub folder_error: Option<String>,
    /// The folder dialog is open, so its buttons wait.
    pub picking_folder: bool,
    pub settings: Settings,
    pub session: Session,
    /// Current window size; layouts adapt to it.
    pub size: Size,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Sign-in
    LoginChanged(String),
    PasswordChanged(String),
    RememberToggled(bool),
    SignIn,
    SignedIn(Result<(Client, Page), String>),
    SignOut,

    Navigate(Screen),
    Refresh,
    SectionLoaded(Section, Result<Page, String>),
    DropdownPicked(Section, String, Choice),
    Follow(Section, Target),
    Back(Section),
    SavePageHtml(Section),
    OpenInBrowser(String),
    DismissNotice,

    CmsLoaded(Result<Page, String>),
    OpenCourse(String, Url),
    CourseLoaded(Result<Page, String>),
    CloseCourse,
    CmsFilter(String),
    CmsKind(Option<String>),
    GradesLoaded(Result<Vec<CourseGrades>, String>),
    TranscriptYear(String),
    TranscriptLoaded(String, Result<Page, String>),
    AttendanceLoaded(Result<Vec<CourseAttendance>, String>),
    EvaluationsLoaded(Result<(Vec<Evaluation>, Vec<Evaluation>), String>),
    /// A CMS item: its URL and the title shown for it.
    OpenLink(Url, String),
    Downloaded(Result<PathBuf, String>),

    MailFolder(Folder),
    MailPage(u32),
    MailQuery(String),
    MailSearch,
    MailClearSearch,
    MailLoaded(Result<(Vec<mail::Summary>, u32), String>),
    MailOpen(String),
    MailMessage(Result<mail::Message, String>),
    MailImages(String, Vec<mail::MailImage>, usize),
    SaveAttachment(mail::Attachment),
    AttachmentSaved(Result<PathBuf, String>),
    MailBody(text_editor::Action),
    ComposeStart(mail::Compose),
    ComposeResume,
    ComposeTo(String),
    ComposeCc(String),
    ComposeShowCc,
    ComposeSubject(String),
    ComposeBody(text_editor::Action),
    ComposeAttach,
    ComposeAttached(Result<Vec<mail::OutFile>, String>),
    ComposeRemove(usize),
    ComposeSend,
    ComposeSent(Result<(), String>),
    ComposeDiscard,
    DetailBody(text_editor::Action),
    CloseDetail,

    Gpa(tools::GpaMsg),
    GpaFromTranscript,
    Calc(tools::CalcMsg),
    SaveDiagnostics,
    DiagnosticsSaved(Result<(usize, PathBuf), String>),

    ThemePicked(theme::AppTheme),
    /// Keeps the suggested downloads folder on first start.
    UseDownloadFolder(PathBuf),
    ChooseDownloadFolder,
    DownloadFolderPicked(Option<Result<PathBuf, String>>),
    OpenDownloadFolder,
    /// Advances the loading animation.
    Tick,
    Resized(Size),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let settings = Settings::load();
        match creds::load() {
            Some((login, password)) => {
                let form = LoginForm { login: login.clone(), password: password.clone(), remember: true, busy: true, error: None };
                (App { settings, session: Session::SignedOut(form), size: initial_size(), folder_error: None, picking_folder: false }, sign_in(login, password))
            }
            None => (
                App { settings, session: Session::SignedOut(LoginForm { remember: true, ..Default::default() }), size: initial_size(), folder_error: None, picking_folder: false },
                Task::none(),
            ),
        }
    }

    fn theme(&self) -> Theme {
        self.settings.theme.build()
    }

    /// Ticks only while something is loading, so an idle app doesn't redraw.
    fn subscription(&self) -> Subscription<Message> {
        let busy = match &self.session {
            Session::SignedOut(form) => form.busy,
            Session::SignedIn(ws) => ws.is_busy(),
        };
        let resized = window::resize_events().map(|(_, size)| Message::Resized(size));
        if busy {
            let tick = iced::time::every(std::time::Duration::from_millis(views::LOADER_FRAME_MS)).map(|_| Message::Tick);
            Subscription::batch([resized, tick])
        } else {
            resized
        }
    }

    fn view(&self) -> iced::Element<'_, Message> {
        views::root(self)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                views::advance_loader();
                return Task::none();
            }
            Message::Resized(size) => {
                self.size = size;
                if let Session::SignedIn(ws) = &mut self.session {
                    ws.size = size;
                }
                return Task::none();
            }
            Message::ThemePicked(choice) => {
                self.settings.theme = choice;
                self.settings.save();
                return Task::none();
            }
            Message::UseDownloadFolder(dir) => {
                self.picking_folder = true;
                return Task::perform(tools::create_download_folder(dir), |r| Message::DownloadFolderPicked(Some(r)));
            }
            Message::ChooseDownloadFolder => {
                if self.picking_folder {
                    return Task::none();
                }
                self.picking_folder = true;
                return Task::perform(tools::pick_download_folder(self.settings.downloads()), Message::DownloadFolderPicked);
            }
            Message::DownloadFolderPicked(r) => {
                self.picking_folder = false;
                match r {
                    None => {}
                    Some(Err(e)) => self.folder_error = Some(e),
                    Some(Ok(dir)) => {
                        let changed = self.settings.download_dir.is_some();
                        self.folder_error = None;
                        self.settings.download_dir = Some(dir.clone());
                        self.settings.save();
                        if let Session::SignedIn(ws) = &mut self.session {
                            if changed {
                                ws.notice = Some(Notice::Info(format!("Downloads now go to {}. Files saved before stay where they are.", dir.display())));
                            }
                            ws.downloads = dir;
                        }
                    }
                }
                return Task::none();
            }
            Message::OpenDownloadFolder => {
                let dir = self.settings.downloads();
                let _ = std::fs::create_dir_all(&dir);
                let _ = opener::open(&dir);
                return Task::none();
            }
            Message::OpenInBrowser(url) => {
                // mailto: and friends go to their own handler, not the browser.
                let _ = if url.starts_with("http") { opener::open_browser(url) } else { opener::open(url) };
                return Task::none();
            }
            _ => {}
        }

        match &mut self.session {
            Session::SignedOut(form) => match message {
                Message::LoginChanged(v) => form.login = v,
                Message::PasswordChanged(v) => form.password = v,
                Message::RememberToggled(v) => form.remember = v,
                Message::SignIn if !form.busy && !form.login.trim().is_empty() && !form.password.is_empty() => {
                    form.busy = true;
                    form.error = None;
                    return sign_in(form.login.clone(), form.password.clone());
                }
                Message::SignedIn(Ok((client, home))) => {
                    if form.remember {
                        let _ = creds::save(&form.login, &form.password);
                    }
                    let mut ws = Workspace::new(client, &home, self.size, self.settings.downloads());
                    let task = ws.open(Screen::Home);
                    self.session = Session::SignedIn(Box::new(ws));
                    return task;
                }
                Message::SignedIn(Err(e)) => {
                    form.busy = false;
                    form.error = Some(e);
                    form.password.clear();
                }
                _ => {}
            },
            Session::SignedIn(ws) => {
                if let Message::SignOut = message {
                    creds::forget();
                    self.session = Session::SignedOut(LoginForm { remember: true, ..Default::default() });
                    return Task::none();
                }
                return ws.update(message);
            }
        }
        Task::none()
    }
}

impl Workspace {
    fn new(client: Client, home: &Page, size: Size, downloads: PathBuf) -> Self {
        Workspace {
            downloads,
            client,
            screen: Screen::Home,
            sections: portal::discover(Some(home)),
            pages: HashMap::new(),
            history: HashMap::new(),
            cms: CmsState { home: Load::Idle, courses: Vec::new(), course: None, weeks: Vec::new(), filter: String::new(), kind: None },
            mail: MailState { folder: Folder::Inbox, offset: 0, list: Load::Idle, open_id: None, open: Load::Idle, body: text_editor::Content::new(), images: Load::Idle, images_failed: 0, query: String::new(), active_query: String::new(), draft: None, composing: false },
            gpa: tools::GpaCalc::default(),
            calc: tools::GradeCalc::default(),
            notice: None,
            schedule: None,
            exams: None,
            grades: Load::Idle,
            attendance: Load::Idle,
            evaluations: Load::Idle,
            transcripts: HashMap::new(),
            transcript_year: None,
            transcript_loading: None,
            cumulative: None,
            transcript_stepped_back: false,
            detail: None,
            saving_pages: false,
            size,
        }
    }

    /// Width available to a page's content.
    pub fn content_width(&self) -> f32 {
        self.size.width - views::SIDEBAR_WIDTH - views::PAGE_PADDING_X * 2.0 - views::SCROLLBAR_ROOM
    }

    /// How many columns of at least `min_width` fit on the page.
    pub fn columns(&self, min_width: f32) -> usize {
        (((self.content_width() + views::COLUMN_GAP) / (min_width + views::COLUMN_GAP)) as usize).max(1)
    }

    fn is_busy(&self) -> bool {
        self.pages.values().any(Load::is_loading)
            || self.grades.is_loading()
            || self.attendance.is_loading()
            || self.evaluations.is_loading()
            || self.cms.home.is_loading()
            || self.cms.course.as_ref().is_some_and(|(_, load)| load.is_loading())
            || self.mail.list.is_loading()
            || self.mail.open.is_loading()
            || self.mail.images.is_loading()
            || self.mail.draft.as_ref().is_some_and(|d| d.sending)
            || self.transcript_loading.is_some()
    }

    pub fn page(&self, s: Section) -> &Load<Page> {
        static IDLE: Load<Page> = Load::Idle;
        self.pages.get(&s).unwrap_or(&IDLE)
    }

    /// Switches screen and starts whatever loads it needs that haven't run yet.
    fn open(&mut self, screen: Screen) -> Task<Message> {
        // Home shares this list, so a search can't outlive the mail page.
        if screen != Screen::Mail && !self.mail.active_query.is_empty() {
            self.mail.query.clear();
            self.mail.active_query.clear();
            self.mail.offset = 0;
            self.mail.list = Load::Idle;
        }
        self.screen = screen;
        self.detail = None;
        let load = match screen {
            Screen::Home => Task::batch([
                self.load_section(Section::Notifications, false),
                self.load_section(Section::Schedule, false),
                self.load_mail(false),
            ]),
            Screen::Section(Section::Grades) => self.load_grades(false),
            Screen::Section(Section::Attendance) => self.load_attendance(false),
            Screen::Section(Section::CourseEvaluation | Section::StaffEvaluation) => self.load_evaluations(false),
            Screen::Section(s) => self.load_section(s, false),
            Screen::Cms => self.load_cms(false),
            Screen::Mail => self.load_mail(false),
            Screen::Gpa => self.load_section(Section::Transcript, false),
            Screen::GradeCalc | Screen::Settings => Task::none(),
        };
        Task::batch([load, scroll_to_top()])
    }

    fn load_section(&mut self, s: Section, force: bool) -> Task<Message> {
        if !force && !matches!(self.page(s), Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.pages.insert(s, Load::Loading);
        let client = self.client.clone();
        let url = self.sections[&s].to_string();
        Task::perform(async move { page::fetch(&client.0, &url).await.map_err(|e| e.to_string()) }, move |r| {
            Message::SectionLoaded(s, r)
        })
    }

    fn load_grades(&mut self, force: bool) -> Task<Message> {
        if !force && !matches!(self.grades, Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.grades = Load::Loading;
        let (client, url) = (self.client.clone(), self.sections[&Section::Grades].to_string());
        Task::perform(async move { academic::all_grades(&client.0, &url).await.map_err(|e| e.to_string()) }, Message::GradesLoaded)
    }

    fn load_attendance(&mut self, force: bool) -> Task<Message> {
        if !force && !matches!(self.attendance, Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.attendance = Load::Loading;
        let (client, url) = (self.client.clone(), self.sections[&Section::Attendance].to_string());
        Task::perform(
            async move { academic::all_attendance(&client.0, &url).await.map_err(|e| e.to_string()) },
            Message::AttendanceLoaded,
        )
    }

    fn load_evaluations(&mut self, force: bool) -> Task<Message> {
        if !force && !matches!(self.evaluations, Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.evaluations = Load::Loading;
        let client = self.client.clone();
        let courses = self.sections[&Section::CourseEvaluation].to_string();
        let staff = self.sections[&Section::StaffEvaluation].to_string();
        Task::perform(
            async move {
                let c = academic::evaluation_status(&client.0, &courses).await.map_err(|e| e.to_string())?;
                let s = academic::evaluation_status(&client.0, &staff).await.map_err(|e| e.to_string())?;
                Ok((c, s))
            },
            Message::EvaluationsLoaded,
        )
    }

    /// Shows a transcript year, fetching it unless it is already cached.
    fn select_year(&mut self, label: String) -> Task<Message> {
        if self.transcripts.contains_key(&label) {
            self.transcript_year = Some(label);
            return Task::none();
        }
        let Some(base) = self.page(Section::Transcript).ready().cloned() else { return Task::none() };
        let Some((name, value)) = base.dropdowns.first().and_then(|d| {
            d.choices.iter().find(|c| c.label == label).map(|c| (d.name.clone(), c.value.clone()))
        }) else {
            return Task::none();
        };
        self.transcript_loading = Some(label.clone());
        let client = self.client.clone();
        Task::perform(
            async move { base.post_back(&client.0, &name, &[(name.clone(), value)]).await.map_err(|e| e.to_string()) },
            move |r| Message::TranscriptLoaded(label.clone(), r),
        )
    }

    /// Labels of the transcript's study years, newest first.
    pub fn transcript_years(&self) -> Vec<String> {
        self.page(Section::Transcript)
            .ready()
            .and_then(|p| p.dropdowns.first())
            .map(|d| d.choices.iter().filter(|c| !c.value.is_empty()).map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    }

    fn load_cms(&mut self, force: bool) -> Task<Message> {
        if !force && !matches!(self.cms.home, Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.cms.home = Load::Loading;
        let client = self.client.clone();
        Task::perform(async move { page::fetch(&client.0, portal::CMS_HOME).await.map_err(|e| e.to_string()) }, Message::CmsLoaded)
    }

    /// Changes to the open draft. Nothing changes while it's being sent.
    fn edit_draft(&mut self, message: Message) -> Task<Message> {
        let Some(d) = &mut self.mail.draft else { return Task::none() };
        if d.sending {
            if let Message::ComposeBody(action) = message
                && !action.is_edit()
            {
                d.body.perform(action);
            }
            return Task::none();
        }
        if !matches!(message, Message::ComposeDiscard) {
            d.confirm_discard = false;
        }
        match message {
            Message::ComposeTo(v) => d.to = v,
            Message::ComposeCc(v) => d.cc = v,
            Message::ComposeShowCc => d.show_cc = true,
            Message::ComposeSubject(v) => d.subject = v,
            Message::ComposeBody(action) => d.body.perform(action),
            Message::ComposeRemove(i) => {
                if i < d.attachments.len() {
                    d.attachments.remove(i);
                }
            }
            Message::ComposeAttach => return Task::perform(tools::pick_attachments(), Message::ComposeAttached),
            Message::ComposeDiscard => {
                if d.confirm_discard || !d.has_content() {
                    self.mail.draft = None;
                    self.mail.composing = false;
                } else {
                    d.confirm_discard = true;
                }
            }
            Message::ComposeSend => {
                let recipients = mail::parse_addresses(&d.to).and_then(|to| Ok((to, mail::parse_addresses(&d.cc)?)));
                let (to, cc) = match recipients {
                    Ok((to, cc)) if to.is_empty() && cc.is_empty() => {
                        d.error = Some("Add who the message is to.".into());
                        return Task::none();
                    }
                    Ok(r) => r,
                    Err(e) => {
                        d.error = Some(e);
                        return Task::none();
                    }
                };
                d.error = None;
                d.sending = true;
                let out = mail::Outgoing {
                    kind: d.kind,
                    original: d.original.clone(),
                    to,
                    cc,
                    subject: d.subject.trim().to_string(),
                    body: d.body.text(),
                    attachments: d.attachments.clone(),
                };
                let client = self.client.clone();
                return Task::perform(async move { mail::send(&client.0, &out).await.map_err(|e| e.to_string()) }, Message::ComposeSent);
            }
            _ => {}
        }
        Task::none()
    }

    fn load_mail(&mut self, force: bool) -> Task<Message> {
        if !force && !matches!(self.mail.list, Load::Idle | Load::Failed(_)) {
            return Task::none();
        }
        self.mail.list = Load::Loading;
        let (client, folder, offset, query) = (self.client.clone(), self.mail.folder, self.mail.offset, self.mail.active_query.clone());
        Task::perform(
            async move { mail::list(&client.0, folder, &query, offset, MAIL_PAGE_SIZE).await.map_err(|e| e.to_string()) },
            Message::MailLoaded,
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(screen) => return self.open(screen),
            Message::Refresh => {
                return match self.screen {
                    Screen::Home => Task::batch([
                        self.load_section(Section::Notifications, true),
                        self.load_section(Section::Schedule, true),
                        self.load_mail(true),
                    ]),
                    Screen::Section(Section::Grades) => self.load_grades(true),
                    Screen::Section(Section::Attendance) => self.load_attendance(true),
                    Screen::Section(Section::CourseEvaluation | Section::StaffEvaluation) => self.load_evaluations(true),
                    Screen::Section(Section::Transcript) if self.transcript_year.is_some() => {
                        let year = self.transcript_year.clone().unwrap();
                        self.transcripts.remove(&year);
                        self.select_year(year)
                    }
                    Screen::Section(s) => {
                        self.history.remove(&s);
                        self.load_section(s, true)
                    }
                    Screen::Cms => match self.cms.course.clone() {
                        Some((title, Load::Ready(p))) => self.update(Message::OpenCourse(title, p.url)),
                        _ => self.load_cms(true),
                    },
                    Screen::Mail => self.load_mail(true),
                    Screen::Gpa => self.load_section(Section::Transcript, true),
                    _ => Task::none(),
                };
            }
            Message::SectionLoaded(s, r) => {
                if let Ok(p) = &r {
                    match s {
                        Section::Schedule => self.schedule = Some(academic::schedule(p)),
                        Section::Exams => self.exams = Some(academic::exams(p)),
                        _ => {}
                    }
                }
                let first_transcript = s == Section::Transcript && r.is_ok() && self.transcript_year.is_none() && self.transcript_loading.is_none();
                if let (Section::Transcript, Ok(p)) = (s, &r) {
                    self.cumulative = academic::transcript(p).cumulative.or(self.cumulative.take());
                }
                self.pages.insert(s, r.into());
                // Open on the current study year.
                if first_transcript {
                    return self.select_year(academic::study_year(chrono::Local::now().date_naive()));
                }
            }
            Message::TranscriptYear(label) => return self.select_year(label),
            Message::TranscriptLoaded(label, r) => {
                self.transcript_loading = None;
                match r {
                    Ok(p) => {
                        let t = academic::transcript(&p);
                        if t.cumulative.is_some() {
                            self.cumulative = t.cumulative.clone();
                        }
                        let empty = t.seasons.is_empty();
                        self.transcripts.insert(label.clone(), t);
                        self.transcript_year = Some(label.clone());
                        // The running year has no grades until it ends: show the last one that does.
                        if empty && !self.transcript_stepped_back {
                            self.transcript_stepped_back = true;
                            let years = self.transcript_years();
                            if let Some(prev) = years.iter().position(|y| *y == label).and_then(|i| years.get(i + 1)).cloned() {
                                return self.select_year(prev);
                            }
                        }
                    }
                    Err(e) => self.notice = Some(Notice::Error(format!("Couldn't load the {label} transcript: {e}"))),
                }
            }
            Message::GradesLoaded(r) => self.grades = r.into(),
            Message::AttendanceLoaded(r) => self.attendance = r.into(),
            Message::EvaluationsLoaded(r) => self.evaluations = r.into(),
            Message::CmsKind(k) => self.cms.kind = k,
            Message::Follow(s, Target::Inline(data)) => {
                self.screen = Screen::Section(s);
                self.detail = Some(Detail::from_data(&data));
                return scroll_to_top();
            }
            Message::DetailBody(action) => {
                if let Some(d) = &mut self.detail {
                    if !action.is_edit() {
                        d.body.perform(action);
                    }
                }
            }
            Message::CloseDetail => self.detail = None,
            Message::Follow(s, target) => {
                if let Target::Href(url) = &target {
                    if url.host_str() != Some("apps.guc.edu.eg") {
                        let _ = opener::open_browser(url.as_str());
                        return Task::none();
                    }
                }
                let Some(current) = self.page(s).ready().cloned() else { return Task::none() };
                self.history.entry(s).or_default().push(current.clone());
                self.pages.insert(s, Load::Loading);
                self.screen = Screen::Section(s);
                let client = self.client.clone();
                return Task::perform(
                    async move { current.follow(&client.0, &target).await.map_err(|e| e.to_string()) },
                    move |r| Message::SectionLoaded(s, r),
                );
            }
            Message::Back(s) => {
                if let Some(prev) = self.history.get_mut(&s).and_then(Vec::pop) {
                    self.pages.insert(s, Load::Ready(prev));
                }
            }
            Message::DropdownPicked(s, name, choice) => {
                let Some(current) = self.page(s).ready().cloned() else { return Task::none() };
                self.pages.insert(s, Load::Loading);
                let client = self.client.clone();
                return Task::perform(
                    async move {
                        current.post_back(&client.0, &name, &[(name.clone(), choice.value)]).await.map_err(|e| e.to_string())
                    },
                    move |r| Message::SectionLoaded(s, r),
                );
            }
            Message::SavePageHtml(s) => {
                if let Some(p) = self.page(s).ready() {
                    self.notice = Some(match settings::save_page(&format!("{s:?}"), &p.html) {
                        Ok(path) => Notice::Info(format!("Saved the raw page to {}", path.display())),
                        Err(e) => Notice::Error(format!("Couldn't save the page: {e}")),
                    });
                }
            }
            Message::DismissNotice => self.notice = None,

            Message::CmsLoaded(r) => {
                if let Ok(p) = &r {
                    self.cms.courses = academic::cms_courses(p);
                }
                self.cms.home = r.into();
            }
            Message::OpenCourse(title, url) => {
                self.cms.course = Some((title, Load::Loading));
                self.cms.filter.clear();
                self.cms.kind = None;
                self.cms.weeks.clear();
                let client = self.client.clone();
                return Task::perform(
                    async move { page::fetch(&client.0, url.as_str()).await.map_err(|e| e.to_string()) },
                    Message::CourseLoaded,
                );
            }
            Message::CourseLoaded(r) => {
                if let Ok(p) = &r {
                    self.cms.weeks = academic::cms_weeks(p);
                }
                if let Some((_, load)) = &mut self.cms.course {
                    *load = r.into();
                }
            }
            Message::CloseCourse => self.cms.course = None,
            Message::CmsFilter(f) => self.cms.filter = f,
            Message::OpenLink(url, title) => {
                if !tools::is_file_link(&url) {
                    let _ = opener::open_browser(url.as_str());
                    return Task::none();
                }
                let folder = self.cms.course.as_ref().map(|(t, _)| t.clone()).unwrap_or_else(|| "Files".into());
                self.notice = Some(Notice::Info(format!("Downloading {}…", tools::saved_name(&title, &url))));
                let client = self.client.clone();
                return Task::perform(tools::download(client, url, title, folder, self.downloads.clone()), Message::Downloaded);
            }
            Message::Downloaded(r) => {
                self.notice = Some(match r {
                    Ok(path) => {
                        let _ = opener::open(&path);
                        Notice::Info(format!("Saved to {}", path.display()))
                    }
                    Err(e) => Notice::Error(format!("Download failed: {e}")),
                });
            }

            Message::MailFolder(f) => {
                self.mail.folder = f;
                self.mail.composing = false;
                self.mail.offset = 0;
                self.mail.open_id = None;
                self.mail.open = Load::Idle;
                self.mail.images = Load::Idle;
                return self.load_mail(true);
            }
            Message::MailQuery(q) => self.mail.query = q,
            Message::MailSearch => {
                self.mail.active_query = self.mail.query.trim().to_string();
                self.mail.offset = 0;
                return self.load_mail(true);
            }
            Message::MailClearSearch => {
                self.mail.query.clear();
                self.mail.active_query.clear();
                self.mail.offset = 0;
                return self.load_mail(true);
            }
            Message::MailPage(offset) => {
                self.mail.offset = offset;
                return self.load_mail(true);
            }
            Message::MailLoaded(r) => self.mail.list = r.into(),
            Message::MailOpen(id) => {
                self.mail.composing = false;
                self.mail.open_id = Some(id.clone());
                self.mail.open = Load::Loading;
                self.mail.images = Load::Idle;
                self.mail.images_failed = 0;
                let client = self.client.clone();
                return Task::perform(async move { mail::get(&client.0, &id).await.map_err(|e| e.to_string()) }, Message::MailMessage);
            }
            Message::MailMessage(r) => {
                // A slow reply for a message the user has since left.
                if r.as_ref().is_ok_and(|m| self.mail.open_id.as_deref() != Some(m.id.as_str())) {
                    return Task::none();
                }
                let mut task = Task::none();
                if let Ok(m) = &r {
                    self.mail.body = text_editor::Content::with_text(&m.body);
                    if m.attachments.iter().any(mail::Attachment::is_image) {
                        self.mail.images = Load::Loading;
                        let (client, id, attachments) = (self.client.clone(), m.id.clone(), m.attachments.clone());
                        task = Task::perform(async move { mail::images(&client.0, &attachments).await }, move |(shown, failed)| {
                            Message::MailImages(id.clone(), shown, failed)
                        });
                    }
                }
                self.mail.open = r.into();
                return task;
            }
            Message::SaveAttachment(attachment) => {
                self.notice = Some(Notice::Info(format!("Downloading {}…", tools::attachment_file_name(&attachment.name))));
                return Task::perform(tools::save_attachment(self.client.clone(), attachment, self.downloads.clone()), Message::AttachmentSaved);
            }
            Message::AttachmentSaved(r) => {
                self.notice = Some(match r {
                    Ok(path) if tools::safe_to_open(&path) => {
                        let _ = opener::open(&path);
                        Notice::Info(format!("Saved to {}", path.display()))
                    }
                    Ok(path) => {
                        if let Some(folder) = path.parent() {
                            let _ = opener::open(folder);
                        }
                        Notice::Info(format!("Saved to {}. This type of file can run programs, so it wasn't opened; only open it if you trust the sender.", path.display()))
                    }
                    Err(e) => Notice::Error(format!("Download failed: {e}")),
                });
            }
            Message::MailImages(id, shown, failed) => {
                if self.mail.open_id.as_deref() == Some(id.as_str()) {
                    self.mail.images_failed = failed;
                    self.mail.images = Load::Ready(
                        shown
                            .into_iter()
                            .map(|i| ShownImage {
                                handle: iced::widget::image::Handle::from_bytes(i.bytes),
                                width: i.width,
                                height: i.height,
                            })
                            .collect(),
                    );
                }
            }
            Message::MailBody(action) => {
                if !action.is_edit() {
                    self.mail.body.perform(action);
                }
            }
            Message::ComposeStart(kind) => {
                if self.mail.draft.as_ref().is_some_and(Draft::has_content) {
                    self.mail.composing = true;
                    self.notice = Some(Notice::Info("Send or discard this draft before starting another message.".into()));
                    return Task::none();
                }
                let original = self.mail.open.ready().filter(|_| kind != mail::Compose::New);
                if kind != mail::Compose::New && original.is_none() {
                    return Task::none();
                }
                self.mail.draft = Some(Draft::new(kind, original, self.client.0.username()));
                self.mail.composing = true;
                if matches!(kind, mail::Compose::New | mail::Compose::Forward) {
                    return iced::widget::text_input::focus(COMPOSE_TO);
                }
            }
            Message::ComposeResume => self.mail.composing = self.mail.draft.is_some(),
            Message::ComposeSent(r) => match r {
                Ok(()) => {
                    self.mail.draft = None;
                    self.mail.composing = false;
                    self.notice = Some(Notice::Info("Message sent.".into()));
                    if self.mail.folder == Folder::Sent {
                        return self.load_mail(true);
                    }
                }
                Err(e) => {
                    if let Some(d) = &mut self.mail.draft {
                        d.sending = false;
                        d.error = Some(format!("Couldn't send it: {e}"));
                    }
                }
            },
            Message::ComposeAttached(r) => {
                if let Some(d) = &mut self.mail.draft {
                    match r {
                        Ok(files) if d.attachments_size() + files.iter().map(|f| f.bytes.len()).sum::<usize>() > MAX_ATTACHMENTS_BYTES => {
                            d.error = Some("Attachments can add up to 10 MB at most, or GUC's mail server turns the message away.".into());
                        }
                        Ok(files) => {
                            d.error = None;
                            d.attachments.extend(files);
                        }
                        Err(e) => d.error = Some(e),
                    }
                }
            }
            Message::ComposeTo(_)
            | Message::ComposeCc(_)
            | Message::ComposeShowCc
            | Message::ComposeSubject(_)
            | Message::ComposeBody(_)
            | Message::ComposeAttach
            | Message::ComposeRemove(_)
            | Message::ComposeSend
            | Message::ComposeDiscard => return self.edit_draft(message),

            Message::Gpa(m) => self.gpa.update(m),
            Message::GpaFromTranscript => {
                if let Some(t) = self.transcript_year.as_ref().and_then(|y| self.transcripts.get(y)) {
                    let courses: Vec<portal::GradedCourse> = t
                        .seasons
                        .iter()
                        .flat_map(|s| &s.courses)
                        .filter_map(|c| Some(portal::GradedCourse { name: c.name.clone(), hours: c.hours?, grade: c.numeric? }))
                        .collect();
                    if courses.is_empty() {
                        self.notice = Some(Notice::Error("No graded courses were found on the transcript page for this year.".into()));
                    } else {
                        self.gpa = tools::GpaCalc::from_courses(&courses);
                        self.screen = Screen::Gpa;
                    }
                }
            }
            Message::Calc(m) => self.calc.update(m),
            Message::SaveDiagnostics => {
                self.saving_pages = true;
                self.notice = Some(Notice::Info("Saving pages… this takes about a minute.".into()));
                return Task::perform(tools::save_diagnostics(self.client.clone(), self.sections.clone()), Message::DiagnosticsSaved);
            }
            Message::DiagnosticsSaved(r) => {
                self.saving_pages = false;
                self.notice = Some(match r {
                    Ok((n, dir)) => Notice::Info(format!("Saved {n} pages to {}", dir.display())),
                    Err(e) => Notice::Error(format!("Couldn't save the pages: {e}")),
                });
            }
            _ => {}
        }
        Task::none()
    }
}

fn sign_in(login: String, password: String) -> Task<Message> {
    Task::perform(
        async move {
            let client = GucClient::new(Credentials::parse(&login, &password)).map_err(|e| e.to_string())?;
            let home = page::fetch(&client, portal::PORTAL_HOME).await.map_err(|e| e.to_string())?;
            Ok((Client(Arc::new(client)), home))
        },
        Message::SignedIn,
    )
}

#[cfg(test)]
mod draft_tests {
    use super::*;
    use guc::mail::{Compose, Mailbox};

    fn person(name: &str, address: &str) -> Mailbox {
        Mailbox { name: name.into(), address: address.into(), routing: "SMTP".into() }
    }

    fn message(from: Mailbox, to: Vec<Mailbox>, cc: Vec<Mailbox>) -> mail::Message {
        mail::Message {
            id: "ID".into(),
            change_key: "CK".into(),
            subject: "Lab moved".into(),
            from: from.name.clone(),
            sender: Some(from),
            to: String::new(),
            to_list: to,
            cc_list: cc,
            received: String::new(),
            body: String::new(),
            links: vec![],
            attachments: vec![],
        }
    }

    #[test]
    fn replies_fill_in_recipients_without_the_user() {
        let me = person("Me", "Ahmed.Ali@student.guc.edu.eg");
        let ta = person("Dr. TA", "ta@guc.edu.eg");
        let other = person("Friend", "friend@student.guc.edu.eg");
        let m = message(ta.clone(), vec![me.clone(), other.clone()], vec![person("Head", "head@guc.edu.eg"), me.clone()]);

        let reply = Draft::new(Compose::Reply, Some(&m), "ahmed.ali");
        assert_eq!((reply.to.as_str(), reply.cc.as_str(), reply.subject.as_str()), ("Dr. TA <ta@guc.edu.eg>", "", "RE: Lab moved"));
        assert_eq!(reply.original, Some(("ID".into(), "CK".into())));

        let all = Draft::new(Compose::ReplyAll, Some(&m), "ahmed.ali");
        assert_eq!(all.to, "Dr. TA <ta@guc.edu.eg>, Friend <friend@student.guc.edu.eg>");
        assert_eq!(all.cc, "Head <head@guc.edu.eg>");
        assert!(all.show_cc);

        // Replying to your own sent mail goes to its recipients.
        let sent = message(me.clone(), vec![ta.clone(), ta.clone()], vec![]);
        assert_eq!(Draft::new(Compose::ReplyAll, Some(&sent), "ahmed.ali").to, "Dr. TA <ta@guc.edu.eg>");

        let fw = Draft::new(Compose::Forward, Some(&m), "ahmed.ali");
        assert_eq!((fw.to.as_str(), fw.subject.as_str()), ("", "FW: Lab moved"));
        assert!(!fw.has_content(), "an untouched forward can be discarded at once");
    }

    #[test]
    fn subject_prefixes_dont_stack() {
        assert_eq!(prefixed("RE:", &[], "Re: Lab moved"), "Re: Lab moved");
        assert_eq!(prefixed("FW:", &["fwd:"], "Fwd: x"), "Fwd: x");
        assert_eq!(prefixed("RE:", &[], ""), "RE:");
        assert_eq!(prefixed("RE:", &[], "Report"), "RE: Report");
    }

    #[test]
    fn new_drafts_count_typed_fields_as_content() {
        let mut d = Draft::new(Compose::New, None, "ahmed.ali");
        assert!(!d.has_content() && d.original.is_none());
        d.subject = "Hi".into();
        assert!(d.has_content());
    }
}
