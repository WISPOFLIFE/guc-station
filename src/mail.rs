//! University mail over Exchange Web Services. OWA itself uses a forms login,
//! but the EWS endpoint accepts the same NTLM credentials as the portal.

use crate::client::{Body, Error, GucClient, Result};
use crate::portal::EWS;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use reqwest::Method;
use url::Url;

#[derive(Debug, Clone)]
pub struct Summary {
    pub id: String,
    pub subject: String,
    pub from: String,
    pub received: String,
    pub is_read: bool,
    pub has_attachments: bool,
}

/// Someone a message is from or to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mailbox {
    pub name: String,
    pub address: String,
    /// "SMTP" for ordinary addresses. Exchange can also hand back its own
    /// internal ("EX") form, which only works as a recipient as it came.
    pub routing: String,
}

impl Mailbox {
    pub fn is_smtp(&self) -> bool {
        self.address.contains('@') && (self.routing.is_empty() || self.routing.eq_ignore_ascii_case("SMTP"))
    }

    pub fn label(&self) -> &str {
        if self.name.is_empty() { &self.address } else { &self.name }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    /// Exchange's version stamp for the item, needed to reply to or forward it.
    pub change_key: String,
    pub subject: String,
    pub from: String,
    pub sender: Option<Mailbox>,
    pub to: String,
    pub to_list: Vec<Mailbox>,
    pub cc_list: Vec<Mailbox>,
    pub received: String,
    pub body: String,
    /// (text, url) for every web or mailto link in the body, without repeats.
    pub links: Vec<(String, String)>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// Embedded in the message body, like a signature logo, rather than attached by hand.
    pub is_inline: bool,
}

impl Attachment {
    /// Files the reader lists for download. Inline images are already part of the mail.
    pub fn is_listed(&self) -> bool {
        !(self.is_inline && self.is_image())
    }

    /// Exchange sometimes labels images application/octet-stream, so the name counts too.
    pub fn is_image(&self) -> bool {
        let name = self.name.to_lowercase();
        self.content_type.to_lowercase().starts_with("image/")
            || [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"].iter().any(|ext| name.ends_with(ext))
    }
}

/// A decodable image from a message, ready to display.
#[derive(Debug, Clone)]
pub struct MailImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Bigger attachments are skipped rather than held in memory for display.
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
/// Anything this small in both directions is a spacer or tracker, not content.
const TINY_IMAGE_PX: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    Inbox,
    Sent,
}

impl Folder {
    fn id(self) -> &'static str {
        match self {
            Folder::Inbox => "inbox",
            Folder::Sent => "sentitems",
        }
    }
}

/// Lists a folder, newest first. A non-empty `query` searches the whole folder
/// on the server (sender, subject and body), and paging applies to the matches.
pub async fn list(client: &GucClient, folder: Folder, query: &str, offset: u32, count: u32) -> Result<(Vec<Summary>, u32)> {
    let search = query_element(query);
    let body = format!(
        r#"<m:FindItem Traversal="Shallow">
      <m:ItemShape>
        <t:BaseShape>IdOnly</t:BaseShape>
        <t:AdditionalProperties>
          <t:FieldURI FieldURI="item:Subject"/>
          <t:FieldURI FieldURI="item:DateTimeReceived"/>
          <t:FieldURI FieldURI="item:HasAttachments"/>
          <t:FieldURI FieldURI="message:From"/>
          <t:FieldURI FieldURI="message:IsRead"/>
        </t:AdditionalProperties>
      </m:ItemShape>
      <m:IndexedPageItemView MaxEntriesReturned="{count}" Offset="{offset}" BasePoint="Beginning"/>
      <m:SortOrder><t:FieldOrder Order="Descending"><t:FieldURI FieldURI="item:DateTimeReceived"/></t:FieldOrder></m:SortOrder>
      <m:ParentFolderIds><t:DistinguishedFolderId Id="{}"/></m:ParentFolderIds>{search}
    </m:FindItem>"#,
        folder.id()
    );
    let xml = call(client, "FindItem", &body).await?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)?;

    let total = doc
        .descendants()
        .find(|n| n.has_tag_name_local("RootFolder"))
        .and_then(|n| n.attribute("TotalItemsInView"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let items = doc
        .descendants()
        .filter(|n| n.has_tag_name_local("Message"))
        .map(|m| Summary {
            id: m.descendants().find(|n| n.has_tag_name_local("ItemId")).and_then(|n| n.attribute("Id")).unwrap_or_default().to_string(),
            subject: child_text(m, "Subject"),
            from: sender(m),
            received: child_text(m, "DateTimeReceived"),
            is_read: child_text(m, "IsRead") == "true",
            has_attachments: child_text(m, "HasAttachments") == "true",
        })
        .collect();
    Ok((items, total))
}

pub async fn get(client: &GucClient, id: &str) -> Result<Message> {
    let body = format!(
        r#"<m:GetItem>
      <m:ItemShape><t:BaseShape>Default</t:BaseShape><t:BodyType>HTML</t:BodyType><t:AdditionalProperties><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURI FieldURI="message:From"/><t:FieldURI FieldURI="message:ToRecipients"/><t:FieldURI FieldURI="message:CcRecipients"/></t:AdditionalProperties></m:ItemShape>
      <m:ItemIds><t:ItemId Id="{}"/></m:ItemIds>
    </m:GetItem>"#,
        escape(id)
    );
    let xml = call(client, "GetItem", &body).await?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)?;
    let m = doc
        .descendants()
        .find(|n| n.has_tag_name_local("Message"))
        .ok_or_else(|| Error::Other("Exchange returned no message.".into()))?;
    let to_list = mailboxes(m, "ToRecipients");
    let to = to_list.iter().map(Mailbox::label).collect::<Vec<_>>().join(", ");
    let change_key = m
        .children()
        .find(|n| n.has_tag_name_local("ItemId"))
        .and_then(|n| n.attribute("ChangeKey"))
        .unwrap_or_default()
        .to_string();
    let body_html = child_text(m, "Body");
    Ok(Message {
        id: id.to_string(),
        change_key,
        attachments: attachments_of(m),
        subject: child_text(m, "Subject"),
        from: sender(m),
        sender: m
            .children()
            .find(|n| n.has_tag_name_local("From"))
            .and_then(|f| f.children().find(|n| n.has_tag_name_local("Mailbox")))
            .map(mailbox),
        to,
        to_list,
        cc_list: mailboxes(m, "CcRecipients"),
        received: child_text(m, "DateTimeReceived"),
        body: html_to_text(&body_html),
        links: links_in(&body_html),
    })
}

