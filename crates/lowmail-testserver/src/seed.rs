//! Deterministic demo data: folders and ~60 realistic messages (pt-BR / en)
//! dated relative to the server start time.

use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, Utc};

use crate::message::MessageData;
use crate::sasl::encode_base64;
use crate::store::Store;

const SEEN: &[&str] = &["\\Seen"];
const UNREAD: &[&str] = &[];
const FLAGGED: &[&str] = &["\\Seen", "\\Flagged"];
const UNREAD_FLAGGED: &[&str] = &["\\Flagged"];
const ANSWERED: &[&str] = &["\\Seen", "\\Answered"];
const FORWARDED: &[&str] = &["\\Seen", "$Forwarded"];

/// Seeds the first user with the full demo mailbox and the other users with
/// a few messages each.
pub(crate) fn seed(store: &Store, users: &[(String, String)], now: DateTime<Utc>) {
    let offset = FixedOffset::west_opt(3 * 3600).expect("valid offset");
    let now = now.with_timezone(&offset);
    for (i, (email, _)) in users.iter().enumerate() {
        let generator = Gen {
            now,
            me: email.to_ascii_lowercase(),
        };
        let mails = if i == 0 {
            let mut inner = store.lock();
            for folder in ["Projetos", "Projetos/Relat&APM-rios", "Projetos/Clientes"] {
                let _ = inner.create_mailbox(&generator.me, folder, None);
            }
            generator.demo()
        } else {
            generator.small()
        };
        let mut sorted = mails;
        sorted.sort_by_key(|m| m.date);
        let mut inner = store.lock();
        for mail in sorted {
            if let Some(mbox) = inner.mailbox_mut(&generator.me, mail.mailbox) {
                mbox.append(
                    Arc::new(MessageData::new(mail.raw)),
                    mail.flags.iter().map(|f| f.to_string()).collect(),
                    mail.date,
                );
                if let Some(m) = mbox.messages.last_mut() {
                    m.recent = false;
                }
            }
        }
    }
}

struct Mail {
    mailbox: &'static str,
    date: DateTime<FixedOffset>,
    flags: &'static [&'static str],
    raw: Vec<u8>,
}

// ---------------------------------------------------------------------------
// MIME building helpers

/// Converts `\n` line endings to CRLF.
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn q_byte(out: &mut String, b: u8) {
    out.push_str(&format!("={b:02X}"));
}

/// RFC 2047 Q-encodes `text` into one or more encoded words (each <= 75
/// chars), separated by folding whitespace.
fn encode_words(text: &str) -> String {
    const PREFIX: &str = "=?UTF-8?Q?";
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let mut enc = String::new();
        let mut buf = [0u8; 4];
        for &b in ch.encode_utf8(&mut buf).as_bytes() {
            match b {
                b' ' => enc.push('_'),
                b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'.'
                | b','
                | b'-'
                | b'!'
                | b'*'
                | b'+'
                | b'/' => enc.push(b as char),
                _ => q_byte(&mut enc, b),
            }
        }
        if PREFIX.len() + current.len() + enc.len() + 2 > 75 {
            words.push(format!("{PREFIX}{current}?="));
            current.clear();
        }
        current.push_str(&enc);
    }
    if !current.is_empty() {
        words.push(format!("{PREFIX}{current}?="));
    }
    words.join("\r\n ")
}

/// Header value: ASCII is kept (folded at spaces), anything else encoded.
fn header(name: &str, value: &str) -> String {
    if !value.is_ascii() {
        return format!("{name}: {}\r\n", encode_words(value));
    }
    let mut out = format!("{name}:");
    let mut line_len = out.len();
    for word in value.split(' ') {
        if line_len + 1 + word.len() > 76 && line_len > name.len() + 1 {
            out.push_str("\r\n");
            line_len = 0;
        }
        out.push(' ');
        out.push_str(word);
        line_len += 1 + word.len();
    }
    out.push_str("\r\n");
    out
}

/// `Name <email>` with the name quoted or encoded as needed.
fn addr(name: &str, email: &str) -> String {
    if name.is_empty() {
        email.to_owned()
    } else if !name.is_ascii() {
        format!("{} <{email}>", encode_words(name))
    } else if name.chars().any(|c| ",.;:@<>()[]\"".contains(c)) {
        format!("\"{}\" <{email}>", name.replace('"', "\\\""))
    } else {
        format!("{name} <{email}>")
    }
}

/// Quoted-printable encoding of UTF-8 text (input uses `\n`).
fn qp(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        let bytes = line.as_bytes();
        let mut len = 0;
        for (j, &b) in bytes.iter().enumerate() {
            let last = j + 1 == bytes.len();
            let mut enc = String::new();
            if b == b'='
                || b >= 0x7f
                || (b < 0x20 && b != b'\t')
                || (last && (b == b' ' || b == b'\t'))
            {
                q_byte(&mut enc, b);
            } else {
                enc.push(b as char);
            }
            if len + enc.len() > 75 {
                out.push_str("=\r\n");
                len = 0;
            }
            out.push_str(&enc);
            len += enc.len();
        }
    }
    out
}

fn base64_lines(data: &[u8]) -> String {
    let encoded = encode_base64(data);
    encoded
        .as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn text_part(body: &str) -> String {
    format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}",
        crlf(body)
    )
}

fn qp_text_part(body: &str) -> String {
    format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{}",
        qp(body)
    )
}

fn html_part(html: &str) -> String {
    format!(
        "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{}",
        qp(html)
    )
}

fn multipart(subtype: &str, boundary: &str, parts: &[String]) -> String {
    let mut out = format!(
        "Content-Type: multipart/{subtype}; boundary=\"{boundary}\"\r\n\r\nThis is a multi-part message in MIME format.\r\n"
    );
    for part in parts {
        out.push_str(&format!("\r\n--{boundary}\r\n{part}"));
    }
    out.push_str(&format!("\r\n--{boundary}--\r\n"));
    out
}

fn attachment(content_type: &str, filename: &str, data: &[u8]) -> String {
    format!(
        "Content-Type: {content_type}; name=\"{filename}\"\r\nContent-Disposition: attachment; filename=\"{filename}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        base64_lines(data)
    )
}

fn inline_image(cid: &str, filename: &str, data: &[u8]) -> String {
    format!(
        "Content-Type: image/png; name=\"{filename}\"\r\nContent-Disposition: inline; filename=\"{filename}\"\r\nContent-ID: <{cid}>\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        base64_lines(data)
    )
}

// ---------------------------------------------------------------------------
// Generated binary attachments

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A valid RGB PNG (stored/uncompressed deflate).
fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn gradient_png() -> Vec<u8> {
    png(48, 32, |x, y| [(x * 5) as u8, (y * 7) as u8, 180])
}

fn logo_png() -> Vec<u8> {
    png(32, 32, |x, y| {
        let inside = (8..24).contains(&x) && (8..24).contains(&y);
        if inside {
            [46, 125, 50]
        } else {
            [255, 255, 255]
        }
    })
}

