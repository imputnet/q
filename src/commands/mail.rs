use crate::cli::usage;
use crate::patchfile::patch_header;
use crate::state::{PatchEntry, State};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use lexopt::prelude::*;
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn cmd_mail(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut mbox = None::<PathBuf>;
    let mut send = false;
    let mut subject = None::<String>;
    let mut prefix = "patch".to_string();
    let mut intro = String::new();
    let mut intro_specified = false;
    let mut sender = None::<String>;
    let mut from = None::<String>;
    let mut to = Vec::<String>::new();
    let mut cc = Vec::<String>::new();
    let mut bcc = Vec::<String>::new();
    let mut signature = default_mail_signature();
    let mut reply_to = None::<ReplyHeaders>;
    let mut charset = default_mail_charset();
    let mut range_args = Vec::<String>::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("mbox") => mbox = Some(PathBuf::from(parser.value()?)),
            Long("send") => send = true,
            Long("subject") => subject = Some(parser.value()?.string()?),
            Long("prefix") => prefix = parser.value()?.string()?,
            Short('m') => {
                if intro_specified {
                    bail!("Introduction message already specified");
                }
                intro = parser.value()?.string()?;
                intro_specified = true;
            }
            Short('M') => {
                if intro_specified {
                    bail!("Introduction message already specified");
                }
                intro = fs::read_to_string(PathBuf::from(parser.value()?))?;
                intro_specified = true;
            }
            Long("sender") => sender = Some(parser.value()?.string()?),
            Long("from") => from = Some(parser.value()?.string()?),
            Long("to") => to.push(parser.value()?.string()?),
            Long("cc") => cc.push(parser.value()?.string()?),
            Long("bcc") => bcc.push(parser.value()?.string()?),
            Long("charset") => {
                charset = parser.value()?.string()?;
            }
            Long("signature") => {
                let value = parser.value()?.string()?;
                signature = if value == "-" {
                    Some(String::new())
                } else {
                    Some(fs::read_to_string(value)?)
                };
            }
            Long("reply-to") => {
                reply_to = Some(read_reply_headers(&PathBuf::from(parser.value()?))?);
            }
            Value(v) => range_args.push(v.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    let state = State::discover(false)?;
    let entries = state.entries()?;
    let selected_entries = select_mail_entries(&state, &entries, &range_args)?;
    if !send && mbox.is_none() {
        bail!("{}", usage("mail"));
    }
    let subject = match (subject, reply_to.as_ref()) {
        (Some(subject), _) => subject,
        (None, Some(reply)) => reply
            .subject
            .as_deref()
            .map(|subject| format!("Re: {}", strip_reply_prefix(subject)))
            .or_else(|| extract_explicit_subject(&intro))
            .context("Introduction has no subject header")?,
        (None, None) => {
            extract_explicit_subject(&intro).context("Introduction has no subject header")?
        }
    };
    let mut patches = Vec::new();
    for entry in &selected_entries {
        let path = state.patch_path(&entry.name);
        let text = fs::read_to_string(&path).unwrap_or_default();
        let header = patch_header(&path)?;
        let meta = mail_metadata(&header).with_context(|| {
            format!(
                "Unable to extract a subject header from {}",
                state.display_patch(&entry.name)
            )
        })?;
        patches.push((entry, text, meta));
    }
    let mut seen = HashMap::<String, Vec<String>>::new();
    for (entry, _, meta) in &patches {
        seen.entry(normalize_mail_subject(&meta.subject))
            .or_default()
            .push(state.display_patch(&entry.name));
    }
    if let Some(duplicates) = seen.values().find(|subjects| subjects.len() > 1) {
        bail!(
            "Patches {} have duplicate subject headers.",
            duplicates.join(", ")
        );
    }
    let sender = match sender {
        Some(sender) => sender,
        None => default_sender()?,
    };
    let sender_address = mail_address(&sender);
    let from = from.unwrap_or_else(|| sender.clone());
    let mut messages = Vec::<MailMessage>::new();
    let mut intro_message = String::new();
    write_generated_mail_headers(&mut intro_message, &sender);
    write_mail_envelope_headers(&mut intro_message, &from, &to, &cc, &bcc);
    if let Some(reply) = &reply_to {
        if let Some(message_id) = &reply.message_id {
            intro_message.push_str(&format!("In-Reply-To: {message_id}\n"));
        }
        if let Some(references) = reply.references_for_reply() {
            intro_message.push_str(&format!("References: {references}\n"));
        }
    }
    intro_message.push_str(&format!(
        "Subject: [{prefix} 0/{}] {subject}\n\n{intro}\n",
        selected_entries.len()
    ));
    append_mail_signature(&mut intro_message, signature.as_deref());
    messages.push(MailMessage {
        text: intro_message,
        recipients: mail_recipients(&to, &cc, &bcc),
    });
    for (idx, (_entry, text, meta)) in patches.iter().enumerate() {
        let mut message = String::new();
        write_generated_mail_headers(&mut message, &sender);
        write_mail_envelope_headers(&mut message, &from, &to, &cc, &bcc);
        if let Some(reply) = &reply_to
            && let Some(references) = reply.references_for_reply()
        {
            message.push_str(&format!("References: {references}\n"));
        }
        let mut recipients = mail_recipients(&to, &cc, &bcc);
        for to in &meta.to {
            message.push_str(&format!("To: {}\n", format_recipient(to, &charset)));
            recipients.push(mail_address(to));
        }
        if !meta.cc.is_empty() {
            let formatted: Vec<_> = meta
                .cc
                .iter()
                .map(|cc| format_recipient(cc, &charset))
                .collect();
            message.push_str(&format!("Cc: {}\n", formatted.join(",\n ")));
            recipients.extend(meta.cc.iter().map(|cc| mail_address(cc)));
        }
        message.push_str(&format!(
            "Subject: [{prefix} {}/{}] {}\n\n",
            idx + 1,
            selected_entries.len(),
            normalize_mail_subject(&meta.subject)
        ));
        insert_patch_mime_headers(&mut message, &charset);
        message.push_str(&mail_body_without_subject(text));
        append_mail_signature(&mut message, signature.as_deref());
        messages.push(MailMessage {
            text: message,
            recipients,
        });
    }
    if send {
        send_mail_messages(&sender, &messages)?;
    } else if let Some(out_path) = mbox {
        let mut out = String::new();
        for message in messages {
            out.push_str(&format!("From {sender_address} Thu Jan  1 00:00:00 1970\n"));
            out.push_str(&message.text);
            out.push('\n');
        }
        fs::write(out_path, out)?;
    }
    Ok(())
}

struct MailMessage {
    text: String,
    recipients: Vec<String>,
}

fn select_mail_entries(
    state: &State,
    entries: &[PatchEntry],
    range_args: &[String],
) -> Result<Vec<PatchEntry>> {
    if range_args.len() > 2 {
        bail!("{}", usage("mail"));
    }
    let (Some(bottom), Some(top)) = (entries.first(), entries.last()) else {
        return Ok(Vec::new());
    };
    let first = match range_args.first().map(String::as_str) {
        Some("-") | None => bottom.name.clone(),
        Some(patch) => state.find_patch(patch)?,
    };
    let last = match range_args.get(1).map(String::as_str) {
        Some("-") => top.name.clone(),
        Some(patch) => state.find_patch(patch)?,
        None if range_args.is_empty() => top.name.clone(),
        None => first.clone(),
    };
    let start = entries
        .iter()
        .position(|entry| entry.name == first)
        .with_context(|| format!("Patch {} not found", state.display_patch(&first)))?;
    let end = entries
        .iter()
        .position(|entry| entry.name == last)
        .with_context(|| format!("Patch {} not found", state.display_patch(&last)))?;
    if start > end {
        bail!(
            "Patch {} not applied before patch {}",
            state.display_patch(&first),
            state.display_patch(&last)
        );
    }
    Ok(entries[start..=end].to_vec())
}

struct ReplyHeaders {
    message_id: Option<String>,
    references: Option<String>,
    in_reply_to: Option<String>,
    subject: Option<String>,
}

impl ReplyHeaders {
    fn references_for_reply(&self) -> Option<String> {
        if let Some(references) = &self.references {
            if let Some(message_id) = &self.message_id {
                return Some(format!("{references}\n {message_id}"));
            }
            return Some(references.clone());
        }
        if let Some(in_reply_to) = &self.in_reply_to
            && !in_reply_to.contains("@*@")
        {
            return Some(in_reply_to.clone());
        }
        self.message_id.clone()
    }
}

fn read_reply_headers(path: &Path) -> Result<ReplyHeaders> {
    let text = fs::read_to_string(path)?;
    let headers = parse_mail_headers(&text);
    Ok(ReplyHeaders {
        message_id: headers.get("message-id").cloned(),
        references: headers.get("references").cloned(),
        in_reply_to: headers.get("in-reply-to").cloned(),
        subject: headers.get("subject").cloned(),
    })
}

fn parse_mail_headers(text: &str) -> std::collections::HashMap<String, String> {
    let mut headers = HashMap::<String, String>::new();
    let mut current = None::<String>;
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(name) = &current
                && let Some(value) = headers.get_mut(name)
            {
                value.push('\n');
                value.push_str(line);
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        headers.insert(name.clone(), value.trim_start().to_string());
        current = Some(name);
    }
    headers
}

fn strip_reply_prefix(subject: &str) -> String {
    let mut subject = subject.trim_start();
    loop {
        let lower = subject.to_ascii_lowercase();
        if lower.starts_with("re:") {
            subject = subject[3..].trim_start();
        } else {
            return subject.to_string();
        }
    }
}

fn write_mail_envelope_headers(
    out: &mut String,
    from: &str,
    to: &[String],
    cc: &[String],
    bcc: &[String],
) {
    out.push_str(&format!("From: {from}\n"));
    if !to.is_empty() {
        out.push_str(&format!("To: {}\n", to.join(",")));
    }
    if !cc.is_empty() {
        out.push_str(&format!("Cc: {}\n", cc.join(",")));
    }
    if !bcc.is_empty() {
        out.push_str(&format!("Bcc: {}\n", bcc.join(",")));
    }
}

fn write_generated_mail_headers(out: &mut String, sender: &str) {
    let domain = mail_address(sender)
        .split_once('@')
        .map(|(_, domain)| domain.to_string())
        .unwrap_or_else(|| "localhost".to_string());
    out.push_str(&format!(
        "Message-ID: <{}@{}>\n",
        message_id_stamp(),
        domain
    ));
    out.push_str(&format!("User-Agent: quilt/{}\n", env!("Q_VERSION")));
    let now: DateTime<Local> = Local::now();
    out.push_str(&format!(
        "Date: {}\n",
        now.format("%a, %d %b %Y %H:%M:%S %z")
    ));
}

fn message_id_stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

fn insert_patch_mime_headers(message: &mut String, charset: &str) {
    if let Some(pos) = message.find("\n\n") {
        message.insert_str(
            pos + 1,
            &format!("MIME-Version: 1.0\nContent-Type: text/plain; charset={charset}\n"),
        );
    }
}

fn mail_recipients(to: &[String], cc: &[String], bcc: &[String]) -> Vec<String> {
    to.iter()
        .chain(cc)
        .chain(bcc)
        .map(|recipient| mail_address(recipient))
        .collect()
}

/// quilt's fallback envelope sender: `$LOGNAME@<mailname or FQDN>`.
fn default_sender() -> Result<String> {
    let hostname = fs::read_to_string("/etc/mailname")
        .ok()
        .or_else(|| command_stdout("hostname", &["-f"]))
        .filter(|name| name.trim().contains('.'))
        .or_else(|| command_stdout("hostname", &[]))
        .unwrap_or_default();
    let user = env::var("LOGNAME")
        .ok()
        .or_else(|| command_stdout("whoami", &[]))
        .unwrap_or_default();
    let sender = format!("{}@{}", user.trim(), hostname.trim());
    match sender.split_once('@') {
        Some((user, host)) if !user.is_empty() && host.contains('.') => Ok(sender),
        _ => bail!("Could not determine the envelope sender address. Please use --sender."),
    }
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn send_mail_messages(sender: &str, messages: &[MailMessage]) -> Result<()> {
    let sendmail = env::var("QUILT_SENDMAIL").unwrap_or_else(|_| "sendmail".to_string());
    let base_args = if let Some(value) = env::var_os("QUILT_SENDMAIL_ARGS") {
        shlex::split(&value.to_string_lossy()).context("failed to parse QUILT_SENDMAIL_ARGS")?
    } else {
        vec!["-f".to_string(), sender.to_string()]
    };
    for message in messages {
        let mut args = base_args.clone();
        args.extend(message.recipients.iter().cloned());
        println!("{sendmail} {}", args.join(" "));
        let mut child = Command::new(&sendmail)
            .args(&args)
            .stdin(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run {sendmail}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(remove_bcc_header(&message.text).as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("sendmail failed");
        }
    }
    Ok(())
}

fn remove_bcc_header(message: &str) -> String {
    message
        .lines()
        .filter(|line| !line.to_ascii_lowercase().starts_with("bcc:"))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn append_mail_signature(out: &mut String, signature: Option<&str>) {
    if let Some(signature) = signature
        && !signature.is_empty()
    {
        out.push_str("\n-- \n");
        out.push_str(signature.trim_end());
        out.push('\n');
    }
}

fn default_mail_signature() -> Option<String> {
    let path = env::var_os("HOME").map(PathBuf::from)?.join(".signature");
    fs::read_to_string(path).ok()
}

fn default_mail_charset() -> String {
    let locale = env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| env::var("LANG").ok())
        .unwrap_or_default();
    if locale.ends_with(".UTF-8") || locale.ends_with(".utf8") || locale.ends_with(".UTF8") {
        "UTF-8".to_string()
    } else {
        "ISO-8859-15".to_string()
    }
}

fn mail_address(value: &str) -> String {
    if let Some(start) = value.rfind('<')
        && let Some(end) = value[start + 1..].find('>')
    {
        return value[start + 1..start + 1 + end].to_string();
    }
    value.to_string()
}

struct MailMeta {
    subject: String,
    to: Vec<String>,
    cc: Vec<String>,
}

fn mail_metadata(header: &str) -> Option<MailMeta> {
    let subject = extract_subject(header)?;
    let mut to = Vec::new();
    let mut cc = Vec::new();
    for line in header.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower
            .starts_with("to:")
            .then(|| line.split_once(':').map(|(_, value)| value))
            .flatten()
        {
            to.push(value.trim().to_string());
        } else if mail_cc_header(&lower)
            && let Some((_, value)) = line.split_once(':')
        {
            cc.push(value.trim().to_string());
        }
    }
    Some(MailMeta { subject, to, cc })
}

fn mail_cc_header(lower: &str) -> bool {
    [
        "cc:",
        "signed-off-by:",
        "acked-by:",
        "suggested-by:",
        "reviewed-by:",
        "requested-by:",
        "reported-by:",
        "tested-by:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn extract_explicit_subject(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("Subject:").map(|s| s.trim().to_string()))
}

fn extract_subject(header: &str) -> Option<String> {
    let lines: Vec<_> = header.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(subject) = line.strip_prefix("Subject:") {
            let mut subject = subject.trim().to_string();
            for continuation in lines
                .iter()
                .skip(idx + 1)
                .take_while(|l| l.starts_with(' '))
            {
                subject.push('\n');
                subject.push_str(continuation);
            }
            return Some(subject);
        }
    }
    if let Some(start) = lines.iter().position(|line| *line == "DESC")
        && let Some(end) = lines
            .iter()
            .skip(start + 1)
            .position(|line| *line == "EDESC")
    {
        return Some(lines[start + 1..start + 1 + end].join(" "));
    }
    let paragraph: Vec<_> = lines
        .iter()
        .take_while(|line| !line.trim().is_empty())
        .map(|line| line.trim())
        .collect();
    if paragraph.is_empty() {
        return None;
    }
    let subject = paragraph.join(" ");
    if subject.len() > 150 {
        None
    } else {
        Some(subject)
    }
}

fn normalize_mail_subject(subject: &str) -> String {
    let mut value = subject.trim().to_string();
    loop {
        let lower = value.to_ascii_lowercase();
        let next =
            if lower.starts_with("re:") || lower.starts_with("aw:") || lower.starts_with("fw:") {
                Some(value[3..].trim())
            } else if lower.starts_with("fwd:") {
                Some(value[4..].trim())
            } else if lower.starts_with("[patch") {
                value.find(']').map(|idx| value[idx + 1..].trim())
            } else {
                None
            };
        let Some(next) = next else {
            return value;
        };
        value = next.to_string();
    }
}

/// Format `Name <addr>` the way quilt's edmail does: quote display names that
/// contain RFC 5322 specials, and RFC 2047-encode non-ASCII ones.
fn format_recipient(recipient: &str, charset: &str) -> String {
    let Some((name, addr)) = recipient
        .strip_suffix('>')
        .and_then(|rest| rest.rsplit_once('<'))
    else {
        return recipient.to_string();
    };
    let name = name.trim_end();
    let display = if let Some(quoted) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        if quoted.is_ascii() {
            name.to_string()
        } else {
            encode_header_word(&quoted.replacen('\\', "", 1), charset)
        }
    } else if !name.is_ascii() || name.contains(['"', '\\']) {
        encode_header_word(name, charset)
    } else if name.contains(|c| "()<>[]:;@\\,\".".contains(c)) {
        format!("\"{name}\"")
    } else {
        name.to_string()
    };
    format!("{display} <{addr}>")
}

fn encode_header_word(word: &str, charset: &str) -> String {
    let mut encoded = format!("=?{charset}?q?");
    for byte in word.bytes() {
        if byte == b'\t' || (0x21..=0x7e).contains(&byte) && !b"?=_".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("={byte:02X}"));
        }
    }
    encoded.push_str("?=");
    encoded
}

fn mail_body_without_subject(text: &str) -> String {
    let mut out = String::new();
    let mut skip_subject = false;
    let mut skip_desc = false;
    for line in text.lines() {
        if line.starts_with("Subject:") {
            skip_subject = true;
            continue;
        }
        if skip_subject && line.starts_with(' ') {
            continue;
        }
        skip_subject = false;
        if line == "DESC" {
            skip_desc = true;
            continue;
        }
        if skip_desc {
            if line == "EDESC" {
                skip_desc = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