fn mailbox(n: roxmltree::Node) -> Mailbox {
    Mailbox { name: child_text(n, "Name"), address: child_text(n, "EmailAddress"), routing: child_text(n, "RoutingType") }
}

/// Only the message's own recipient lists, not those of a mail attached to it.
fn mailboxes(m: roxmltree::Node, list: &str) -> Vec<Mailbox> {
    m.children()
        .filter(|n| n.has_tag_name_local(list))
        .flat_map(|r| r.children().filter(|n| n.has_tag_name_local("Mailbox")))
        .map(mailbox)
        .collect()
}

/// GUC addresses are the username at a GUC domain, which is how the user's
/// own address is recognised when building a reply-all.
pub fn is_me(m: &Mailbox, username: &str) -> bool {
    let (local, domain) = m.address.split_once('@').unwrap_or_default();
    let domain = domain.to_lowercase();
    local.eq_ignore_ascii_case(username) && (domain == "guc.edu.eg" || domain.ends_with(".guc.edu.eg"))
}

// ---------------------------------------------------------------- sending

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compose {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// A file to attach, already read into memory.
#[derive(Debug, Clone)]
pub struct OutFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Outgoing {
    pub kind: Compose,
    /// The message being answered or forwarded: its id and change key.
    pub original: Option<(String, String)>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub subject: String,
    /// Plain text as typed. Exchange adds the quoted original for replies and
    /// forwards, and a forward keeps the original's attachments.
    pub body: String,
    pub attachments: Vec<OutFile>,
}

/// Sends a message, keeping a copy in Sent Items. If sending fails, the
/// draft made for it is deleted again so a retry doesn't leave duplicates.
pub async fn send(client: &GucClient, out: &Outgoing) -> Result<()> {
    let (id, change_key) = prepare_draft(client, out).await?;
    let result = async {
        let xml = call(client, "SendItem", &send_request(&id, &change_key)).await?;
        let doc = roxmltree::Document::parse(&xml).map_err(|e| Error::Other(e.to_string()))?;
        check_response(&doc)
    }
    .await;
    if result.is_err() {
        let _ = discard_draft(client, &id).await;
    }
    result
}

/// Saves the message in Drafts with its attachments, ready to send, and
/// returns its id and change key. Exchange can only add attachments to a
/// saved item, which is why sending goes through a draft.
pub async fn prepare_draft(client: &GucClient, out: &Outgoing) -> Result<(String, String)> {
    let xml = call(client, "CreateItem", &create_request(out)?).await?;
    let (id, change_key) = created_item(&xml)?;
    if out.attachments.is_empty() {
        return Ok((id, change_key));
    }
    let attached = async {
        let xml = call(client, "CreateAttachment", &attach_request(&id, &change_key, &out.attachments)).await?;
        attached_change_key(&xml)
    }
    .await;
    match attached {
        Ok(new_key) => Ok((id, new_key.unwrap_or(change_key))),
        Err(e) => {
            let _ = discard_draft(client, &id).await;
            Err(e)
        }
    }
}

/// Deletes a draft this app made itself; nothing else is ever deleted.
pub async fn discard_draft(client: &GucClient, id: &str) -> Result<()> {
    let body = format!(
        r#"<m:DeleteItem DeleteType="HardDelete"><m:ItemIds><t:ItemId Id="{}"/></m:ItemIds></m:DeleteItem>"#,
        escape(id)
    );
    let xml = call(client, "DeleteItem", &body).await?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)
}

