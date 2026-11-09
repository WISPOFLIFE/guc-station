//! Turns an ASP.NET WebForms page into plain data: its data tables, links,
//! dropdowns, labelled values and the form state needed to post back.

use crate::client::{Body, Error, GucClient, Result};
use reqwest::Method;
use scraper::{ElementRef, Html, Selector};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct Table {
    pub id: Option<String>,
    pub rows: Vec<Vec<String>>,
    /// The first link or button in each cell, parallel to `rows`.
    pub targets: Vec<Vec<Option<Target>>>,
}

/// Something clickable inside a page, resolved the way a browser would.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Href(Url),
    /// `javascript:__doPostBack('target','argument')` and its WebForm_ cousins.
    PostBack { target: String, argument: String },
    /// A submit button: posting the form with the button's own name=value.
    Submit { name: String, value: String },
    /// A popup whose content is already on the page, as `data-*` attributes
    /// (name without the prefix, value), e.g. a notification's body.
    Inline(Vec<(String, String)>),
}

impl Target {
    /// The value of an `Inline` target's `data-<key>` attribute.
    pub fn data(&self, key: &str) -> Option<&str> {
        match self {
            Target::Inline(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Link {
    pub text: String,
    pub href: Url,
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl PartialEq for Choice {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

#[derive(Debug, Clone)]
pub struct Dropdown {
    pub name: String,
    pub choices: Vec<Choice>,
    pub selected: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub url: Url,
    pub title: String,
    pub tables: Vec<Table>,
    pub links: Vec<Link>,
    pub dropdowns: Vec<Dropdown>,
    /// `<span id=...>` values, which is where WebForms puts single fields.
    pub labels: Vec<(String, String)>,
    pub html: String,
    fields: Vec<(String, String)>,
    action: Url,
}

impl Page {
    pub fn parse(url: Url, html: String) -> Page {
        let doc = Html::parse_document(&html);
        let title = select_first(&doc, "title").map(|e| clean(&e)).unwrap_or_default();

        let tables = doc
            .select(&sel("table"))
            .filter(|t| t.select(&sel("table")).nth(0).is_none()) // leaf tables only; outer ones are layout
            .map(|t| {
                let (rows, targets) = table_rows(t, &url);
                Table { id: t.value().id().map(str::to_string), rows, targets }
            })
            .filter(|t| t.rows.len() >= 1 && t.rows.iter().any(|r| r.iter().any(|c| !c.is_empty())))
            .collect();

        let links = doc
            .select(&sel("a[href]"))
            .filter_map(|a| {
                let href = a.value().attr("href")?;
                if href.starts_with("javascript:") || href.starts_with('#') {
                    return None;
                }
                Some(Link { text: clean(&a), href: url.join(href).ok()? })
            })
            .collect();

        let dropdowns = doc
            .select(&sel("select[name]"))
            .map(|s| {
                let choices: Vec<Choice> = s
                    .select(&sel("option"))
                    .map(|o| Choice {
                        value: o.value().attr("value").map(str::to_string).unwrap_or_else(|| clean(&o)),
                        label: clean(&o),
                    })
                    .collect();
                let selected = s
                    .select(&sel("option[selected]"))
                    .next()
                    .and_then(|o| o.value().attr("value").map(str::to_string));
                Dropdown { name: s.value().attr("name").unwrap_or_default().to_string(), choices, selected }
            })
            .collect();

        let labels = doc
            .select(&sel("span[id]"))
            .filter_map(|s| {
                let text = clean(&s);
                (!text.is_empty()).then(|| (s.value().id().unwrap_or_default().to_string(), text))
            })
            .collect();

        let form = select_first(&doc, "form");
        let action = form
            .and_then(|f| f.value().attr("action"))
            .and_then(|a| url.join(a).ok())
            .unwrap_or_else(|| url.clone());
        let fields = form.map(form_fields).unwrap_or_default();

        Page { url, title, tables, links, dropdowns, labels, html, fields, action }
    }

    /// The first label whose id contains `needle` (case-insensitive).
    pub fn label(&self, needle: &str) -> Option<&str> {
        let needle = needle.to_lowercase();
        self.labels.iter().find(|(id, _)| id.to_lowercase().contains(&needle)).map(|(_, v)| v.as_str())
    }

    /// Follows a link or button the way a browser would, returning the next page.
    pub async fn follow(&self, client: &GucClient, target: &Target) -> Result<Page> {
        match target {
            Target::Href(url) => fetch(client, url.as_str()).await,
            Target::PostBack { target, argument } => {
                self.post_back(client, target, &[("__EVENTARGUMENT".into(), argument.clone())]).await
            }
            Target::Submit { name, value } => self.post_back(client, "", &[(name.clone(), value.clone())]).await,
            Target::Inline(_) => Ok(self.clone()),
        }
    }

    /// Simulates a WebForms postback: the page's current form state plus
    /// `changes`, with `__EVENTTARGET` set to whatever triggered it.
    pub async fn post_back(&self, client: &GucClient, event_target: &str, changes: &[(String, String)]) -> Result<Page> {
        let mut fields = self.fields.clone();
        for (name, value) in changes.iter().cloned().chain([("__EVENTTARGET".to_string(), event_target.to_string())]) {
            match fields.iter_mut().find(|(n, _)| *n == name) {
                Some(f) => f.1 = value,
                None => fields.push((name, value)),
            }
        }
        let mut waits = 0;
        loop {
            let resp = client.send(Method::POST, self.action.clone(), Body::Form(fields.clone()), &[]).await?;
            let html = resp.text();
            if throttled(&html, &mut waits).await?.is_some() {
                continue;
            }
            return match script_redirect(&resp.url, &html) {
                Some(next) => fetch(client, next.as_str()).await,
                None => Ok(Page::parse(resp.url.clone(), html)),
            };
        }
    }
}

pub async fn fetch(client: &GucClient, url: &str) -> Result<Page> {
    let mut url = url.to_string();
    let mut waits = 0;
    // A couple of script redirects at most; more means something is looping.
    let mut redirects = 0;
    while redirects < 3 {
        let resp = client.get(&url).await?;
        if !resp.content_type.contains("html") && !resp.content_type.is_empty() {
            return Err(Error::Other(format!("Expected a page but got {}", resp.content_type)));
        }
        let html = resp.text();
        if throttled(&html, &mut waits).await?.is_some() {
            continue;
        }
        redirects += 1;
        match script_redirect(&resp.url, &html) {
            Some(next) => url = next.to_string(),
            None => return Ok(Page::parse(resp.url.clone(), html)),
        }
    }
    Err(Error::Other("The portal kept redirecting this page.".into()))
}

/// When the portal says "temporarily paused ... refresh again after
/// 00:00:59", sleeps that long and returns Some so the caller retries. Gives
/// up after two waits.
async fn throttled(html: &str, waits: &mut u32) -> Result<Option<u64>> {
    let Some(secs) = crate::academic::pause_seconds(html) else { return Ok(None) };
    if *waits >= 2 {
        return Err(Error::Other("The portal is busy and asked us to wait. Try again in a minute.".into()));
    }
    *waits += 1;
    tokio::time::sleep(std::time::Duration::from_secs(secs + 1)).await;
    Ok(Some(secs))
}

/// Some portal pages answer with only `<script>sTo('CODE')</script>`, whose
/// packed helper reloads the same page as `?v=CODE`. A browser runs that
/// script; we have to do it by hand.
pub fn script_redirect(url: &Url, html: &str) -> Option<Url> {
    if html.len() > 4096 {
        return None;
    }
    let start = html.find("sTo('")? + 5;
    let code = &html[start..start + html[start..].find('\'')?];
    let mut next = url.clone();
    next.set_query(Some(&format!("v={code}")));
    Some(next)
}

fn table_rows(table: ElementRef, base: &Url) -> (Vec<Vec<String>>, Vec<Vec<Option<Target>>>) {
    table
        .select(&sel("tr"))
        .map(|tr| {
            tr.children()
                .filter_map(ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .map(|c| (clean(&c), cell_target(c, base)))
                .unzip::<_, _, Vec<_>, Vec<_>>()
        })
        .filter(|(r, _)| !r.is_empty())
        .unzip()
}

fn cell_target(cell: ElementRef, base: &Url) -> Option<Target> {
    // Popup buttons carry their whole content in data-* attributes.
    if let Some(b) = cell.select(&sel("[data-toggle=modal]")).next() {
        let data: Vec<(String, String)> = b
            .value()
            .attrs()
            .filter_map(|(k, v)| Some((k.strip_prefix("data-")?.to_string(), v.to_string())))
            .filter(|(k, _)| k != "toggle" && k != "target")
            .collect();
        if !data.is_empty() {
            return Some(Target::Inline(data));
        }
    }
    if let Some(a) = cell.select(&sel("a[href]")).next() {
        let href = a.value().attr("href").unwrap_or_default();
        return match href.strip_prefix("javascript:") {
            Some(script) => parse_postback(script),
            None if href.starts_with('#') => None,
            None => base.join(href).ok().map(Target::Href),
        };
    }
    let button = cell.select(&sel("input[type=submit][name], input[type=button][name], button[name]")).next()?;
    let v = button.value();
    if let Some(script) = v.attr("onclick").filter(|s| s.contains("PostBack")) {
        if let Some(t) = parse_postback(script) {
            return Some(t);
        }
    }
    Some(Target::Submit {
        name: v.attr("name").unwrap_or_default().to_string(),
        value: v.attr("value").map(str::to_string).unwrap_or_else(|| clean(&button)),
    })
}

/// Pulls the target and argument out of `__doPostBack('a','b')` or
/// `WebForm_DoPostBackWithOptions(new WebForm_PostBackOptions("a", "b", ...))`.
fn parse_postback(script: &str) -> Option<Target> {
    let args = &script[script.find("PostBack")?..];
    let first_quote = args.find(['\'', '"'])?;
    let mut quoted = args[first_quote..].split(['\'', '"']).skip(1).step_by(2);
    let target = quoted.next()?.to_string();
    let argument = quoted.next().unwrap_or_default().to_string();
    (!target.is_empty()).then_some(Target::PostBack { target, argument })
}

fn form_fields(form: ElementRef) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for input in form.select(&sel("input[name]")) {
        let v = input.value();
        let kind = v.attr("type").unwrap_or("text").to_ascii_lowercase();
        let name = v.attr("name").unwrap_or_default().to_string();
        let value = v.attr("value").unwrap_or_default().to_string();
        match kind.as_str() {
            // Buttons only count when clicked, which callers pass as a change.
            "submit" | "button" | "image" | "reset" | "file" => {}
            "checkbox" | "radio" => {
                if v.attr("checked").is_some() {
                    out.push((name, if value.is_empty() { "on".into() } else { value }));
                }
            }
            _ => out.push((name, value)),
        }
    }
    for select in form.select(&sel("select[name]")) {
        let name = select.value().attr("name").unwrap_or_default().to_string();
        let chosen = select.select(&sel("option[selected]")).next().or_else(|| select.select(&sel("option")).next());
        if let Some(o) = chosen {
            out.push((name, o.value().attr("value").map(str::to_string).unwrap_or_else(|| clean(&o))));
        }
    }
    for area in form.select(&sel("textarea[name]")) {
        out.push((area.value().attr("name").unwrap_or_default().to_string(), area.text().collect()));
    }
    out
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn select_first<'a>(doc: &'a Html, s: &str) -> Option<ElementRef<'a>> {
    doc.select(&sel(s)).next()
}

/// Element text with whitespace (including &nbsp;) collapsed.
pub fn clean(e: &ElementRef) -> String {
    e.text().flat_map(|t| t.split(|c: char| c.is_whitespace() || c == '\u{a0}')).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<html><head><title> Grades </title></head><body>
      <form action="./CheckGrade.aspx" method="post">
        <input type="hidden" name="__VIEWSTATE" value="abc" />
        <input type="submit" name="btn" value="Go" />
        <select name="ctl00$course"><option value="0">Choose</option><option value="12" selected="selected">CSEN 401</option></select>
        <table><tr><td><table id="midDg"><tr><th>Course</th><th>Percentage</th></tr><tr><td>CSEN&nbsp;401</td><td> 82.5 </td></tr></table></td></tr></table>
        <span id="ctl00_cmGpaLbl">1.42</span>
        <a href="CourseViewStn.aspx?id=5">Course</a><a href="javascript:void(0)">x</a>
      </form></body></html>"#;

    #[test]
    fn extracts_page_parts() {
        let p = Page::parse(Url::parse("https://apps.guc.edu.eg/student_ext/Grade/x.aspx").unwrap(), SAMPLE.into());
        assert_eq!(p.title, "Grades");
        assert_eq!(p.tables.len(), 1, "layout table is skipped");
        assert_eq!(p.tables[0].id.as_deref(), Some("midDg"));
        assert_eq!(p.tables[0].rows[1], vec!["CSEN 401", "82.5"]);
        assert_eq!(p.dropdowns[0].selected.as_deref(), Some("12"));
        assert_eq!(p.label("gpa"), Some("1.42"));
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].href.as_str(), "https://apps.guc.edu.eg/student_ext/Grade/CourseViewStn.aspx?id=5");
        assert_eq!(p.action.as_str(), "https://apps.guc.edu.eg/student_ext/Grade/CheckGrade.aspx");
        assert!(p.fields.iter().any(|(n, v)| n == "__VIEWSTATE" && v == "abc"));
        assert!(!p.fields.iter().any(|(n, _)| n == "btn"));
    }

    #[test]
    fn finds_cell_targets() {
        let html = r##"<table><tr><th>ID</th><th>Actions</th></tr>
            <tr><td>1</td><td><a href="javascript:__doPostBack('ctl00$grid','Select$0')">View</a></td></tr>
            <tr><td>2</td><td><a href="Details.aspx?id=2">View</a></td></tr>
            <tr><td>3</td><td><input type="submit" name="ctl00$grid$ctl04$btn" value="View" /></td></tr>
            <tr><td>4</td><td><a href="javascript:WebForm_DoPostBackWithOptions(new WebForm_PostBackOptions(&quot;ctl00$x&quot;, &quot;&quot;, true))">View</a></td></tr>
            <tr><td>5</td><td><button type="button" class="btn" data-toggle="modal" data-target="#myModal" data-subject_text="Quiz 1" data-body_text="Dear all,&#10;Room C7.">View</button></td></tr></table>"##;
        let p = Page::parse(Url::parse("https://apps.guc.edu.eg/student_ext/Main/N.aspx").unwrap(), html.into());
        let t = &p.tables[0].targets;
        assert_eq!(t[0][1], None);
        assert_eq!(t[1][1], Some(Target::PostBack { target: "ctl00$grid".into(), argument: "Select$0".into() }));
        assert_eq!(t[2][1], Some(Target::Href(Url::parse("https://apps.guc.edu.eg/student_ext/Main/Details.aspx?id=2").unwrap())));
        assert_eq!(t[3][1], Some(Target::Submit { name: "ctl00$grid$ctl04$btn".into(), value: "View".into() }));
        assert_eq!(t[4][1], Some(Target::PostBack { target: "ctl00$x".into(), argument: "".into() }));
        let popup = t[5][1].as_ref().unwrap();
        assert_eq!(popup.data("subject_text"), Some("Quiz 1"));
        assert_eq!(popup.data("body_text"), Some("Dear all,\nRoom C7."));
        assert_eq!(popup.data("toggle"), None);
    }
}
