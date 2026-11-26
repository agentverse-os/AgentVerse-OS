//! Text for the notification channels (docs/agents-ux.md §9.12): Markdown of an agent's answer or report turned into
//! what each service understands (Telegram HTML, Slack mrkdwn, Discord Markdown, plain text), cut into messages of the
//! service's size, with secret-looking strings closed and mentions defused. Nothing here talks to the network.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dialect { Telegram, Slack, Discord, Plain }

/// What a closed secret is replaced with.
pub const MASK: &str = "«скрыто»";

static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| [
    // to its END line — or, when the END line is not there (a key cut short, pasted in part), to the end of the text
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
    r"\bsk-[A-Za-z0-9_\-]{16,}",
    r"\bgh[pousr]_[A-Za-z0-9]{20,}",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}",
    r"\bglpat-[A-Za-z0-9_\-]{16,}",
    r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\bAIza[0-9A-Za-z_\-]{30,}",
    r"\b\d{8,10}:AA[A-Za-z0-9_\-]{30,}",
    r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
    r"\btk_[a-z0-9]{20,}",
].iter().map(|p| Regex::new(p).unwrap()).collect());
/// `password = …`, `API_KEY: …`, `token=…`: the name stays, the value is closed. The name is the word itself, with a
/// prefix or a suffix through an underscore (`db_password`, `token_value`) — not a longer word ("tokens" is a count).
static NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b((?:[a-z0-9_]*_)?(?:password|passwd|secret|token|api[_-]?key|access[_-]?key)(?:_[a-z0-9_]+)?)\b(\s*[:=]\s*)(["']?)([^\s"'`,;]{6,})"#).unwrap());

/// Close what looks like a key or a token. A safety net, not a guarantee: an unusual secret passes.
pub fn mask_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for re in SECRETS.iter() { if re.is_match(&out) { out = re.replace_all(&out, MASK).into_owned(); } }
    // a value of digits alone is a number (`max_token = 200000`), not a secret
    NAMED.replace_all(&out, |c: &regex::Captures| if c[4].contains(MASK) || c[4].chars().all(|x| x.is_ascii_digit() || matches!(x, '.' | '_')) { c[0].to_string() } else { format!("{}{}{}{MASK}", &c[1], &c[2], &c[3]) }).into_owned()
}
/// Whether a text carries a secret of a well-known shape (a private key, a provider token): such a file is not sent.
pub fn has_secret(text: &str) -> bool { SECRETS.iter().any(|re| re.is_match(text)) }

/// File names that are secrets by their very name.
pub fn secret_name(name: &str) -> bool {
    let n = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    n == ".env" || n.starts_with(".env.") || n.ends_with(".env") || n.ends_with(".pem") || n.ends_with(".key") || n.ends_with(".p12") || n.ends_with(".pfx") || n.ends_with(".kdbx") || n.ends_with(".keystore")
        || n.starts_with("id_rsa") || n.starts_with("id_ed25519") || n.starts_with("id_ecdsa") || n.starts_with("id_dsa") || n.starts_with("credentials") || n.starts_with("secrets.")
        || matches!(n.as_str(), ".netrc" | ".npmrc" | ".pypirc" | ".git-credentials" | ".htpasswd" | "kubeconfig" | ".claude.json" | "auth.json" | "mcp.json" | "service-account.json")
}

/// A mention that would ping a whole chat is broken with a zero-width space; nothing else changes for the reader.
pub fn defuse_mentions(text: &str) -> String {
    text.replace("@everyone", "@\u{200b}everyone").replace("@here", "@\u{200b}here").replace("@channel", "@\u{200b}channel")
        .replace("<!channel>", "<\u{200b}!channel>").replace("<!here>", "<\u{200b}!here>").replace("<!everyone>", "<\u{200b}!everyone>")
}

fn esc(d: Dialect, s: &str) -> String {
    match d {
        Dialect::Telegram | Dialect::Slack => s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
        Dialect::Discord | Dialect::Plain => s.to_string(),
    }
}

/// Plain words made safe for the dialect: nothing in them is read as markup or as a mention.
pub fn text(d: Dialect, s: &str) -> String { esc(d, &defuse_mentions(s)) }

/// The first line of a message: plain words made safe for the dialect and set in bold where the dialect has bold.
pub fn head(d: Dialect, title: &str, rest: &str) -> String {
    let (t, r) = (esc(d, &defuse_mentions(title)), esc(d, &defuse_mentions(rest)));
    match d {
        Dialect::Telegram => format!("<b>{t}</b>{r}"),
        Dialect::Slack => format!("*{}*{r}", t.replace('*', "∗")),
        Dialect::Discord => format!("**{}**{r}", t.replace('*', "∗")),
        Dialect::Plain => format!("{t}{r}"),
    }
}
/// The "open" link under a message.
pub fn link(d: Dialect, label: &str, url: &str) -> String {
    match d {
        Dialect::Telegram => format!("<a href=\"{}\">{}</a>", esc(d, url).replace('"', "%22"), esc(d, label)),
        Dialect::Slack => format!("<{}|{}>", url.replace(['<', '>', '|'], ""), esc(d, label)),
        Dialect::Discord => format!("[{label}](<{url}>)"),
        Dialect::Plain => format!("{label}: {url}"),
    }
}

/// Markdown → the dialect. Headings become bold lines, lists keep their marks, tables go into a fixed-width block,
/// raw HTML is shown as text. Discord takes Markdown almost as it is, so only tables and mentions are touched there.
pub fn render(d: Dialect, md: &str) -> String { render_with(d, md, false) }
/// The same with tables set without padding: a table with one long cell is padded into a text many times its source.
pub fn render_tight(d: Dialect, md: &str) -> String { render_with(d, md, true) }
/// A column is padded to its widest cell, but not beyond this: a longer cell simply sticks out.
const PAD_MAX: usize = 28;
fn render_with(d: Dialect, md: &str, tight: bool) -> String {
    let md = defuse_mentions(md);
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut in_code = false;
    let mut link_url: Vec<String> = Vec::new();
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut cell = String::new();
    let mut quote = 0usize;
    let (bold, ital, strike) = match d { Dialect::Telegram => (("<b>", "</b>"), ("<i>", "</i>"), ("<s>", "</s>")), Dialect::Slack => (("*", "*"), ("_", "_"), ("~", "~")), Dialect::Discord => (("**", "**"), ("*", "*"), ("~~", "~~")), Dialect::Plain => (("", ""), ("", ""), ("", "")) };
    let nl = |out: &mut String, quote: usize, d: Dialect| { out.push('\n'); if quote > 0 && d != Dialect::Telegram { out.push_str("> "); } };
    for ev in Parser::new_ext(&md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        if let Some(t) = table.as_mut() {
            match ev {
                Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => t.push(Vec::new()),
                Event::End(TagEnd::TableCell) => { if let Some(r) = t.last_mut() { r.push(std::mem::take(&mut cell).trim().to_string()); } }
                Event::Text(x) | Event::Code(x) => cell.push_str(&x),
                Event::End(TagEnd::Table) => {
                    let rows = table.take().unwrap_or_default();
                    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                    let width: Vec<usize> = (0..cols).map(|c| if tight { 0 } else { rows.iter().map(|r| r.get(c).map(|s| s.chars().count()).unwrap_or(0)).max().unwrap_or(0).min(PAD_MAX) }).collect();
                    let body: String = rows.iter().map(|r| (0..cols).map(|c| { let s = r.get(c).cloned().unwrap_or_default(); let pad = width[c].saturating_sub(s.chars().count()); format!("{s}{}", " ".repeat(pad)) }).collect::<Vec<_>>().join(" │ ").trim_end().to_string()).collect::<Vec<_>>().join("\n");
                    match d { Dialect::Telegram => out.push_str(&format!("<pre>{}</pre>\n\n", esc(d, &body))), Dialect::Plain => out.push_str(&format!("{body}\n\n")), _ => out.push_str(&format!("```\n{body}\n```\n\n")) }
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Table(_)) => table = Some(Vec::new()),
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => { if lists.is_empty() { out.push_str("\n\n"); } else { out.push('\n'); } }
            Event::Start(Tag::Heading { .. }) => out.push_str(bold.0),
            Event::End(TagEnd::Heading(_)) => { out.push_str(bold.1); out.push_str("\n\n"); }
            Event::Start(Tag::Strong) => out.push_str(bold.0),
            Event::End(TagEnd::Strong) => out.push_str(bold.1),
            Event::Start(Tag::Emphasis) => out.push_str(ital.0),
            Event::End(TagEnd::Emphasis) => out.push_str(ital.1),
            Event::Start(Tag::Strikethrough) => out.push_str(strike.0),
            Event::End(TagEnd::Strikethrough) => out.push_str(strike.1),
            Event::Start(Tag::BlockQuote(_)) => { quote += 1; if d == Dialect::Telegram { out.push_str("<blockquote>"); } else { out.push_str("> "); } }
            Event::End(TagEnd::BlockQuote(_)) => { quote = quote.saturating_sub(1); let t = out.trim_end().len(); out.truncate(t); if d == Dialect::Telegram { out.push_str("</blockquote>"); } out.push_str("\n\n"); }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                let lang = match kind { CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(), _ => String::new() };
                match d { Dialect::Telegram => out.push_str(&if lang.is_empty() { "<pre>".to_string() } else { format!("<pre><code class=\"language-{}\">", esc(d, &lang).replace('"', "")) }), Dialect::Plain => {}, _ => out.push_str(&format!("```{}\n", if d == Dialect::Discord { lang.as_str() } else { "" })) }
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                match d { Dialect::Telegram => { let t = out.trim_end_matches('\n').len(); out.truncate(t); out.push_str(if out.rfind("<pre><code").map(|i| out[i..].find("</code>").is_none()).unwrap_or(false) { "</code></pre>" } else { "</pre>" }); }, Dialect::Plain => {}, _ => { if !out.ends_with('\n') { out.push('\n'); } out.push_str("```"); } }
                out.push_str("\n\n");
            }
            Event::Start(Tag::List(start)) => { if !lists.is_empty() && !out.ends_with('\n') { out.push('\n'); } lists.push(start); }
            Event::End(TagEnd::List(_)) => { lists.pop(); if lists.is_empty() { out.push('\n'); } }
            Event::Start(Tag::Item) => {
                out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                match lists.last_mut() { Some(Some(n)) => { out.push_str(&format!("{n}. ")); *n += 1; } _ => out.push_str("• ") }
            }
            Event::End(TagEnd::Item) => { if !out.ends_with('\n') { out.push('\n'); } }
            Event::TaskListMarker(done) => out.push_str(if done { "☑ " } else { "☐ " }),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_string(); let web = url.starts_with("http://") || url.starts_with("https://");
                match d { Dialect::Telegram if web => out.push_str(&format!("<a href=\"{}\">", esc(d, &url).replace('"', "%22"))), Dialect::Slack if web => out.push_str(&format!("<{}|", url.replace(['<', '>', '|'], ""))), Dialect::Discord if web => out.push('['), _ => {} }
                link_url.push(if web { url } else { String::new() });
            }
            Event::End(TagEnd::Link) => {
                let url = link_url.pop().unwrap_or_default();
                if !url.is_empty() { match d { Dialect::Telegram => out.push_str("</a>"), Dialect::Slack => out.push('>'), Dialect::Discord => out.push_str(&format!("](<{url}>)")), Dialect::Plain => out.push_str(&format!(" ({url})")) } }
            }
            Event::Start(Tag::Image { dest_url, .. }) => { out.push_str("🖼 "); link_url.push(dest_url.to_string()); }
            Event::End(TagEnd::Image) => { let url = link_url.pop().unwrap_or_default(); if url.starts_with("http") { out.push_str(&format!(" ({})", esc(d, &url))); } }
            Event::Text(t) => out.push_str(&esc(d, &t)),
            Event::Code(t) => match d { Dialect::Telegram => out.push_str(&format!("<code>{}</code>", esc(d, &t))), Dialect::Plain => out.push_str(&t), _ => out.push_str(&format!("`{}`", esc(d, &t).replace('`', "'"))) },
            Event::Html(t) | Event::InlineHtml(t) => out.push_str(&esc(d, &t)),
            Event::SoftBreak | Event::HardBreak => { if in_code { out.push('\n'); } else { nl(&mut out, quote, d); } }
            Event::Rule => out.push_str("———\n\n"),
            _ => {}
        }
    }
    let mut out = out.trim().to_string();
    while out.contains("\n\n\n") { out = out.replace("\n\n\n", "\n\n"); }
    out
}