fn create_request(out: &Outgoing) -> Result<String> {
    let recipients = |tag: &str, list: &[Mailbox]| -> String {
        if list.is_empty() {
            return String::new();
        }
        let boxes: String = list.iter().map(mailbox_xml).collect();
        format!("<t:{tag}>{boxes}</t:{tag}>")
    };
    let subject = format!("<t:Subject>{}</t:Subject>", escape(&out.subject));
    let (to, cc) = (recipients("ToRecipients", &out.to), recipients("CcRecipients", &out.cc));
    let body = escape(&text_to_html(&out.body));
    let item = match (out.kind, &out.original) {
        (Compose::New, _) => format!(r#"<t:Message>{subject}<t:Body BodyType="HTML">{body}</t:Body>{to}{cc}</t:Message>"#),
        (kind, Some((id, change_key))) => {
            let tag = match kind {
                Compose::Reply => "ReplyToItem",
                Compose::ReplyAll => "ReplyAllToItem",
                _ => "ForwardItem",
            };
            format!(
                r#"<t:{tag}>{subject}{to}{cc}<t:ReferenceItemId Id="{}" ChangeKey="{}"/><t:NewBodyContent BodyType="HTML">{body}</t:NewBodyContent></t:{tag}>"#,
                escape(id),
                escape(change_key)
            )
        }
        (_, None) => return Err(Error::Other("The message being answered is no longer open.".into())),
    };
    Ok(format!(
        r#"<m:CreateItem MessageDisposition="SaveOnly"><m:SavedItemFolderId><t:DistinguishedFolderId Id="drafts"/></m:SavedItemFolderId><m:Items>{item}</m:Items></m:CreateItem>"#
    ))
}

fn mailbox_xml(m: &Mailbox) -> String {
    let name = if m.name.is_empty() { String::new() } else { format!("<t:Name>{}</t:Name>", escape(&m.name)) };
    let routing = if m.routing.is_empty() { String::new() } else { format!("<t:RoutingType>{}</t:RoutingType>", escape(&m.routing)) };
    format!("<t:Mailbox>{name}<t:EmailAddress>{}</t:EmailAddress>{routing}</t:Mailbox>", escape(&m.address))
}

fn attach_request(id: &str, change_key: &str, files: &[OutFile]) -> String {
    let files: String = files
        .iter()
        .map(|f| {
            format!(
                "<t:FileAttachment><t:Name>{}</t:Name><t:ContentType>{}</t:ContentType><t:Content>{}</t:Content></t:FileAttachment>",
                escape(&f.name),
                content_type(&f.name),
                B64.encode(&f.bytes)
            )
        })
        .collect();
    format!(
        r#"<m:CreateAttachment><m:ParentItemId Id="{}" ChangeKey="{}"/><m:Attachments>{files}</m:Attachments></m:CreateAttachment>"#,
        escape(id),
        escape(change_key)
    )
}

fn send_request(id: &str, change_key: &str) -> String {
    format!(
        r#"<m:SendItem SaveItemToFolder="true"><m:ItemIds><t:ItemId Id="{}" ChangeKey="{}"/></m:ItemIds><m:SavedItemFolderId><t:DistinguishedFolderId Id="sentitems"/></m:SavedItemFolderId></m:SendItem>"#,
        escape(id),
        escape(change_key)
    )
}

fn created_item(xml: &str) -> Result<(String, String)> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)?;
    doc.descendants()
        .find(|n| n.has_tag_name_local("ItemId"))
        .and_then(|n| Some((n.attribute("Id")?.to_string(), n.attribute("ChangeKey").unwrap_or_default().to_string())))
        .ok_or_else(|| Error::Other("The mail server didn't create the message.".into()))
}

/// Adding attachments changes the draft, so sending needs its new change key.
fn attached_change_key(xml: &str) -> Result<Option<String>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)?;
    Ok(doc.descendants().filter(|n| n.has_tag_name_local("AttachmentId")).filter_map(|n| n.attribute("RootItemChangeKey")).last().map(str::to_string))
}

fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" => "text/plain",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Typed text as a simple HTML body, so replies sit cleanly above the quoted original.
pub fn text_to_html(text: &str) -> String {
    let lines: Vec<String> = text.trim_end().lines().map(|l| if l.is_empty() { "&nbsp;".to_string() } else { escape(l).replace("  ", " &nbsp;") }).collect();
    let body: String = lines.iter().map(|l| format!("<div>{l}</div>")).collect();
    format!(r#"<html><body><div style="font-family:Calibri,Arial,sans-serif;font-size:11pt">{body}</div></body></html>"#)
}

/// Reads a To or Cc box: addresses separated by commas or semicolons, each
/// either bare or as `Name <address>`.
pub fn parse_addresses(input: &str) -> std::result::Result<Vec<Mailbox>, String> {
    let mut out = Vec::new();
    for part in input.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (name, address) = match (part.rfind('<'), part.ends_with('>')) {
            (Some(i), true) => (part[..i].trim().trim_matches('"').to_string(), part[i + 1..part.len() - 1].trim().to_string()),
            _ => (String::new(), part.to_string()),
        };
        let valid = address.split('@').count() == 2
            && address.split('@').all(|s| !s.is_empty())
            && address.rsplit('@').next().is_some_and(|d| d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
            && !address.chars().any(|c| c.is_whitespace() || "<>,;\"".contains(c));
        if !valid {
            return Err(format!("\"{part}\" isn't an email address."));
        }
        out.push(Mailbox { name, address, routing: String::new() });
    }
    Ok(out)
}

/// How a mailbox is written back into a To or Cc box.
pub fn address_text(m: &Mailbox) -> String {
    if m.name.is_empty() || m.name.contains([',', ';', '<', '>', '"']) {
        m.address.clone()
    } else {
        format!("{} <{}>", m.name, m.address)
    }
}

fn query_element(query: &str) -> String {
    match query.trim() {
        "" => String::new(),
        q => format!("<m:QueryString>{}</m:QueryString>", escape(q)),
    }
}

/// Downloads a message's image attachments. Returns the ones that can be shown
/// and how many couldn't be; one bad image doesn't lose the others.
pub async fn images(client: &GucClient, attachments: &[Attachment]) -> (Vec<MailImage>, usize) {
    let mut shown = Vec::new();
    let mut failed = 0;
    for a in attachments.iter().filter(|a| a.is_image()) {
        if a.size > MAX_IMAGE_BYTES {
            failed += 1;
            continue;
        }
        let image = attachment_bytes(client, &a.id).await.ok().and_then(|bytes| {
            let (width, height) = image_size(&bytes)?;
            Some(MailImage { bytes, width, height })
        });
        match image {
            Some(i) if i.width <= TINY_IMAGE_PX && i.height <= TINY_IMAGE_PX => {}
            Some(i) => shown.push(i),
            None => failed += 1,
        }
    }
    (shown, failed)
}

pub async fn attachment_bytes(client: &GucClient, id: &str) -> Result<Vec<u8>> {
    let body = format!(
        r#"<m:GetAttachment><m:AttachmentIds><t:AttachmentId Id="{}"/></m:AttachmentIds></m:GetAttachment>"#,
        escape(id)
    );
    attachment_content(&call(client, "GetAttachment", &body).await?)
}

fn attachment_content(xml: &str) -> Result<Vec<u8>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Other(e.to_string()))?;
    check_response(&doc)?;
    let content = child_text(doc.root_element(), "Content");
    let compact: String = content.split_whitespace().collect();
    B64.decode(compact).map_err(|e| Error::Other(e.to_string()))
}

fn attachments_of(message: roxmltree::Node) -> Vec<Attachment> {
    message
        .descendants()
        .filter(|n| n.has_tag_name_local("FileAttachment"))
        .map(|a| Attachment {
            id: a.descendants().find(|n| n.has_tag_name_local("AttachmentId")).and_then(|n| n.attribute("Id")).unwrap_or_default().to_string(),
            name: child_text(a, "Name"),
            content_type: child_text(a, "ContentType"),
            size: child_text(a, "Size").parse().unwrap_or(0),
            is_inline: child_text(a, "IsInline") == "true",
        })
        .filter(|a| !a.id.is_empty())
        .collect()
}

/// Reads only the image header, so a bad file is caught before display.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::io::Reader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

async fn call(client: &GucClient, action: &str, body: &str) -> Result<String> {
    let envelope = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
  xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"
  xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">
  <soap:Header><t:RequestServerVersion Version="Exchange2013"/></soap:Header>
  <soap:Body>{body}</soap:Body>
</soap:Envelope>"#
    );
    let soap_action = format!("http://schemas.microsoft.com/exchange/services/2006/messages/{action}");
    let resp = client
        .send(Method::POST, Url::parse(EWS).unwrap(), Body::Xml(envelope), &[("SOAPAction", &soap_action)])
        .await?;
    Ok(resp.text())
}

