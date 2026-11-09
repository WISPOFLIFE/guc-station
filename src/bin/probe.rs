//! Signs in and saves the raw HTML of every portal section to `fixtures/`,
//! so page parsers can be written against real markup.
//!
//! Reads credentials from the GUC_USER and GUC_PASS environment variables,
//! or else uses the sign-in the app saved in the OS credential store.
//! The saved pages contain personal data; `fixtures/` is git-ignored.

use guc::client::{Credentials, GucClient};
use guc::page;
use guc::portal;
use std::io::Write;
use std::path::Path;

/// Prints a line and appends it to fixtures/probe.log, so a run leaves a
/// record even when the console window is gone.
macro_rules! log {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{line}");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("fixtures/probe.log") {
            let _ = writeln!(f, "{line}");
        }
    }};
}

#[tokio::main]
async fn main() {
    let saved = guc::creds::load();
    let (user, pass) = match (std::env::var("GUC_USER"), std::env::var("GUC_PASS"), saved) {
        (Ok(u), Ok(p), _) => (u, p),
        (_, _, Some(saved)) => saved,
        _ => {
            eprintln!("Set GUC_USER and GUC_PASS, or sign in once in the app with \"Keep me signed in\".");
            std::process::exit(2);
        }
    };
    let out = Path::new("fixtures");
    std::fs::create_dir_all(out).expect("create fixtures/");
    let _ = std::fs::remove_file(out.join("probe.log"));
    let creds = Credentials::parse(&user, &pass);
    log!("Signing in as user={:?} domain={:?} (password: {} chars)", creds.username, creds.domain, creds.password.chars().count());
    let client = GucClient::new(creds).expect("client");

    let home = match page::fetch(&client, portal::PORTAL_HOME).await {
        Ok(p) => p,
        Err(e) => {
            log!("Sign-in failed: {e}");
            log!("Server NTLM domain: {:?}", client.ntlm_domain());
            std::process::exit(1);
        }
    };
    log!("Server NTLM domain: {:?}", client.ntlm_domain());
    log!("Signed in. Home page: {} ({} links)", home.title, home.links.len());
    save(out, "portal_home", &home.html);

    // `probe mailaddr`: reads a few inbox messages and prints only the kind
    // of address each sender and recipient has (routing type and domain).
    if std::env::args().nth(1).as_deref() == Some("mailaddr") {
        let user = client.username().to_lowercase(); let shape = |m: &guc::mail::Mailbox| format!("{}{}@{}", if m.address.split('@').next().unwrap_or("").to_lowercase() == user { "ME:" } else { "" }, if m.routing.is_empty() { "?" } else { &m.routing }, m.address.rsplit('@').next().filter(|_| m.address.contains('@')).unwrap_or("(no @)"));
        let (items, _) = guc::mail::list(&client, guc::mail::Folder::Inbox, "", 0, 8).await.expect("list");
        for (i, it) in items.iter().enumerate() {
            match guc::mail::get(&client, &it.id).await {
                Ok(m) => log!("#{i} changekey={} from={:?} to={:?} cc={:?}", !m.change_key.is_empty(), m.sender.as_ref().map(shape), m.to_list.iter().map(shape).collect::<Vec<_>>(), m.cc_list.iter().map(shape).collect::<Vec<_>>()),
                Err(e) => log!("#{i} FAILED {e}"),
            }
        }
        return;
    }

    // `probe drafts`: builds a new message and a reply, each with a small
    // attachment, as drafts addressed to the user, then deletes them. Never sends.
    if std::env::args().nth(1).as_deref() == Some("drafts") {
        use guc::mail::{Compose, Mailbox, OutFile, Outgoing};
        let me = Mailbox { address: format!("{}@student.guc.edu.eg", client.username()), ..Default::default() };
        let (items, _) = guc::mail::list(&client, guc::mail::Folder::Inbox, "", 0, 1).await.expect("list");
        let first = guc::mail::get(&client, &items[0].id).await.expect("get");
        let file = OutFile { name: "draft-check.txt".into(), bytes: b"GUC Station draft check".to_vec() };
        for kind in [Compose::New, Compose::Reply, Compose::Forward] {
            let out = Outgoing {
                kind,
                original: (kind != Compose::New).then(|| (first.id.clone(), first.change_key.clone())),
                to: vec![me.clone()],
                cc: vec![],
                subject: "GUC Station draft check (not sent)".into(),
                body: "Draft check.

Second paragraph.".into(),
                attachments: vec![file.clone()],
            };
            match guc::mail::prepare_draft(&client, &out).await {
                Ok((id, key)) => {
                    let changed = key != first.change_key;
                    let saved = guc::mail::get(&client, &id).await;
                    let summary = saved.as_ref().map(|m| (m.subject.clone(), m.to_list.len(), m.attachments.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), m.body.contains("Second paragraph"))).map_err(|e| e.to_string());
                    let deleted = guc::mail::discard_draft(&client, &id).await;
                    log!("{kind:?}: draft ok key_changed={changed} saved={summary:?} deleted={:?}", deleted.map_err(|e| e.to_string()));
                }
                Err(e) => log!("{kind:?}: FAILED {e}"),
            }
        }
        return;
    }

    // `probe evaluations`: open the first course and staff evaluation forms
    // and save them. Only picks from the dropdowns; never submits.
    if std::env::args().nth(1).as_deref() == Some("evaluations") {
        let sections = portal::discover(Some(&home));
        for (section, name) in [(portal::Section::CourseEvaluation, "eval_course"), (portal::Section::StaffEvaluation, "eval_staff")] {
            let Ok(p) = page::fetch(&client, sections[&section].as_str()).await else { continue };
            save(out, name, &p.html);
            let Some(d) = p.dropdowns.iter().find(|d| d.choices.len() > 1) else { continue };
            let mut saved_form = false;
            for (i, c) in d.choices.iter().filter(|c| !c.value.is_empty()).enumerate() {
                match p.post_back(&client, &d.name, &[(d.name.clone(), c.value.clone())]).await {
                    Ok(form) => {
                        let done = form.html.contains("already evaluated");
                        log!("{name} #{i}: {}", if done { "already evaluated" } else { "PENDING" });
                        if !done && !saved_form {
                            save(out, &format!("{name}_form"), &form.html);
                            saved_form = true;
                        }
                    }
                    Err(e) => log!("{name} #{i}: FAILED {e}"),
                }
            }
        }
        return;
    }

    for (section, url) in portal::discover(Some(&home)) {
        let name = format!("{section:?}").to_lowercase();
        match page::fetch(&client, url.as_str()).await {
            Ok(p) => {
                log!("{name:<18} {url}  tables={} dropdowns={}", p.tables.len(), p.dropdowns.len());
                save(out, &name, &p.html);
                // The transcript is empty until a study year is picked.
                if section == portal::Section::Transcript {
                    if let Some(d) = p.dropdowns.first() {
                        // Skip the newest (future) year; the portal throttles each request.
                        for c in d.choices.iter().filter(|c| !c.value.is_empty()).skip(1).take(3) {
                            let mut result = p.post_back(&client, &d.name, &[(d.name.clone(), c.value.clone())]).await;
                            while let Some(wait) = result.as_ref().ok().and_then(|y| guc::academic::pause_seconds(&y.html)) {
                                log!("  year {} paused, waiting {wait}s", c.label);
                                tokio::time::sleep(std::time::Duration::from_secs(wait + 2)).await;
                                result = p.post_back(&client, &d.name, &[(d.name.clone(), c.value.clone())]).await;
                            }
                            match result {
                                Ok(y) => {
                                    log!("  year {:<12} tables={}", c.label, y.tables.len());
                                    save(out, &format!("transcript_{}", c.label), &y.html);
                                }
                                Err(e) => log!("  year {} FAILED: {e}", c.label),
                            }
                        }
                    }
                }
            }
            Err(e) => log!("{name:<18} {url}  FAILED: {e}"),
        }
    }

    match page::fetch(&client, portal::CMS_HOME).await {
        Ok(p) => {
            log!("cms_home           tables={} links={}", p.tables.len(), p.links.len());
            save(out, "cms_home", &p.html);
            if let Some(course) = p.links.iter().find(|l| l.href.path().to_lowercase().contains("courseview")) {
                if let Ok(c) = page::fetch(&client, course.href.as_str()).await {
                    save(out, "cms_course", &c.html);
                }
            }
        }
        Err(e) => log!("cms_home           FAILED: {e}"),
    }

    match guc::mail::list(&client, guc::mail::Folder::Inbox, "", 0, 5).await {
        Ok((items, total)) => log!("mail               {total} messages, newest: {:?}", items.first().map(|m| &m.subject)),
        Err(e) => log!("mail               FAILED: {e}"),
    }
    log!("Done. Pages are in {}", std::fs::canonicalize(out).unwrap_or(out.to_path_buf()).display());
}

fn save(dir: &Path, name: &str, html: &str) {
    let _ = std::fs::write(dir.join(format!("{name}.html")), html);
}