/// Markdown cut into pieces of at most `limit` characters: by blocks (blank lines), a fenced code block stays whole while
/// it fits; one that does not fit is cut by lines and every piece gets its own fence, so that each piece is valid Markdown
/// on its own.
pub fn split_md(md: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(200);
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new(); let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") { fence = !fence; }
        if line.trim().is_empty() && !fence { if !cur.trim().is_empty() { blocks.push(std::mem::take(&mut cur)); } cur.clear(); continue; }
        if !cur.is_empty() { cur.push('\n'); } cur.push_str(line);
    }
    if !cur.trim().is_empty() { blocks.push(cur); }
    let mut pieces: Vec<String> = Vec::new();
    for b in blocks {
        if b.chars().count() <= limit { pieces.push(b); continue; }
        let fenced = b.trim_start().starts_with("```");
        let (open, lines): (String, Vec<&str>) = if fenced { let mut l: Vec<&str> = b.lines().collect(); let open = l.remove(0).to_string(); if l.last().map(|x| x.trim_start().starts_with("```")).unwrap_or(false) { l.pop(); } (open, l) } else { (String::new(), b.lines().collect()) };
        let room = limit.saturating_sub(if fenced { open.chars().count() + 8 } else { 0 }).max(100);
        let mut part = String::new();
        let flush = |part: &mut String, pieces: &mut Vec<String>| { if part.is_empty() { return } let p = std::mem::take(part); pieces.push(if fenced { format!("{open}\n{p}\n```") } else { p }); };
        for line in lines {
            let mut line = line.to_string();
            while line.chars().count() > room { let head: String = line.chars().take(room).collect(); line = line.chars().skip(room).collect(); flush(&mut part, &mut pieces); pieces.push(if fenced { format!("{open}\n{head}\n```") } else { head }); }
            if part.chars().count() + line.chars().count() + 1 > room { flush(&mut part, &mut pieces); }
            if !part.is_empty() { part.push('\n'); } part.push_str(&line);
        }
        flush(&mut part, &mut pieces);
    }
    let mut out: Vec<String> = Vec::new();
    for p in pieces {
        match out.last_mut() { Some(last) if last.chars().count() + p.chars().count() + 2 <= limit => { last.push_str("\n\n"); last.push_str(&p); } _ => out.push(p) }
    }
    out
}