fn check_response(doc: &roxmltree::Document) -> Result<()> {
    let failed = doc
        .descendants()
        .find(|n| n.attribute("ResponseClass") == Some("Error"))
        .map(|n| child_text(n, "MessageText"));
    match failed {
        Some(msg) => Err(Error::Other(format!("Mail server: {msg}"))),
        None => Ok(()),
    }
}

trait LocalName {
    fn has_tag_name_local(&self, name: &str) -> bool;
}

impl LocalName for roxmltree::Node<'_, '_> {
    fn has_tag_name_local(&self, name: &str) -> bool {
        self.is_element() && self.tag_name().name() == name
    }
}

fn child_text(node: roxmltree::Node, name: &str) -> String {
    node.descendants().find(|n| n.has_tag_name_local(name)).and_then(|n| n.text()).unwrap_or_default().to_string()
}

fn sender(m: roxmltree::Node) -> String {
    let from = m.descendants().find(|n| n.has_tag_name_local("From"));
    from.map(|f| {
        let name = child_text(f, "Name");
        if name.is_empty() { child_text(f, "EmailAddress") } else { name }
    })
    .unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Readable plain text from an HTML mail body. HTML treats line breaks in the
/// source as spaces, so only real structure (paragraphs, `<br>`, lists, rows)
/// breaks a line here.
pub fn html_to_text(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    let mut w = TextWriter::default();
    w.walk(doc.root_element());
    let mut text = String::new();
    let mut blank = false;
    for line in w.out.lines().map(str::trim_end) {
        if line.is_empty() {
            blank = !text.is_empty();
        } else {
            // A long paragraph gets space above it too, except list items,
            // which stay attached to the line introducing them.
            let is_item = line.trim_start().starts_with("• ") || line.trim_start().starts_with(|c: char| c.is_ascii_digit());
            if blank || (line.chars().count() >= LONG_LINE && !is_item && !text.is_empty()) {
                text.push('\n');
            }
            blank = false;
            text.push_str(line);
            text.push('\n');
        }
    }
    text.trim_end().to_string()
}

/// A paragraph at least this long gets a blank line after it, so body text
/// separates while short lines like signatures stay together.
const LONG_LINE: usize = 70;
const MAX_CELL: usize = 80;

#[derive(Default)]
struct TextWriter {
    out: String,
    /// Open lists, innermost last: None for bullets, Some(next number) for numbered.
    lists: Vec<Option<u32>>,
}

impl TextWriter {
    fn at_line_start(&self) -> bool {
        self.out.is_empty() || self.out.ends_with('\n')
    }

    fn text(&mut self, raw: &str) {
        for c in raw.chars() {
            match c {
                '\u{200b}' | '\u{feff}' | '\u{ad}' => {}
                c if c.is_whitespace() || c == '\u{a0}' => {
                    if !self.at_line_start() && !self.out.ends_with(' ') {
                        self.out.push(' ');
                    }
                }
                c => self.out.push(c),
            }
        }
    }

    fn end_line(&mut self) {
        let kept = self.out.trim_end_matches(' ').len();
        self.out.truncate(kept);
        if !self.at_line_start() {
            self.out.push('\n');
        }
    }

    fn blank_line(&mut self) {
        self.end_line();
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    /// Ends a block, adding paragraph spacing after long text.
    fn end_block(&mut self) {
        self.end_line();
        let last = self.out.trim_end_matches('\n').rsplit('\n').next().unwrap_or("");
        if last.chars().count() >= LONG_LINE {
            self.blank_line();
        }
    }

    fn children(&mut self, el: scraper::ElementRef) {
        for child in el.children() {
            match child.value() {
                scraper::Node::Text(t) => self.text(t),
                scraper::Node::Element(_) => {
                    if let Some(e) = scraper::ElementRef::wrap(child) {
                        self.walk(e);
                    }
                }
                _ => {}
            }
        }
    }

    fn walk(&mut self, el: scraper::ElementRef) {
        let name = el.value().name();
        let style: String = el.value().attr("style").unwrap_or("").to_lowercase().split_whitespace().collect();
        if matches!(name, "style" | "script" | "head" | "title" | "meta" | "link" | "xml") || style.contains("display:none") {
            return;
        }
        // Word writes list markers as text, with bullets in a symbol font that
        // comes through as stray characters like "Ø" or "§".
        if style.contains("mso-list:ignore") {
            let marker: String = el.text().collect::<String>().replace('\u{a0}', " ").trim().to_string();
            let numbered = marker.chars().any(|c| c.is_ascii_digit())
                || (marker.chars().count() > 1 && marker.ends_with(['.', ')']));
            self.end_line();
            self.out.push_str(if numbered { marker.as_str() } else { "•" });
            self.out.push(' ');
            return;
        }
        match name {
            "br" => {
                if self.at_line_start() {
                    self.out.push('\n');
                } else {
                    self.end_line();
                }
            }
            "hr" => self.blank_line(),
            "ul" | "ol" => {
                self.end_line();
                self.lists.push(if name == "ol" { Some(1) } else { None });
                self.children(el);
                self.lists.pop();
                self.end_line();
            }
            "li" => {
                self.end_line();
                let depth = self.lists.len().saturating_sub(1);
                self.out.push_str(&"    ".repeat(depth));
                match self.lists.last_mut() {
                    Some(Some(n)) => {
                        self.out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => self.out.push_str("• "),
                }
                self.children(el);
                self.end_block();
            }
            "td" | "th" => {
                // Short cells in a multi-column row are data, so they stay on
                // one line even though Word wraps each in a paragraph. Single
                // cells and long ones are usually page layout, kept as blocks.
                let columns = el.parent().and_then(scraper::ElementRef::wrap).map_or(0, |row| {
                    row.children().filter_map(scraper::ElementRef::wrap).filter(|c| matches!(c.value().name(), "td" | "th")).count()
                });
                let cell: String = el.text().collect::<Vec<_>>().join(" ").replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ");
                if columns >= 2 && cell.chars().count() <= MAX_CELL {
                    if !cell.is_empty() {
                        if !self.at_line_start() {
                            self.out.push_str("   ");
                        }
                        self.out.push_str(&cell);
                    }
                } else {
                    self.end_line();
                    self.children(el);
                    self.end_block();
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.blank_line();
                self.children(el);
                self.blank_line();
            }
            "p" => {
                self.end_line();
                let start = self.out.len();
                self.children(el);
                if self.out[start..].trim().is_empty() {
                    self.blank_line(); // Word spaces paragraphs with empty ones.
                } else {
                    self.end_block();
                }
            }
            "div" | "table" | "tbody" | "thead" | "tfoot" | "tr" | "blockquote" | "center" | "section" | "article" | "header" | "footer" | "address" | "dl" | "dt" | "dd" => {
                self.end_line();
                self.children(el);
                self.end_block();
            }
            _ => self.children(el),
        }
    }
}

fn links_in(html: &str) -> Vec<(String, String)> {
    let doc = scraper::Html::parse_document(html);
    let sel = scraper::Selector::parse("a[href]").expect("static selector");
    let mut seen = std::collections::HashSet::new();
    doc.select(&sel)
        .filter_map(|a| {
            let href = a.value().attr("href")?.trim().to_string();
            let ok = href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:");
            if !ok || !seen.insert(href.clone()) {
                return None;
            }
            let text = crate::page::clean(&a);
            Some((if text.is_empty() { href.clone() } else { text }, href))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_find_item_response() {
        let xml = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>
          <m:FindItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
          <m:ResponseMessages><m:FindItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="42"><t:Items><t:Message><t:ItemId Id="AAA=" ChangeKey="x"/>
          <t:Subject>Midterm rooms</t:Subject><t:DateTimeReceived>2026-10-20T09:00:00Z</t:DateTimeReceived>
          <t:From><t:Mailbox><t:Name>Exams Office</t:Name></t:Mailbox></t:From><t:IsRead>false</t:IsRead>
          </t:Message></t:Items></m:RootFolder></m:FindItemResponseMessage></m:ResponseMessages></m:FindItemResponse></s:Body></s:Envelope>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        check_response(&doc).unwrap();
        let m = doc.descendants().find(|n| n.has_tag_name_local("Message")).unwrap();
        assert_eq!(child_text(m, "Subject"), "Midterm rooms");
        assert_eq!(sender(m), "Exams Office");
    }

    #[test]
    fn reads_attachments_and_their_content() {
        let xml = r#"<m:Message xmlns:m="m" xmlns:t="t"><t:Attachments>
          <t:FileAttachment><t:AttachmentId Id="A1"/><t:Name>poster.jpg</t:Name><t:ContentType>image/jpeg</t:ContentType><t:Size>285963</t:Size><t:IsInline>true</t:IsInline></t:FileAttachment>
          <t:FileAttachment><t:AttachmentId Id="A2"/><t:Name>sheet.xlsx</t:Name><t:ContentType>application/vnd.ms-excel</t:ContentType><t:Size>72122</t:Size></t:FileAttachment>
          <t:FileAttachment><t:AttachmentId Id="A3"/><t:Name>photo.PNG</t:Name><t:ContentType>application/octet-stream</t:ContentType><t:Size>9</t:Size></t:FileAttachment>
        </t:Attachments></m:Message>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let a = attachments_of(doc.root_element());
        assert_eq!(a.len(), 3);
        assert_eq!((a[0].id.as_str(), a[0].name.as_str(), a[0].size), ("A1", "poster.jpg", 285963));
        assert_eq!(a.iter().map(Attachment::is_image).collect::<Vec<_>>(), vec![true, false, true]);
        assert_eq!(a.iter().map(Attachment::is_listed).collect::<Vec<_>>(), vec![false, true, true], "only inline images stay out of the list");

        let reply = r#"<s:Envelope xmlns:s="s" xmlns:m="m" xmlns:t="t"><s:Body><m:GetAttachmentResponseMessage ResponseClass="Success">
          <m:Attachments><t:FileAttachment><t:Content>aGVs
          bG8=</t:Content></t:FileAttachment></m:Attachments></m:GetAttachmentResponseMessage></s:Body></s:Envelope>"#;
        assert_eq!(attachment_content(reply).unwrap(), b"hello");
    }

    #[test]
    fn reads_image_size_without_trusting_the_file() {
        assert_eq!(image_size(include_bytes!("../assets/brand/app-icon-256.png")), Some((256, 256)));
        assert_eq!(image_size(b"not an image"), None);
    }

    #[test]
    fn search_query_is_escaped_and_optional() {
        assert_eq!(query_element("   "), "");
        assert_eq!(query_element(" Q&A <quiz> "), "<m:QueryString>Q&amp;A &lt;quiz&gt;</m:QueryString>");
    }

    #[test]
    fn mail_html_becomes_text() {
        let t = html_to_text("<html><head><style>p{}</style></head><body><p>Dear students,</p><p>Rooms&nbsp;are <b>posted</b>.</p></body></html>");
        assert_eq!(t, "Dear students,\nRooms are posted.");
    }

    #[test]
    fn source_line_breaks_are_not_real_breaks() {
        let t = html_to_text("<p>GUC is pleased to announce\n   a new offer\r\nwith Lime.</p>");
        assert_eq!(t, "GUC is pleased to announce a new offer with Lime.");
        assert_eq!(html_to_text("one<br>two<br/><br>three"), "one\ntwo\n\nthree");
    }

    #[test]
    fn word_list_markers_become_bullets_and_numbers() {
        let html = r#"<p class="MsoListParagraph" style="mso-list:l2 level1 lfo4"><!--[if !supportLists]--><span style="font-family:Wingdings"><span style="mso-list:Ignore">Ø<span style="font:7.0pt">&nbsp;
          </span></span></span><!--[endif]--><b>Midterm</b> exam</p>
          <p style="mso-list:l1"><span style='mso-list:Ignore'>§&nbsp;</span>Second</p>
          <p><span style="mso-list: Ignore">1.<span>&nbsp;&nbsp;</span></span>Numbered</p>
          <p><span style="mso-list:Ignore">o&nbsp;</span>Sub point</p>"#;
        assert_eq!(html_to_text(html), "• Midterm exam\n• Second\n1. Numbered\n• Sub point");
    }

    #[test]
    fn html_lists_and_tables_keep_their_shape() {
        assert_eq!(html_to_text("<ul><li>a</li><li>b</li></ul><ol><li>c</li><li>d</li></ol>"), "• a\n• b\n1. c\n2. d");
        assert_eq!(html_to_text("<ul><li>a<ul><li>b</li></ul></li></ul>"), "• a\n    • b");
        assert_eq!(html_to_text("<table><tr><td>Day</td><td>Room</td></tr><tr><td>Sat</td><td></td></tr></table>"), "Day   Room\nSat");
        assert_eq!(html_to_text("<table><tr><td><p>1000</p></td><td><p>22nd of <b>November</b></p></td></tr></table>"), "1000   22nd of November");
        assert_eq!(html_to_text("<table><tr><td><p>Layout one</p><p>Layout two</p></td></tr></table>"), "Layout one\nLayout two");
    }

    #[test]
    fn paragraphs_are_spaced_but_signatures_stay_together() {
        let long = "x".repeat(LONG_LINE);
        let t = html_to_text(&format!("<p>Hello,</p><p>&nbsp;</p><p>&nbsp;</p><p>{long}</p><p>Dr. Name</p><p>Department</p><div style='display:none'>hidden</div>"));
        assert_eq!(t, format!("Hello,\n\n{long}\n\nDr. Name\nDepartment"));
        assert_eq!(html_to_text(&format!("<p>Hi,</p><p>{long}</p>")), format!("Hi,\n\n{long}"));
        assert_eq!(html_to_text(&format!("<p>Points:</p><ul><li>{long}</li></ul>")), format!("Points:\n• {long}"));
    }

    #[test]
    fn reads_addresses_typed_into_to_and_cc() {
        let got = parse_addresses(" a.b@guc.edu.eg; Dr. Name <c.d@student.guc.edu.eg>, ").unwrap();
        assert_eq!(got.iter().map(|m| (m.name.as_str(), m.address.as_str())).collect::<Vec<_>>(), [("", "a.b@guc.edu.eg"), ("Dr. Name", "c.d@student.guc.edu.eg")]);
        assert!(parse_addresses("").unwrap().is_empty());
        for bad in ["ahmed.ali", "a@b", "a@@b.com", "two words@guc.edu.eg", "a@guc.edu.eg.", "@guc.edu.eg"] {
            assert!(parse_addresses(bad).is_err(), "{bad}");
        }
        let m = &parse_addresses("Dr. Name <c.d@guc.edu.eg>").unwrap()[0];
        assert_eq!(parse_addresses(&address_text(m)).unwrap()[0], *m);
    }

    #[test]
    fn typed_text_becomes_escaped_html() {
        let html = text_to_html("Hi <all> & co\n\nThanks");
        assert!(html.contains("<div>Hi &lt;all&gt; &amp; co</div><div>&nbsp;</div><div>Thanks</div>"), "{html}");
    }

    #[test]
    fn builds_send_requests() {
        let to = parse_addresses("x@guc.edu.eg").unwrap();
        let mut out = Outgoing { kind: Compose::New, original: None, to, cc: vec![], subject: "A & B".into(), body: "Hello".into(), attachments: vec![] };
        let new = create_request(&out).unwrap();
        assert!(new.contains(r#"MessageDisposition="SaveOnly""#) && new.contains("<t:Subject>A &amp; B</t:Subject>"));
        assert!(new.contains("<t:ToRecipients><t:Mailbox><t:EmailAddress>x@guc.edu.eg</t:EmailAddress></t:Mailbox></t:ToRecipients>"));
        assert!(!new.contains("CcRecipients"));
        // Element order matters to Exchange: subject, body, then recipients.
        assert!(new.find("<t:Subject>") < new.find("<t:Body") && new.find("<t:Body") < new.find("<t:ToRecipients>"));

        out.kind = Compose::ReplyAll;
        assert!(create_request(&out).is_err(), "a reply needs its original");
        out.original = Some(("ID1".into(), "CK1".into()));
        let reply = create_request(&out).unwrap();
        assert!(reply.contains(r#"<t:ReplyAllToItem><t:Subject>"#) && reply.contains(r#"<t:ReferenceItemId Id="ID1" ChangeKey="CK1"/><t:NewBodyContent BodyType="HTML">"#));
        assert!(reply.find("<t:ToRecipients>") < reply.find("<t:ReferenceItemId"));

        let attach = attach_request("ID2", "CK2", &[OutFile { name: "Notes.pdf".into(), bytes: b"hi".to_vec() }]);
        assert!(attach.contains("<t:Name>Notes.pdf</t:Name><t:ContentType>application/pdf</t:ContentType><t:Content>aGk=</t:Content>"));
        assert!(send_request("ID2", "CK3").contains(r#"<t:ItemId Id="ID2" ChangeKey="CK3"/>"#));
    }

    #[test]
    fn reads_send_responses() {
        let created = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><m:CreateItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"><m:ResponseMessages><m:CreateItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id="NEW" ChangeKey="CK"/></t:Message></m:Items></m:CreateItemResponseMessage></m:ResponseMessages></m:CreateItemResponse></s:Body></s:Envelope>"#;
        assert_eq!(created_item(created).unwrap(), ("NEW".into(), "CK".into()));
        let attached = r#"<r xmlns:t="t"><t:FileAttachment><t:AttachmentId Id="A" RootItemId="NEW" RootItemChangeKey="CK2"/></t:FileAttachment></r>"#;
        assert_eq!(attached_change_key(attached).unwrap().as_deref(), Some("CK2"));
        let failed = r#"<r><m ResponseClass="Error"><MessageText>The message exceeds the maximum supported size.</MessageText></m></r>"#;
        assert!(created_item(failed).unwrap_err().to_string().contains("maximum supported size"));
    }

    #[test]
    fn recognises_own_address() {
        let m = |a: &str| Mailbox { address: a.into(), ..Default::default() };
        assert!(is_me(&m("Ahmed.Ali@student.guc.edu.eg"), "ahmed.ali"));
        assert!(!is_me(&m("ahmed.ali@gmail.com"), "ahmed.ali"));
        assert!(!is_me(&m("ahmed.ali2@student.guc.edu.eg"), "ahmed.ali"));
    }

    #[test]
    fn collects_links_once() {
        let l = links_in(r##"<a href="https://forms.office.com/x">Register here</a> <a href="https://forms.office.com/x">again</a> <a href="#top">top</a> <a href="mailto:sa@guc.edu.eg"></a>"##);
        assert_eq!(l, vec![
            ("Register here".to_string(), "https://forms.office.com/x".to_string()),
            ("mailto:sa@guc.edu.eg".to_string(), "mailto:sa@guc.edu.eg".to_string()),
        ]);
    }
}