/// A minimal single-page PDF with correct xref offsets.
fn pdf(title: &str) -> Vec<u8> {
    let content = format!(
        "BT /F1 20 Tf 72 760 Td ({title}) Tj ET\nBT /F1 12 Tf 72 730 Td (Gerado pelo LowMail test server) Tj ET\n"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{obj}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for off in offsets {
        out.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

/// Deterministic pseudo-random bytes behind a ZIP signature.
fn zip_blob(len: usize) -> Vec<u8> {
    let mut out = b"PK\x03\x04\x14\x00\x00\x00\x08\x00".to_vec();
    let mut state = 0x2545_f491_u32;
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        out.push(state as u8);
    }
    out
}

fn latin1(text: &str) -> Vec<u8> {
    text.chars().map(|c| c as u32 as u8).collect()
}

// ---------------------------------------------------------------------------
// Message generation

struct Gen {
    now: DateTime<FixedOffset>,
    me: String,
}

/// `(days ago, minutes ago, (name, email), subject, text/link, html, flags)`.
type RichSpec = (
    i64,
    i64,
    (&'static str, &'static str),
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

/// A short single-part text message description.
struct Simple {
    days: i64,
    mins: i64,
    from: (&'static str, &'static str),
    subject: &'static str,
    body: &'static str,
    flags: &'static [&'static str],
}

impl Gen {
    fn at(&self, days: i64, mins: i64) -> DateTime<FixedOffset> {
        self.now - Duration::days(days) - Duration::minutes(mins)
    }

    fn me_name(&self) -> String {
        let local = self.me.split('@').next().unwrap_or("user");
        let mut chars = local.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().chain(chars).collect(),
            None => "User".into(),
        }
    }

    fn me_addr(&self) -> String {
        addr(&self.me_name(), &self.me)
    }

    /// Standard header block (ends with `MIME-Version`).
    fn head(
        &self,
        date: DateTime<FixedOffset>,
        from: &str,
        to: &str,
        subject: &str,
        id: &str,
        extra: &str,
    ) -> String {
        format!(
            "Date: {}\r\nFrom: {from}\r\nTo: {to}\r\n{}Message-ID: <{id}>\r\n{extra}MIME-Version: 1.0\r\n",
            date.to_rfc2822(),
            header("Subject", subject),
        )
    }

    fn mail(
        &self,
        mailbox: &'static str,
        date: DateTime<FixedOffset>,
        flags: &'static [&'static str],
        raw: String,
    ) -> Mail {
        Mail {
            mailbox,
            date,
            flags,
            raw: raw.into_bytes(),
        }
    }

    fn simple(&self, mailbox: &'static str, s: &Simple, id: &str) -> Mail {
        let date = self.at(s.days, s.mins);
        let head = self.head(
            date,
            &addr(s.from.0, s.from.1),
            &self.me_addr(),
            s.subject,
            id,
            "",
        );
        let part = if s.body.is_ascii() {
            text_part(s.body)
        } else {
            qp_text_part(s.body)
        };
        self.mail(mailbox, date, s.flags, head + &part)
    }

    /// Demo mailbox for the main user.
    fn demo(&self) -> Vec<Mail> {
        let mut mails = Vec::new();
        for (i, s) in SIMPLE_INBOX.iter().enumerate() {
            mails.push(self.simple("INBOX", s, &format!("simple{i}.seed@lowmail.test")));
        }
        mails.extend(self.alternatives());
        mails.extend(self.newsletters());
        mails.extend(self.attachments());
        mails.extend(self.thread());
        mails.extend(self.encodings());
        mails.extend(self.special_headers());
        mails.extend(self.other_folders());
        mails
    }

    /// A few messages for secondary users.
    fn small(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let alice = addr("Alice", "alice@lowmail.test");
        let mut mails = Vec::new();
        let d = self.at(1, 30);
        mails.push(self.mail(
            "INBOX",
            d,
            UNREAD,
            self.head(d, &alice, &me, "Bem-vindo ao LowMail", &format!("welcome.{}@lowmail.test", self.me), "")
                + &qp_text_part("Olá!\n\nSua conta de testes está pronta. Responda este e-mail se precisar de algo.\n\nAbraços,\nAlice\n"),
        ));
        let d = self.at(3, 200);
        mails.push(self.mail(
            "INBOX",
            d,
            SEEN,
            self.head(
                d,
                &addr("Carlos Pereira", "carlos@acme.com.br"),
                &me,
                "Reunião amanhã às 10h",
                &format!("meeting.{}@acme.com.br", self.me),
                "",
            ) + &qp_text_part(
                "Oi,\n\nConfirmando nossa reunião amanhã às 10h na sala 3.\n\nCarlos\n",
            ),
        ));
        let d = self.at(8, 15);
        mails.push(self.mail(
            "INBOX",
            d,
            SEEN,
            self.head(d, &addr("Notion", "notify@notion.example"), &me, "Your weekly digest", &format!("digest.{}@notion.example", self.me), "")
                + &multipart("alternative", "bob-alt-1", &[
                    text_part("Here is what happened in your workspace this week.\n- 3 pages edited\n- 2 comments\n"),
                    html_part("<html><body><h2>Your weekly digest</h2><ul><li>3 pages edited</li><li>2 comments</li></ul></body></html>"),
                ]),
        ));
        mails
    }

    fn alternatives(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let specs: [RichSpec; 6] = [
            (
                2,
                95,
                ("Carla Mendes", "carla.mendes@acme.com.br"),
                "Resumo semanal da equipe",
                "Oi pessoal,\n\nResumo da semana:\n- Sprint 14 concluída com 92% dos pontos\n- Deploy da versão 0.9 na sexta\n- Retrospectiva na segunda às 15h\n\nBom fim de semana!\nCarla\n",
                "<html><body><p>Oi pessoal,</p><p><b>Resumo da semana:</b></p><ul><li>Sprint 14 concluída com <b>92%</b> dos pontos</li><li>Deploy da versão 0.9 na sexta</li><li>Retrospectiva na segunda às 15h</li></ul><p>Bom fim de semana!<br>Carla</p></body></html>",
                SEEN,
            ),
            (
                6,
                40,
                ("Notion", "notify@notion.example"),
                "Your weekly digest",
                "Here is what happened in LowMail HQ this week:\n\n- Roadmap Q4 was edited by Mariana\n- 4 new comments on \"Sync engine\"\n",
                "<html><body><h2 style=\"font-family:sans-serif\">Your weekly digest</h2><p>Here is what happened in <b>LowMail HQ</b> this week:</p><ul><li>Roadmap Q4 was edited by Mariana</li><li>4 new comments on <i>Sync engine</i></li></ul></body></html>",
                SEEN,
            ),
            (
                0,
                75,
                ("Slack", "notification@slack.example"),
                "New messages in #lowmail-dev",
                "You have 5 unread messages in #lowmail-dev.\n\nPedro: the IMAP IDLE fix is merged\nAna: great, deploying to staging\n",
                "<html><body><p>You have <b>5 unread messages</b> in <a href=\"https://slack.example/lowmail-dev\">#lowmail-dev</a>.</p><blockquote><b>Pedro:</b> the IMAP IDLE fix is merged<br><b>Ana:</b> great, deploying to staging</blockquote></body></html>",
                UNREAD,
            ),
            (
                12,
                300,
                ("Banco Roxo", "fatura@bancoroxo.example"),
                "Sua fatura fechou: R$ 1.284,90",
                "Olá, Alice!\n\nSua fatura de outubro fechou no valor de R$ 1.284,90.\nVencimento: dia 10.\n\nBanco Roxo\n",
                "<html><body style=\"font-family:Arial\"><h1 style=\"color:#820ad1\">Sua fatura fechou</h1><p>Valor: <b>R$ 1.284,90</b></p><p>Vencimento: dia 10</p><p><a href=\"https://bancoroxo.example/fatura\">Ver fatura</a></p></body></html>",
                SEEN,
            ),
            (
                17,
                120,
                ("Trello", "do-not-reply@trello.example"),
                "3 cards due this week",
                "Cards due soon:\n- Ajustar layout da caixa de entrada (Wed)\n- Testes E2E do SMTP (Thu)\n- Revisar traduções (Fri)\n",
                "<html><body><h3>Cards due soon</h3><ol><li>Ajustar layout da caixa de entrada <i>(Wed)</i></li><li>Testes E2E do SMTP <i>(Thu)</i></li><li>Revisar traduções <i>(Fri)</i></li></ol></body></html>",
                SEEN,
            ),
            (
                33,
                60,
                ("Caixa de Arquivos", "share@arquivos.example"),
                "Arquivos compartilhados com você",
                "Mariana Costa compartilhou a pasta \"Design LowMail\" com você.\nAcesse: https://arquivos.example/s/abc123\n",
                "<html><body><p><b>Mariana Costa</b> compartilhou a pasta <b>Design LowMail</b> com você.</p><p><a href=\"https://arquivos.example/s/abc123\">Abrir pasta</a></p></body></html>",
                SEEN,
            ),
        ];
        specs
            .iter()
            .enumerate()
            .map(|(i, (days, mins, from, subject, text, html, flags))| {
                let date = self.at(*days, *mins);
                let head = self.head(
                    date,
                    &addr(from.0, from.1),
                    &me,
                    subject,
                    &format!("alt{i}.seed@lowmail.test"),
                    "",
                );
                let body = multipart(
                    "alternative",
                    &format!("alt-boundary-{i}"),
                    &[qp_text_part(text), html_part(html)],
                );
                self.mail("INBOX", date, flags, head + &body)
            })
            .collect()
    }

    fn newsletters(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let style = "<style>\n  body { margin:0; background:#f4f4f7; font-family:Helvetica,Arial,sans-serif; }\n  .wrap { max-width:600px; margin:0 auto; background:#ffffff; }\n  h1 { color:#1a1a2e; font-size:24px; }\n  .cta { background:#e94560; color:#fff; padding:12px 20px; border-radius:4px; text-decoration:none; }\n  .muted { color:#888; font-size:12px; }\n</style>";
        let items: [RichSpec; 4] = [
            (
                1,
                400,
                ("The Daily Byte", "newsletter@dailybyte.example"),
                "Rust 2.0 rumors, AI chips and more",
                "https://dailybyte.example/unsubscribe?u=alice",
                "<h1>The Daily Byte</h1><img src=\"https://images.dailybyte.example/hero-rust.jpg\" width=\"600\" alt=\"Rust\"><h2>Rust 2.0 rumors</h2><p>Language team members discussed what a future edition could bring.</p><img src=\"https://images.dailybyte.example/chips.png\" width=\"300\" alt=\"Chips\"><h2>AI chips everywhere</h2><p>New accelerators land in laptops this quarter.</p><p><a class=\"cta\" href=\"https://dailybyte.example/read\">Read more</a></p>",
                UNREAD,
            ),
            (
                9,
                500,
                ("Café com Código", "ola@cafecomcodigo.example"),
                "☕ Café com Código #87: async em Rust sem mistérios",
                "https://cafecomcodigo.example/descadastrar?e=alice",
                "<h1>Café com Código #87</h1><img src=\"https://cdn.cafecomcodigo.example/capa-87.png\" width=\"600\" alt=\"Capa\"><p>Nesta edição: <b>async em Rust sem mistérios</b>, dicas de Tokio e uma entrevista com quem mantém um cliente de e-mail open source.</p><p><a class=\"cta\" href=\"https://cafecomcodigo.example/87\">Ler a edição completa</a></p>",
                SEEN,
            ),
            (
                21,
                610,
                ("Loja Exemplo", "ofertas@lojaexemplo.example"),
                "Ofertas da semana – até 60% off",
                "https://lojaexemplo.example/unsub/alice",
                "<h1>Ofertas da semana</h1><table width=\"100%\"><tr><td><img src=\"https://img.lojaexemplo.example/fone.jpg\" width=\"180\" alt=\"Fone\"><p>Fone Bluetooth<br><b>R$ 129,90</b></p></td><td><img src=\"https://img.lojaexemplo.example/teclado.jpg\" width=\"180\" alt=\"Teclado\"><p>Teclado mecânico<br><b>R$ 349,00</b></p></td></tr></table><p><a class=\"cta\" href=\"https://lojaexemplo.example/ofertas\">Ver todas as ofertas</a></p>",
                UNREAD,
            ),
            (
                45,
                180,
                ("Product Hunt Daily", "hello@producthunt.example"),
                "Top 5 products today",
                "https://producthunt.example/unsubscribe",
                "<h1>Top 5 products today</h1><ol><li><img src=\"https://ph-files.example/1.png\" width=\"40\"> LowMail - a lean email client</li><li><img src=\"https://ph-files.example/2.png\" width=\"40\"> NoteFlow</li><li>PixelPal</li><li>QueueCat</li><li>Shipyard</li></ol>",
                SEEN,
            ),
        ];
        items
            .iter()
            .enumerate()
            .map(|(i, (days, mins, from, subject, unsub, content, flags))| {
                let date = self.at(*days, *mins);
                let extra = format!(
                    "List-Unsubscribe: <{unsub}>, <mailto:unsubscribe@{}>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\nList-Id: <news.{}>\r\nPrecedence: bulk\r\n",
                    from.1.split('@').nth(1).unwrap_or("example.com"),
                    from.1.split('@').nth(1).unwrap_or("example.com"),
                );
                let head = self.head(date, &addr(from.0, from.1), &me, subject, &format!("news{i}.seed@lowmail.test"), &extra);
                let html = format!(
                    "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n{style}\n</head>\n<body>\n<div class=\"wrap\">\n{content}\n<p class=\"muted\">Você recebeu este e-mail porque se inscreveu. <a href=\"{unsub}\">Descadastrar</a></p>\n</div>\n<img src=\"https://track.example/open.gif?n={i}\" width=\"1\" height=\"1\" alt=\"\">\n</body>\n</html>\n"
                );
                let body = if i % 2 == 0 {
                    html_part(&html)
                } else {
                    multipart(
                        "alternative",
                        &format!("news-boundary-{i}"),
                        &[qp_text_part("Veja esta edição no navegador: https://cafecomcodigo.example/87\n"), html_part(&html)],
                    )
                };
                self.mail("INBOX", date, flags, head + &body)
            })
            .collect()
    }

    fn attachments(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let mut mails = Vec::new();

        let date = self.at(4, 130);
        let head = self.head(
            date,
            &addr("Ana Ribeiro", "ana.ribeiro@juridico.example"),
            &me,
            "Contrato revisado em anexo",
            "contrato.seed@juridico.example",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-contrato",
            &[
                qp_text_part(
                    "Olá Alice,\n\nSegue em anexo o contrato revisado com as alterações da cláusula 7.\nPor favor, confirme até quinta-feira.\n\nAtenciosamente,\nAna Ribeiro\nDepartamento Jurídico\n",
                ),
                attachment(
                    "application/pdf",
                    "contrato-revisado.pdf",
                    &pdf("Contrato LowMail - revisao 3"),
                ),
            ],
        );
        mails.push(self.mail("INBOX", date, FLAGGED, head + &body));

        let date = self.at(7, 20);
        let head = self.head(
            date,
            &addr("Lucas Martins", "lucas.martins@acme.com.br"),
            &me,
            "Screenshot do bug na lista de mensagens",
            "screenshot.seed@acme.com.br",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-screenshot",
            &[
                qp_text_part(
                    "Oi Alice,\n\nO bug aparece quando a lista tem mais de 500 mensagens. Print em anexo.\n\nLucas\n",
                ),
                attachment("image/png", "bug-lista.png", &gradient_png()),
            ],
        );
        mails.push(self.mail("INBOX", date, SEEN, head + &body));

        let date = self.at(11, 45);
        let log = "2025-10-01T10:00:01Z INFO  imap: connected to imap.lowmail.test:993\n2025-10-01T10:00:02Z INFO  imap: selected INBOX (1234 messages)\n2025-10-01T10:00:05Z WARN  sync: slow FETCH (2.3s)\n2025-10-01T10:00:09Z ERROR smtp: 451 4.7.1 try again later\n";
        let head = self.head(
            date,
            &addr("Monitoramento", "alertas@infra.example"),
            &me,
            "Logs do servidor - incidente 4521",
            "logs.seed@infra.example",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-logs",
            &[
                text_part("Segue o log do incidente 4521 para análise.\n"),
                attachment(
                    "text/plain; charset=utf-8",
                    "servidor.log.txt",
                    crlf(log).as_bytes(),
                ),
            ],
        );
        mails.push(self.mail("INBOX", date, SEEN, head + &body));

        let date = self.at(15, 300);
        let csv = "data;categoria;descricao;valor\n2025-09-02;Transporte;Uber aeroporto;68,40\n2025-09-05;Alimentação;Almoço com cliente;142,00\n2025-09-12;Software;Licença IDE;299,00\n2025-09-20;Hospedagem;Hotel São Paulo;520,00\n";
        let head = self.head(
            date,
            &addr("Financeiro", "financeiro@acme.com.br"),
            &me,
            "Planilha de despesas – setembro",
            "despesas.seed@acme.com.br",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-csv",
            &[
                qp_text_part(
                    "Olá,\n\nAnexamos a planilha de despesas de setembro para conferência.\n\nEquipe Financeira\n",
                ),
                attachment(
                    "text/csv; charset=utf-8",
                    "despesas-setembro.csv",
                    crlf(csv).as_bytes(),
                ),
            ],
        );
        mails.push(self.mail("INBOX", date, ANSWERED, head + &body));

        let date = self.at(26, 90);
        let head = self.head(
            date,
            &addr("Pedro Almeida", "pedro.almeida@acme.com.br"),
            &me,
            "Backup do projeto (zip)",
            "backup.seed@acme.com.br",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-zip",
            &[
                text_part(
                    "Alice, segue o backup compactado do projeto antes da migração.\n\nPedro\n",
                ),
                attachment("application/zip", "lowmail-backup.zip", &zip_blob(4096)),
            ],
        );
        mails.push(self.mail("INBOX", date, SEEN, head + &body));

        // HTML with an inline cid: image (multipart/related inside alternative).
        let date = self.at(2, 30);
        let head = self.head(
            date,
            &addr("Equipe LowMail", "eventos@lowmail.test"),
            &me,
            "Convite: lançamento do LowMail 1.0",
            "convite.seed@lowmail.test",
            "",
        );
        let related = multipart(
            "related",
            "related-convite",
            &[
                html_part(
                    "<html><body style=\"font-family:sans-serif\"><p><img src=\"cid:logo@lowmail.test\" alt=\"LowMail\" width=\"64\" height=\"64\"></p><h1>Lançamento do LowMail 1.0</h1><p>Você está convidada para o lançamento no dia <b>15 de novembro</b>, às 19h.</p><p>Confirme sua presença respondendo este e-mail.</p></body></html>",
                ),
                inline_image("logo@lowmail.test", "logo.png", &logo_png()),
            ],
        );
        let body = multipart(
            "alternative",
            "alt-convite",
            &[
                qp_text_part(
                    "Lançamento do LowMail 1.0\n\nVocê está convidada para o lançamento no dia 15 de novembro, às 19h.\n",
                ),
                related,
            ],
        );
        mails.push(self.mail("INBOX", date, UNREAD_FLAGGED, head + &body));
        mails
    }

    fn thread(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let carlos = addr("Carlos Pereira", "carlos.pereira@acme.com.br");
        let mariana = addr("Mariana Costa", "mariana.costa@acme.com.br");
        let ids = [
            "migracao-1.seed@acme.com.br",
            "migracao-2.seed@acme.com.br",
            "migracao-3.seed@acme.com.br",
            "migracao-4.seed@acme.com.br",
        ];
        let steps: [(i64, i64, &str, &str, &'static [&'static str]); 4] = [
            (
                5,
                600,
                &carlos,
                "Pessoal,\n\nVamos migrar o servidor de e-mail no próximo sábado, das 8h às 12h.\nDurante a janela o IMAP ficará indisponível.\n\nAlguma objeção?\n\nCarlos\n",
                ANSWERED,
            ),
            (
                5,
                420,
                &mariana,
                "Sem objeções. Só precisamos avisar o time de suporte.\n\nMariana\n\nEm sex., Carlos Pereira escreveu:\n> Vamos migrar o servidor de e-mail no próximo sábado, das 8h às 12h.\n",
                SEEN,
            ),
            (
                4,
                900,
                &carlos,
                "Avisado. Também vou deixar um aviso no status page.\n\n> Sem objeções. Só precisamos avisar o time de suporte.\n",
                SEEN,
            ),
            (
                3,
                100,
                &mariana,
                "Perfeito! Migração concluída com sucesso, todos os clientes reconectaram.\n\n> Avisado. Também vou deixar um aviso no status page.\n",
                UNREAD,
            ),
        ];
        let mut mails = Vec::new();
        for (i, (days, mins, from, body, flags)) in steps.iter().enumerate() {
            let date = self.at(*days, *mins);
            let subject = if i == 0 {
                "Migração do servidor de e-mail".to_owned()
            } else {
                "Re: Migração do servidor de e-mail".to_owned()
            };
            let mut extra = String::new();
            if i > 0 {
                let refs: Vec<String> = ids[..i].iter().map(|id| format!("<{id}>")).collect();
                extra.push_str(&format!("In-Reply-To: <{}>\r\n", ids[i - 1]));
                extra.push_str(&format!("References: {}\r\n", refs.join("\r\n ")));
            }
            let to = format!("{me}, {}", if i % 2 == 0 { &mariana } else { &carlos });
            let head = self.head(date, from, &to, &subject, ids[i], &extra);
            mails.push(self.mail("INBOX", date, flags, head + &qp_text_part(body)));
        }
        // Alice's own reply lives in Sent.
        let date = self.at(5, 500);
        let extra = format!("In-Reply-To: <{}>\r\nReferences: <{}>\r\n", ids[0], ids[0]);
        let head = self.head(
            date,
            &me,
            &format!("{carlos}, {mariana}"),
            "Re: Migração do servidor de e-mail",
            "migracao-alice.seed@lowmail.test",
            &extra,
        );
        mails.push(self.mail(
            "Sent",
            date,
            SEEN,
            head + &qp_text_part(
                "Por mim tudo certo. Posso ajudar com os testes depois da janela.\n\nAlice\n",
            ),
        ));
        mails
    }

    fn encodings(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let mut mails = Vec::new();

        // UTF-8 B-encoded subject.
        let date = self.at(10, 200);
        let subject = format!(
            "=?UTF-8?B?{}?=",
            encode_base64("Relatório mensal de vendas – março".as_bytes())
        );
        let raw = format!(
            "Date: {}\r\nFrom: {}\r\nTo: {me}\r\nSubject: {subject}\r\nMessage-ID: <vendas.seed@acme.com.br>\r\nMIME-Version: 1.0\r\n{}",
            date.to_rfc2822(),
            addr("Patrícia Lopes", "patricia.lopes@acme.com.br"),
            qp_text_part(
                "Olá Alice,\n\nO relatório de vendas de março mostra crescimento de 18% em relação a fevereiro.\nDetalhes na reunião de quinta.\n\nPatrícia\n"
            ),
        );
        mails.push(self.mail("INBOX", date, SEEN, raw));

        // ISO-8859-1 Q-encoded subject and sender name.
        let date = self.at(13, 50);
        let raw = format!(
            "Date: {}\r\nFrom: =?ISO-8859-1?Q?Loja_A=E7=E3o?= <contato@lojaacao.example>\r\nTo: {me}\r\nSubject: =?ISO-8859-1?Q?Promo=E7=E3o_de_ver=E3o:_frete_gr=E1tis?=\r\nMessage-ID: <promo.seed@lojaacao.example>\r\nMIME-Version: 1.0\r\n{}",
            date.to_rfc2822(),
            text_part("Aproveite a promocao de verao com frete gratis em todo o site.\n"),
        );
        mails.push(self.mail("INBOX", date, UNREAD, raw));

        // UTF-8 Q-encoded display name.
        let date = self.at(16, 15);
        let raw = format!(
            "Date: {}\r\nFrom: =?UTF-8?Q?Jo=C3=A3o_Gon=C3=A7alves?= <joao.goncalves@consultoria.example>\r\nTo: {me}\r\nSubject: Proposta comercial\r\nMessage-ID: <proposta.seed@consultoria.example>\r\nMIME-Version: 1.0\r\n{}",
            date.to_rfc2822(),
            qp_text_part(
                "Prezada Alice,\n\nConforme conversamos, envio a proposta comercial para a consultoria de migração.\nValor total: R$ 48.000,00 em 3 parcelas.\n\nAtenciosamente,\nJoão Gonçalves\n"
            ),
        );
        mails.push(self.mail("INBOX", date, FLAGGED, raw));

        // ISO-8859-1 B-encoded name and subject.
        let date = self.at(19, 333);
        let raw = format!(
            "Date: {}\r\nFrom: =?ISO-8859-1?B?{}?= <jose.antonio@cooperativa.example>\r\nTo: {me}\r\nSubject: =?ISO-8859-1?B?{}?=\r\nMessage-ID: <informacoes.seed@cooperativa.example>\r\nMIME-Version: 1.0\r\n{}",
            date.to_rfc2822(),
            encode_base64(&latin1("José Antônio")),
            encode_base64(&latin1("Informações importantes sobre a assembléia")),
            text_part("Prezados cooperados, a assembleia sera no dia 20, as 18h.\n"),
        );
        mails.push(self.mail("INBOX", date, SEEN, raw));

        // ISO-8859-1 body (8bit).
        let date = self.at(23, 250);
        let mut raw = format!(
            "Date: {}\r\nFrom: {}\r\nTo: {me}\r\nSubject: =?ISO-8859-1?Q?Ata_da_reuni=E3o_de_condom=EDnio?=\r\nMessage-ID: <ata.seed@condominio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=ISO-8859-1\r\nContent-Transfer-Encoding: 8bit\r\n\r\n",
            date.to_rfc2822(),
            addr("Síndico", "sindico@condominio.example"),
        )
        .into_bytes();
        raw.extend_from_slice(&latin1(&crlf(
            "Ata da reunião ordinária\n\nPauta:\n1. Manutenção do elevador - aprovada\n2. Pintura da fachada - orçamento em análise\n3. Horário da piscina: das 8h às 22h\n\nAção: síndico enviará cotações até o próximo mês.\n",
        )));
        mails.push(Mail {
            mailbox: "INBOX",
            date,
            flags: SEEN,
            raw,
        });
        mails
    }

    fn special_headers(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let mut mails = Vec::new();

        // Very long subject (folded).
        let date = self.at(8, 240);
        let subject = "Atualização importante sobre a política de privacidade, termos de uso, retenção de dados e as novas integrações com provedores de identidade que entram em vigor no próximo trimestre";
        let head = self.head(
            date,
            &addr("Compliance", "compliance@acme.com.br"),
            &me,
            subject,
            "longsubject.seed@acme.com.br",
            "",
        );
        mails.push(self.mail("INBOX", date, SEEN, head + &qp_text_part("Olá,\n\nLeia com atenção as mudanças descritas no assunto deste e-mail.\n\nEquipe de Compliance\n")));

        // Reply-To differs from From.
        let date = self.at(1, 200);
        let extra = format!(
            "Reply-To: {}\r\n",
            addr("Fernanda Souza", "fernanda.souza@gmail.example")
        );
        let head = self.head(
            date,
            &addr("Site LowMail", "no-reply@lowmail.test"),
            &me,
            "Novo contato pelo formulário do site",
            "contato.seed@lowmail.test",
            &extra,
        );
        mails.push(self.mail("INBOX", date, UNREAD, head + &qp_text_part("Nome: Fernanda Souza\nE-mail: fernanda.souza@gmail.example\nMensagem: Gostaria de saber se o LowMail funciona com Exchange.\n\n(responda este e-mail para falar diretamente com a pessoa)\n")));

        // Mailing list message.
        let date = self.at(3, 700);
        let extra = "List-Id: Rust Brasil <rust-br.lists.example>\r\nList-Unsubscribe: <mailto:rust-br-leave@lists.example>, <https://lists.example/rust-br/unsubscribe>\r\nList-Post: <mailto:rust-br@lists.example>\r\nPrecedence: list\r\n";
        let head = self.head(
            date,
            &addr("Bruno Teixeira", "bruno.t@devs.example"),
            "rust-br@lists.example",
            "[rust-br] Dúvida sobre async traits",
            "rustbr.seed@devs.example",
            extra,
        );
        mails.push(self.mail("INBOX", date, SEEN, head + &qp_text_part("Pessoal, alguém já usou async fn em traits com dyn dispatch em produção?\nQuais crates vocês recomendam?\n\nBruno\n")));

        // Many Cc recipients.
        let date = self.at(6, 500);
        let cc: Vec<String> = [
            ("Pedro Almeida", "pedro.almeida"),
            ("Ana Ribeiro", "ana.ribeiro"),
            ("Lucas Martins", "lucas.martins"),
            ("Juliana Ferreira", "juliana.ferreira"),
            ("Rafael Souza", "rafael.souza"),
            ("Beatriz Rocha", "beatriz.rocha"),
            ("Gabriel Lima", "gabriel.lima"),
            ("Fernanda Dias", "fernanda.dias"),
            ("Marcos Vieira", "marcos.vieira"),
            ("Carla Mendes", "carla.mendes"),
            ("Daniel Kim", "daniel.kim"),
            ("Emily Chen", "emily.chen"),
            ("Tom Becker", "tom.becker"),
            ("Olivia Brown", "olivia.brown"),
        ]
        .iter()
        .map(|(n, u)| addr(n, &format!("{u}@acme.com.br")))
        .collect();
        let extra = format!("Cc: {}\r\n", cc.join(",\r\n "));
        let head = self.head(
            date,
            &addr("Mariana Costa", "mariana.costa@acme.com.br"),
            &me,
            "Kickoff do projeto Atlas",
            "atlas.seed@acme.com.br",
            &extra,
        );
        mails.push(self.mail("INBOX", date, FLAGGED, head + &qp_text_part("Olá a todos,\n\nO kickoff do projeto Atlas será na terça às 10h, auditório principal.\nPauta: objetivos, cronograma e papéis.\n\nMariana\n")));

        // Local sender (bob).
        let date = self.at(0, 20);
        let head = self.head(
            date,
            &addr("Bob", "bob@lowmail.test"),
            &me,
            "Testando o LowMail",
            "bob-test.seed@lowmail.test",
            "",
        );
        mails.push(self.mail("INBOX", date, UNREAD, head + &text_part("Oi Alice, este é um e-mail interno enviado entre contas do servidor de testes.\n\nBob\n")));
        mails
    }

    fn other_folders(&self) -> Vec<Mail> {
        let me = self.me_addr();
        let mut mails = Vec::new();
        let mut push = |mailbox: &'static str,
                        days: i64,
                        mins: i64,
                        from: String,
                        to: String,
                        subject: &str,
                        body: &str,
                        flags: &'static [&'static str],
                        id: &str| {
            let date = self.at(days, mins);
            let head = self.head(date, &from, &to, subject, id, "");
            mails.push(self.mail(mailbox, date, flags, head + &qp_text_part(body)));
        };

        // Sent
        push(
            "Sent",
            2,
            60,
            me.clone(),
            addr("Sarah Johnson", "sarah@northwind.example"),
            "Re: Partnership proposal follow-up",
            "Hi Sarah,\n\nThanks for the follow-up. Next Tuesday at 3pm works for me.\n\nBest,\nAlice\n",
            SEEN,
            "sent1.seed@lowmail.test",
        );
        push(
            "Sent",
            9,
            30,
            me.clone(),
            addr("Equipe", "equipe@acme.com.br"),
            "Atualização do projeto LowMail",
            "Pessoal,\n\nA sincronização IMAP está estável e os testes E2E passam.\nPróximo passo: ManageSieve.\n\nAlice\n",
            SEEN,
            "sent2.seed@lowmail.test",
        );
        push(
            "Sent",
            20,
            400,
            me.clone(),
            addr("Pedro Almeida", "pedro.almeida@acme.com.br"),
            "Re: Almoço sexta?",
            "Fechado! 12h30 no restaurante de sempre.\n",
            SEEN,
            "sent3.seed@lowmail.test",
        );

        // Drafts
        let date = self.at(0, 300);
        let head = self.head(
            date,
            &me,
            &addr("Cliente Acme", "compras@acme-cliente.example"),
            "Proposta de implantação (rascunho)",
            "draft1.seed@lowmail.test",
            "",
        );
        mails.push(self.mail("Drafts", date, &["\\Seen", "\\Draft"], head + &qp_text_part("Prezados,\n\nSegue nossa proposta de implantação do LowMail:\n\n- Fase 1: \n- Fase 2: \n\n[terminar antes de enviar]\n")));

        // Archive
        let mut push = |mailbox: &'static str,
                        days: i64,
                        mins: i64,
                        from: String,
                        subject: &str,
                        body: &str,
                        flags: &'static [&'static str],
                        id: &str| {
            let date = self.at(days, mins);
            let head = self.head(date, &from, &me, subject, id, "");
            mails.push(self.mail(mailbox, date, flags, head + &qp_text_part(body)));
        };
        push(
            "Archive",
            60,
            100,
            addr("Companhia Aérea", "reservas@voeja.example"),
            "Confirmação de reserva – GRU → FLN",
            "Localizador: XK72QP\nVoo VJ 3381, 08:45, assento 14C.\n",
            SEEN,
            "arch1.seed@voeja.example",
        );
        push(
            "Archive",
            72,
            200,
            addr("Prefeitura", "nfse@prefeitura.example"),
            "Nota fiscal eletrônica - agosto",
            "NFS-e nº 2025/0815 emitida no valor de R$ 3.200,00.\n",
            SEEN,
            "arch2.seed@prefeitura.example",
        );
        push(
            "Archive",
            85,
            10,
            addr("Ana Ribeiro", "ana.ribeiro@juridico.example"),
            "Contrato 2024 assinado",
            "Contrato assinado por ambas as partes. Arquivar.\n",
            SEEN,
            "arch3.seed@juridico.example",
        );

        // Junk
        push(
            "Junk",
            1,
            600,
            addr("Prize Center", "winner@prizes.example"),
            "You won $1,000,000!!! Claim now",
            "Congratulations! Click the link to claim your prize: http://prizes.example/claim\n",
            &["$Junk"],
            "junk1.seed@prizes.example",
        );
        push(
            "Junk",
            4,
            60,
            addr("Renda Extra", "renda@ganhe.example"),
            "Ganhe dinheiro rápido trabalhando de casa",
            "Ganhe R$ 5.000 por semana sem sair de casa!!!\n",
            &["$Junk"],
            "junk2.seed@ganhe.example",
        );
        push(
            "Junk",
            14,
            30,
            addr("Security Team", "security@paypa1.example"),
            "Verify your account now",
            "Your account will be suspended. Verify at http://paypa1.example/verify\n",
            &["\\Seen", "$Junk"],
            "junk3.seed@paypa1.example",
        );

        // Trash
        push(
            "Trash",
            30,
            30,
            addr("Loja Exemplo", "ofertas@lojaexemplo.example"),
            "Promoção expirada",
            "Esta promoção já terminou.\n",
            SEEN,
            "trash1.seed@lojaexemplo.example",
        );

        // Projetos
        push(
            "Projetos",
            40,
            120,
            addr("Mariana Costa", "mariana.costa@acme.com.br"),
            "Visão geral dos projetos 2025",
            "Lista de projetos ativos: Atlas, LowMail, Portal do Cliente.\n",
            SEEN,
            "proj1.seed@acme.com.br",
        );
        push(
            "Projetos/Clientes",
            18,
            60,
            addr("Rafael Souza", "rafael.souza@acme.com.br"),
            "Cliente Acme – levantamento de requisitos",
            "Principais requisitos: SSO, retenção de 5 anos, busca full-text.\n",
            SEEN,
            "cli1.seed@acme.com.br",
        );
        push(
            "Projetos/Clientes",
            25,
            300,
            addr("Daniel Kim", "daniel@globex.example"),
            "Globex – proposal approved",
            "Great news: the board approved the proposal. Let's schedule the kickoff.\n",
            FLAGGED,
            "cli2.seed@globex.example",
        );
        push(
            "Projetos/Clientes",
            50,
            90,
            addr("Olivia Brown", "olivia@initech.example"),
            "Initech meeting notes",
            "Notes from today's call attached in the shared folder.\n",
            SEEN,
            "cli3.seed@initech.example",
        );

        // Projetos/Relatórios (one with an attachment)
        let date = self.at(29, 100);
        let head = self.head(
            date,
            &addr("BI Reports", "bi@acme.com.br"),
            &me,
            "Relatório trimestral Q3",
            "rel1.seed@acme.com.br",
            "",
        );
        let body = multipart(
            "mixed",
            "mixed-q3",
            &[
                qp_text_part("Segue o relatório trimestral do Q3 em CSV.\n"),
                attachment(
                    "text/csv; charset=utf-8",
                    "q3.csv",
                    crlf("mes;receita\njul;120000\nago;132500\nset;141200\n").as_bytes(),
                ),
            ],
        );
        mails.push(self.mail("Projetos/Relat&APM-rios", date, SEEN, head + &body));
        let date = self.at(55, 100);
        let head = self.head(
            date,
            &addr("BI Reports", "bi@acme.com.br"),
            &me,
            "Relatório de uso do produto",
            "rel2.seed@acme.com.br",
            "",
        );
        mails.push(self.mail(
            "Projetos/Relat&APM-rios",
            date,
            SEEN,
            head + &qp_text_part("Usuários ativos mensais: 12.430 (+6%).\n"),
        ));
        mails
    }
}