/// The first `n` characters of a text in plain words — the short result under a notice.
pub fn brief(md: &str, n: usize) -> String {
    let plain = render(Dialect::Plain, md);
    let one: String = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n { one } else { format!("{}…", one.chars().take(n).collect::<String>().trim_end()) }
}

/// The title of a report: its first heading, or its first line.
pub fn title_of(md: &str) -> String {
    let line = md.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let t = line.trim_start_matches('#').trim();
    let t: String = render(Dialect::Plain, t).chars().take(120).collect();
    if t.is_empty() { "Отчёт".into() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_closed_and_found() {
        let t = "ключ sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA и ghp_abcdefghijklmnopqrstuvwxyz0123456789, бот 7312345678:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw-x\nPASSWORD=hunter2hunter2\napi_key: \"abcdef123456\"\nобычный текст: token economy matters";
        let m = mask_secrets(t);
        assert!(!m.contains("sk-ant") && !m.contains("ghp_") && !m.contains("AAHdq") && !m.contains("hunter2") && !m.contains("abcdef123456"), "{m}");
        assert!(m.contains("PASSWORD=«скрыто»") && m.contains("api_key: \"«скрыто»"), "{m}");
        assert!(m.contains("token economy matters"), "ordinary words stay: {m}");
        assert!(has_secret("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----") && has_secret("AKIAIOSFODNN7EXAMPLE") && !has_secret("ordinary report about keys and tokens"));
        // a key cut short has no END line: it is a key all the same, and the mask takes it to the end of the text
        let cut = "вот ключ:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA7\nxyz";
        assert!(has_secret(cut)); assert_eq!(mask_secrets(cut), "вот ключ:\n«скрыто»");
        // counts and limits are not secrets
        let counts = "Total tokens: 152340, max_tokens=200000, max_token = 4096, token_budget: 1_000_000";
        assert_eq!(mask_secrets(counts), counts);
        assert_eq!(mask_secrets("db_password=Zx81kkQ0 token_value: abcdef99"), "db_password=«скрыто» token_value: «скрыто»");
        for n in [".env", ".env.local", "prod.env", "id_rsa", "id_ed25519.pub", "server.pem", "tls.key", "credentials.json", ".netrc", "dir/.npmrc", "mcp.json"] { assert!(secret_name(n), "{n}"); }
        for n in ["report.md", "chart.png", "data.csv", "environment.md", "keys-explained.pdf", "tokens.txt"] { assert!(!secret_name(n), "{n}"); }
    }

    #[test]
    fn markdown_for_every_service() {
        let md = "# Итоги <отчёта>\n\nТри **важных** вывода и _один_ вопрос, см. [источник](https://example.com/a?b=1&c=2).\n\n- первый `код`\n- второй\n  1. вложенный\n\n```rust\nfn main() { println!(\"<ok>\"); }\n```\n\n| имя | цена |\n|---|---|\n| A | 10 |\n| Bbb | 5 |\n\n> цитата @everyone\n";
        let tg = render(Dialect::Telegram, md);
        assert!(tg.starts_with("<b>Итоги &lt;отчёта&gt;</b>"), "{tg}");
        assert!(tg.contains("<b>важных</b>") && tg.contains("<i>один</i>") && tg.contains("<a href=\"https://example.com/a?b=1&amp;c=2\">источник</a>"), "{tg}");
        assert!(tg.contains("• первый <code>код</code>") && tg.contains("1. вложенный"), "{tg}");
        assert!(tg.contains("<pre><code class=\"language-rust\">fn main() { println!(\"&lt;ok&gt;\"); }</code></pre>"), "{tg}");
        assert!(tg.contains("<pre>имя │ цена\nA   │ 10\nBbb │ 5</pre>"), "{tg}");
        assert!(tg.contains("<blockquote>цитата @\u{200b}everyone</blockquote>"), "{tg}");
        // every tag that was opened is closed: Telegram refuses the whole message otherwise
        for tag in ["b", "i", "pre", "code", "a", "blockquote"] { assert_eq!(Regex::new(&format!("<{tag}[ >]")).unwrap().find_iter(&tg).count(), tg.matches(&format!("</{tag}>")).count(), "{tag}: {tg}"); }
        let sl = render(Dialect::Slack, md);
        assert!(sl.starts_with("*Итоги &lt;отчёта&gt;*") && sl.contains("*важных*") && sl.contains("_один_") && sl.contains("<https://example.com/a?b=1&c=2|источник>") && sl.contains("```\nfn main()") && sl.contains("> цитата"), "{sl}");
        let dc = render(Dialect::Discord, md);
        assert!(dc.contains("**важных**") && dc.contains("[источник](<https://example.com/a?b=1&c=2>)") && dc.contains("```rust\nfn main()") && dc.contains("@\u{200b}everyone") && !dc.contains("@everyone"), "{dc}");
        let pl = render(Dialect::Plain, md);
        assert!(pl.starts_with("Итоги <отчёта>") && pl.contains("источник (https://example.com/a?b=1&c=2)") && !pl.contains("**"), "{pl}");
        assert_eq!(brief("# Заголовок\n\nПервое предложение. Второе предложение длиннее первого.", 30), "Заголовок Первое предложение.…");
        assert_eq!(title_of("\n# Итоги **недели**\nтекст"), "Итоги недели");
        // one long cell does not pad every row of the table to its width
        let wide = format!("| a | b |\n|---|---|\n| x | {} |\n{}", "длинная ячейка ".repeat(20), "| y | z |\n".repeat(20));
        let padded = render(Dialect::Telegram, &wide); let tight = render_tight(Dialect::Telegram, &wide);
        assert!(padded.chars().count() < wide.chars().count() + 21 * (PAD_MAX + 8), "{}", padded.chars().count());
        assert!(tight.chars().count() <= wide.chars().count() + 20 && tight.contains("y │ z"), "{}", tight.chars().count());
        assert_eq!(title_of(""), "Отчёт");
    }

    #[test]
    fn long_text_is_cut_into_valid_pieces() {
        let para = "слово ".repeat(120);
        let code = format!("```sh\n{}\n```", (0..200).map(|i| format!("echo line number {i} of the long script")).collect::<Vec<_>>().join("\n"));
        let md = format!("# Заголовок\n\n{para}\n\n{para}\n\n{code}\n\nконец");
        let parts = split_md(&md, 1000);
        assert!(parts.len() > 5 && parts.iter().all(|p| p.chars().count() <= 1000), "{:?}", parts.iter().map(|p| p.chars().count()).collect::<Vec<_>>());
        // a cut code block stays a code block in every piece
        for p in &parts { assert_eq!(p.matches("```").count() % 2, 0, "an open fence in: {p}"); }
        assert!(parts.iter().filter(|p| p.contains("echo line number")).all(|p| p.contains("```sh")));
        let joined = parts.join("\n");
        assert!(joined.contains("echo line number 0 of") && joined.contains("echo line number 199 of") && joined.contains("конец"));
        assert_eq!(split_md("коротко", 1000), vec!["коротко"]);
        // every piece renders into balanced Telegram tags
        for p in &parts { let h = render(Dialect::Telegram, p); assert_eq!(h.matches("<pre").count(), h.matches("</pre>").count(), "{h}"); }
    }

    #[test]
    fn heads_and_links_are_safe() {
        assert_eq!(head(Dialect::Telegram, "✅ gamma · <b>x</b>", " — ход завершён"), "<b>✅ gamma · &lt;b&gt;x&lt;/b&gt;</b> — ход завершён");
        assert_eq!(head(Dialect::Slack, "a*b <!channel>", ""), "*a∗b &lt;\u{200b}!channel&gt;*");
        assert_eq!(link(Dialect::Telegram, "Открыть", "https://h/agents/?open=p/s&x=\"1\""), "<a href=\"https://h/agents/?open=p/s&amp;x=%221%22\">Открыть</a>");
        assert_eq!(link(Dialect::Discord, "Открыть", "https://h/a"), "[Открыть](<https://h/a>)");
    }
}