/// Plain-text INBOX messages.
const SIMPLE_INBOX: &[Simple] = &[
    Simple {
        days: 0,
        mins: 40,
        from: ("Mariana Costa", "mariana.costa@acme.com.br"),
        subject: "Reunião de planejamento – Q4",
        body: "Oi Alice,\n\nPodemos marcar a reunião de planejamento do Q4 para quarta às 14h?\nPreciso fechar as metas até sexta.\n\nObrigada,\nMariana\n",
        flags: UNREAD,
    },
    Simple {
        days: 0,
        mins: 130,
        from: ("GitHub", "noreply@github.example"),
        subject: "[lowmail/core] Pull request #42 merged",
        body: "Merged #42 into main.\n\nfeat(imap): handle unsolicited EXPUNGE during IDLE\n\n-- \nReply to this email directly or view it on GitHub.\n",
        flags: UNREAD,
    },
    Simple {
        days: 1,
        mins: 60,
        from: ("Pedro Almeida", "pedro.almeida@acme.com.br"),
        subject: "Almoço sexta?",
        body: "E aí, bora almoçar na sexta? Aquele restaurante novo perto do escritório.\n\nPedro\n",
        flags: ANSWERED,
    },
    Simple {
        days: 1,
        mins: 500,
        from: ("Linear", "notifications@linear.example"),
        subject: "LOW-128: Sync de pastas IMAP",
        body: "Gabriel Lima moved LOW-128 \"Sync de pastas IMAP\" to In Progress.\n\nPriority: High\nCycle: 14\n",
        flags: UNREAD,
    },
    Simple {
        days: 2,
        mins: 300,
        from: ("Banco Exemplo", "avisos@bancoexemplo.example"),
        subject: "Seu extrato de outubro está disponível",
        body: "Olá,\n\nSeu extrato mensal já está disponível no internet banking.\nEste é um e-mail automático, não responda.\n",
        flags: SEEN,
    },
    Simple {
        days: 3,
        mins: 20,
        from: ("Lucas Martins", "lucas.martins@acme.com.br"),
        subject: "Feedback sobre o protótipo",
        body: "Alice,\n\nTestei o protótipo ontem. A busca ficou muito rápida!\nSó achei o contraste do modo escuro um pouco baixo.\n\nLucas\n",
        flags: SEEN,
    },
    Simple {
        days: 4,
        mins: 30,
        from: ("Sarah Johnson", "sarah@northwind.example"),
        subject: "Partnership proposal follow-up",
        body: "Hi Alice,\n\nFollowing up on our call last week. Would you be available next Tuesday to go over the partnership proposal in detail?\n\nBest regards,\nSarah Johnson\nNorthwind Traders\n",
        flags: &["\\Seen", "\\Answered", "\\Flagged"],
    },
    Simple {
        days: 5,
        mins: 60,
        from: ("Equipe RH", "rh@acme.com.br"),
        subject: "Lembrete: avaliação de desempenho",
        body: "Olá,\n\nLembramos que o prazo para a autoavaliação termina na próxima sexta-feira.\n\nEquipe de RH\n",
        flags: SEEN,
    },
    Simple {
        days: 6,
        mins: 200,
        from: ("AWS Notifications", "no-reply@aws.example"),
        subject: "AWS Billing Alert: threshold exceeded",
        body: "Your estimated charges for this month have exceeded USD 50.00.\n\nAccount: 1234-5678-9012\n",
        flags: SEEN,
    },
    Simple {
        days: 7,
        mins: 400,
        from: ("Juliana Ferreira", "juliana.ferreira@acme.com.br"),
        subject: "Fotos do evento",
        body: "Oi Alice!\n\nAs fotos do evento de ontem já estão na pasta compartilhada.\nFicaram ótimas!\n\nJu\n",
        flags: FORWARDED,
    },
    Simple {
        days: 9,
        mins: 100,
        from: ("Tom Becker", "tom@contoso.example"),
        subject: "Quick question about the API",
        body: "Hi Alice,\n\nDoes the LowMail API support OAuth2 for IMAP, or only app passwords?\n\nThanks,\nTom\n",
        flags: ANSWERED,
    },
    Simple {
        days: 10,
        mins: 20,
        from: ("Correios", "rastreamento@correios.example"),
        subject: "Objeto saiu para entrega",
        body: "Seu objeto BR123456789BR saiu para entrega ao destinatário.\n",
        flags: SEEN,
    },
    Simple {
        days: 12,
        mins: 90,
        from: ("Rafael Souza", "rafael.souza@acme.com.br"),
        subject: "Re: Orçamento do servidor",
        body: "Alice,\n\nO orçamento do servidor novo ficou em R$ 18.500,00. Podemos aprovar?\n\nRafael\n",
        flags: SEEN,
    },
    Simple {
        days: 14,
        mins: 10,
        from: ("Emily Chen", "emily.chen@fabrikam.example"),
        subject: "Conference talk accepted!",
        body: "Hi Alice,\n\nGreat news: your talk \"Building a lean email client in Rust\" was accepted for RustConf!\n\nCheers,\nEmily\n",
        flags: FLAGGED,
    },
    Simple {
        days: 18,
        mins: 30,
        from: ("Administração Condomínio", "adm@condominio.example"),
        subject: "Aviso de manutenção do elevador",
        body: "Prezados moradores,\n\nO elevador social estará em manutenção na terça-feira, das 9h às 17h.\n\nAdministração\n",
        flags: SEEN,
    },
    Simple {
        days: 20,
        mins: 80,
        from: ("Gabriel Lima", "gabriel.lima@acme.com.br"),
        subject: "Revisão de código – módulo de busca",
        body: "Oi Alice,\n\nDeixei alguns comentários no PR do módulo de busca. O principal é sobre normalização de acentos.\n\nGabriel\n",
        flags: SEEN,
    },
    Simple {
        days: 22,
        mins: 200,
        from: ("Stripe", "receipts@stripe.example"),
        subject: "Your receipt from LowBloat",
        body: "Receipt #1847-2203\n\nLowMail Pro (monthly) ... USD 9.00\nTotal ... USD 9.00\n",
        flags: SEEN,
    },
    Simple {
        days: 28,
        mins: 300,
        from: ("Beatriz Rocha", "beatriz.rocha@gmail.example"),
        subject: "Convite: aniversário da Clara",
        body: "Oi Alice!\n\nVai ter festinha de aniversário da Clara no sábado às 16h. Você vem?\n\nBeijos,\nBia\n",
        flags: SEEN,
    },
    Simple {
        days: 31,
        mins: 60,
        from: ("Daniel Kim", "daniel@initech.example"),
        subject: "Re: Deployment checklist",
        body: "Hi Alice,\n\nI added the rollback step to the deployment checklist. Please take a look.\n\nDaniel\n",
        flags: ANSWERED,
    },
    Simple {
        days: 37,
        mins: 90,
        from: ("Marcos Vieira", "marcos.vieira@gmail.example"),
        subject: "Viagem para Floripa – roteiro",
        body: "Alice,\n\nMontei um roteiro de 4 dias em Florianópolis: Lagoa da Conceição, Campeche e Santo Antônio de Lisboa.\n\nMarcos\n",
        flags: SEEN,
    },
    Simple {
        days: 42,
        mins: 40,
        from: ("Olivia Brown", "olivia@initech.example"),
        subject: "Invoice #2024-118 overdue",
        body: "Hello,\n\nOur records show invoice #2024-118 is 15 days overdue. Please let us know if there is an issue.\n\nOlivia Brown\nAccounts Receivable\n",
        flags: UNREAD_FLAGGED,
    },
    Simple {
        days: 48,
        mins: 70,
        from: ("Fernanda Dias", "fernanda.dias@acme.com.br"),
        subject: "Documentação do projeto atualizada",
        body: "Pessoal,\n\nAtualizei a documentação com o novo fluxo de autenticação.\n\nFernanda\n",
        flags: SEEN,
    },
    Simple {
        days: 55,
        mins: 30,
        from: ("Security", "security@lowmail.test"),
        subject: "Your verification code",
        body: "Your verification code is 482913. It expires in 10 minutes.\n",
        flags: SEEN,
    },
    Simple {
        days: 63,
        mins: 10,
        from: ("Eventos Tech", "inscricoes@eventostech.example"),
        subject: "Confirmação de inscrição no evento",
        body: "Sua inscrição no Rust Summit Brasil está confirmada.\nData: 12 de dezembro, São Paulo.\n",
        flags: SEEN,
    },
    Simple {
        days: 70,
        mins: 120,
        from: ("Gabriel Lima", "gabriel.lima@acme.com.br"),
        subject: "Re: Re: Fwd: Ideias para o roadmap",
        body: "Gostei das ideias 2 e 5. A 3 parece cara para o próximo trimestre.\n",
        flags: SEEN,
    },
    Simple {
        days: 76,
        mins: 300,
        from: ("Emily Chen", "emily.chen@fabrikam.example"),
        subject: "Weekly status report",
        body: "Status: green.\n- Search indexing: done\n- Offline mode: in progress\n- Calendar: not started\n",
        flags: SEEN,
    },
    Simple {
        days: 82,
        mins: 30,
        from: ("Cobrança", "cobranca@escola.example"),
        subject: "Lembrete de pagamento - boleto",
        body: "O boleto da mensalidade vence em 3 dias. Linha digitável disponível no portal.\n",
        flags: SEEN,
    },
    Simple {
        days: 88,
        mins: 200,
        from: ("Pesquisa", "pesquisa@lowmail.test"),
        subject: "Pesquisa de satisfação",
        body: "Conte para nós como está sendo sua experiência com o LowMail. Leva 2 minutos!\n",
        flags: SEEN,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_words_are_short() {
        let enc =
            encode_words("Atualização importante sobre a política de privacidade e termos de uso");
        for word in enc.split("\r\n ") {
            assert!(word.len() <= 75, "{word}");
        }
        let decoded = crate::message::decode_text(enc.as_bytes());
        assert_eq!(
            decoded,
            "Atualização importante sobre a política de privacidade e termos de uso"
        );
    }

    #[test]
    fn qp_roundtrip_lines() {
        let enc = qp(&"á".repeat(60));
        assert!(enc.lines().all(|l| l.len() <= 76));
    }

    #[test]
    fn demo_seed() {
        let users = vec![
            ("alice@lowmail.test".to_owned(), "alice".to_owned()),
            ("bob@lowmail.test".to_owned(), "bob".to_owned()),
        ];
        let store = Store::new(&users);
        seed(&store, &users, Utc::now());
        let inner = store.lock();
        let inbox = inner.mailbox("alice@lowmail.test", "INBOX").unwrap();
        assert!(inbox.messages.len() >= 55, "{}", inbox.messages.len());
        for m in &inbox.messages {
            assert!(
                !m.data
                    .raw
                    .windows(2)
                    .any(|w| w[1] == b'\n' && w[0] != b'\r'),
                "bare LF in uid {}",
                m.uid
            );
            assert!(!m.data.meta().body_structure.is_empty());
        }
        assert!(
            inner
                .mailbox("alice@lowmail.test", "Projetos/Relat&APM-rios")
                .is_some()
        );
        assert_eq!(
            inner
                .mailbox("bob@lowmail.test", "INBOX")
                .unwrap()
                .messages
                .len(),
            3
        );
        assert_eq!(
            inner
                .mailbox("alice@lowmail.test", "Drafts")
                .unwrap()
                .messages
                .len(),
            1
        );
    }
}
