//! Server-rendered staff desk pages.

use crate::auth::Role;
use crate::store::{StaffSession, TicketStatus};
use crate::{Facilitator, FacilitatorError};

const STYLE: &str = ":root{--bg:#f4f6f8;--card:#fff;--text:#1b2733;--muted:#5d6b79;--line:#e3e8ed;\
--accent:#2457d6;--accent-text:#fff;--open:#b45309;--open-bg:#fef3c7;--ack:#1d4ed8;--ack-bg:#dbeafe;\
--done:#166534;--done-bg:#dcfce7;--esc:#b91c1c;--esc-bg:#fee2e2;--off:#475569;--off-bg:#e2e8f0}\
@media(prefers-color-scheme:dark){:root{--bg:#0f1720;--card:#19232e;--text:#e6edf3;--muted:#93a4b5;\
--line:#2a3745;--accent:#6b93f5;--accent-text:#0f1720;--open:#fbbf24;--open-bg:#3a2d0c;--ack:#93c5fd;\
--ack-bg:#16294d;--done:#86efac;--done-bg:#12301d;--esc:#fca5a5;--esc-bg:#431616;--off:#cbd5e1;--off-bg:#2b3643}}\
*{box-sizing:border-box}\
body{font-family:system-ui,-apple-system,\"Segoe UI\",sans-serif;margin:0;background:var(--bg);color:var(--text);line-height:1.45}\
header.bar{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:.75rem;\
padding:.9rem 1.5rem;background:var(--card);border-bottom:1px solid var(--line);position:sticky;top:0}\
header.bar h1{margin:0;font-size:1.25rem}\
main{max-width:76rem;margin:0 auto;padding:1.25rem 1.5rem 3rem;display:grid;grid-template-columns:minmax(0,1fr);gap:1.25rem}\
section.card{background:var(--card);border:1px solid var(--line);border-radius:.75rem;padding:1rem 1.25rem;\
box-shadow:0 1px 2px rgba(0,0,0,.05)}\
section.card h2{margin:0 0 .75rem;font-size:1.05rem}\
.scroll{overflow-x:auto}\
table{border-collapse:collapse;width:100%}\
th{font-size:.75rem;text-transform:uppercase;letter-spacing:.04em;color:var(--muted);font-weight:600}\
th,td{border-bottom:1px solid var(--line);padding:.55rem .6rem;text-align:left;vertical-align:middle}\
tbody tr:last-child td{border-bottom:0}\
tr.escalated td{background:var(--esc-bg)}\
.empty{color:var(--muted);margin:.25rem 0}\
.badge{display:inline-block;padding:.12rem .55rem;border-radius:999px;font-size:.78rem;font-weight:600;\
background:var(--off-bg);color:var(--off)}\
.badge.open{background:var(--open-bg);color:var(--open)}\
.badge.acknowledged{background:var(--ack-bg);color:var(--ack)}\
.badge.done{background:var(--done-bg);color:var(--done)}\
.badge.escalated{background:var(--esc-bg);color:var(--esc)}\
form.inline{display:inline-block;margin:0 .25rem .25rem 0}\
form.checkin{display:flex;flex-wrap:wrap;gap:.75rem 1rem;align-items:end;margin-bottom:1rem}\
label{display:grid;gap:.2rem;font-size:.8rem;color:var(--muted)}\
label.check{display:flex;align-items:center;gap:.4rem;padding-bottom:.45rem}\
input:not([type=checkbox]){font:inherit;padding:.4rem .55rem;border:1px solid var(--line);border-radius:.4rem;\
background:var(--bg);color:var(--text)}\
button{font:inherit;font-size:.85rem;padding:.38rem .75rem;border:1px solid var(--line);border-radius:.4rem;\
background:var(--card);color:var(--text);cursor:pointer}\
button:hover{border-color:var(--accent)}\
button:focus-visible,input:focus-visible{outline:2px solid var(--accent);outline-offset:1px}\
button.primary{background:var(--accent);border-color:var(--accent);color:var(--accent-text)}\
button.danger{color:var(--esc)}\
.who{display:flex;align-items:center;gap:.6rem;color:var(--muted);font-size:.9rem}\
.who strong{color:var(--text)}\
.login{max-width:24rem;margin:12vh auto 0;padding:0 1rem}\
.login form{display:grid;gap:.9rem}\
.notice{padding:.6rem .8rem;border-radius:.5rem;background:var(--ack-bg);color:var(--ack)}\
.notice[role=alert]{background:var(--esc-bg);color:var(--esc)}\
@media(max-width:40rem){header.bar,main{padding-left:1rem;padding-right:1rem}}";

fn page_shell(title: &str, body: &str, live: bool) -> String {
    let refresh = if live {
        "<meta http-equiv=\"refresh\" content=\"10\">"
    } else {
        ""
    };
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
        <title>{title}</title>{refresh}<style>{STYLE}</style></head><body>{body}</body></html>"
    )
}

fn status_badge(label: &str) -> String {
    let class = match label.to_ascii_lowercase().as_str() {
        known @ ("open" | "acknowledged" | "done" | "escalated" | "offline") => known.to_string(),
        _ => "other".to_string(),
    };
    format!(
        "<span class=\"badge {class}\">{}</span>",
        escape_html(label)
    )
}

pub(crate) fn login_html(
    state: &Facilitator,
    message: Option<&str>,
) -> Result<String, FacilitatorError> {
    let title = escape_html(state.pack().desk_title());
    let notice = if state.has_users()? {
        String::new()
    } else {
        format!(
            "<p class=\"notice\">No staff accounts exist yet. Create one on the server with \
            <code>aice-{} &lt;property.json&gt; user add &lt;name&gt; supervisor</code>.</p>",
            state.pack().as_str()
        )
    };
    let message = message
        .map(|text| {
            format!(
                "<p class=\"notice\" role=\"alert\">{}</p>",
                escape_html(text)
            )
        })
        .unwrap_or_default();
    let body = format!(
        "<main class=\"login\"><section class=\"card\"><h1>{title}</h1>{notice}{message}\
        <form method=\"post\" action=\"/login\">\
        <label>Username <input name=\"username\" autocomplete=\"username\" required></label>\
        <label>Password <input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>\
        <button class=\"primary\">Log in</button></form></section></main>"
    );
    Ok(page_shell(&title, &body, false))
}

pub(crate) fn desk_html(
    state: &Facilitator,
    session: &StaffSession,
) -> Result<String, FacilitatorError> {
    let csrf = escape_html(&session.csrf);
    let csrf_field = format!("<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">");
    let tickets = state.list_tickets()?;
    let mut rows = String::new();
    for ticket in &tickets {
        let room = escape_html(&ticket.room);
        let tool = escape_html(&ticket.tool_name);
        let status = escape_html(&ticket.status);
        let id = escape_html(&ticket.id);
        let action = |name: &str, label: &str, class: &str| {
            format!(
                "<form class=\"inline\" method=\"post\" action=\"/api/tickets/{id}/{name}\">\
                {csrf_field}<button class=\"{class}\">{label}</button></form>"
            )
        };
        let buttons = match TicketStatus::parse(&ticket.status) {
            Some(TicketStatus::Done) if session.role == Role::Supervisor => {
                action("reopen", "Reopen", "")
            }
            Some(TicketStatus::Done) => String::new(),
            _ => format!(
                "{}{}{}",
                action("acknowledge", "Acknowledge", "primary"),
                action("done", "Done", ""),
                action("escalate", "Escalate", "danger")
            ),
        };
        rows.push_str(&format!(
            "<tr class=\"{status}\"><td>{room}</td><td>{tool}</td><td>{}</td><td>{buttons}</td></tr>",
            status_badge(&ticket.status)
        ));
    }
    let tickets_table = if tickets.is_empty() {
        "<p class=\"empty\">No requests right now.</p>".to_string()
    } else {
        format!(
            "<div class=\"scroll\"><table><thead><tr><th>Room</th><th>Request</th><th>Status</th>\
            <th></th></tr></thead><tbody>{rows}</tbody></table></div>"
        )
    };
    let audit = if session.role == Role::Supervisor {
        let mut audit_rows = String::new();
        for event in state.list_audit(50)? {
            audit_rows.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                event.at_millis,
                escape_html(&event.actor),
                escape_html(&event.action),
                escape_html(event.ticket_id.as_deref().unwrap_or("")),
                escape_html(&format!(
                    "{} {}",
                    event.from_status.as_deref().unwrap_or(""),
                    event.to_status.as_deref().unwrap_or("")
                )),
            ));
        }
        format!(
            "<section class=\"card\"><h2>Audit log</h2><div class=\"scroll\"><table><thead><tr>\
            <th>At (ms)</th><th>Who</th><th>Action</th><th>Ticket</th><th>Change</th></tr></thead>\
            <tbody>{audit_rows}</tbody></table></div></section>"
        )
    } else {
        String::new()
    };
    let stays = stays_html(state, &csrf_field, state.memory_consent_default())?;
    let devices = if session.role == Role::Supervisor {
        devices_html(state, &csrf_field)?
    } else {
        String::new()
    };
    let title = escape_html(state.pack().desk_title());
    let body = format!(
        "<header class=\"bar\"><h1>{title}</h1>\
        <form class=\"who\" method=\"post\" action=\"/logout\">{csrf_field}\
        <span><strong>{}</strong> ({})</span> <button>Log out</button></form></header>\
        <main><section class=\"card\"><h2>Requests</h2>{tickets_table}</section>\
        {stays}{devices}{audit}</main>",
        escape_html(&session.username),
        session.role.as_str(),
    );
    Ok(page_shell(&title, &body, true))
}

fn stays_html(
    state: &Facilitator,
    csrf_field: &str,
    consent_default: bool,
) -> Result<String, FacilitatorError> {
    let mut rows = String::new();
    for stay in state.list_stays()? {
        let id = escape_html(&stay.id);
        let status = match (stay.closed_millis, stay.purged_millis) {
            (None, _) => "open",
            (Some(_), Some(_)) => "memory deleted",
            (Some(_), None) if stay.purge_after_millis.is_some() => "closed (memory to delete)",
            (Some(_), None) => "closed (memory kept)",
        };
        let close = if stay.closed_millis.is_none() {
            let (flip, label) = if stay.memory_consent {
                ("no", "Memory: on (turn off)")
            } else {
                ("yes", "Memory: off (resident agreed)")
            };
            format!(
                "<form class=\"inline\" method=\"post\" action=\"/api/stays/{id}/consent\">\
                {csrf_field}<input type=\"hidden\" name=\"memory_consent\" value=\"{flip}\">\
                <button>{label}</button></form>\
                <form class=\"inline\" method=\"post\" action=\"/api/stays/{id}/close\">\
                {csrf_field}<button class=\"danger\">Check out</button></form>"
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{id}</td><td>{}</td><td>{close}</td></tr>",
            escape_html(&stay.room),
            status_badge(status)
        ));
    }
    Ok(format!(
        "<section class=\"card\"><h2>Stays</h2><form class=\"checkin\" method=\"post\" action=\"/api/stays\">{csrf_field}\
        <label>Room <input name=\"room\" size=\"6\" required></label>\
        <label>Continue memory of stay <input name=\"continue_from\" size=\"24\"></label>\
        <label class=\"check\"><input type=\"checkbox\" name=\"memory_consent\" value=\"yes\"{checked}> Agreed to voice memory</label>\
        <input type=\"hidden\" name=\"memory_consent\" value=\"no\">\
        <button class=\"primary\">Check in</button></form>\
        <div class=\"scroll\"><table><thead><tr><th>Room</th><th>Stay</th><th>Status</th><th></th></tr></thead>\
        <tbody>{rows}</tbody></table></div></section>",
        checked = if consent_default { " checked" } else { "" }
    ))
}

fn devices_html(state: &Facilitator, csrf_field: &str) -> Result<String, FacilitatorError> {
    let mut rows = String::new();
    for device in state.list_devices()? {
        let id = escape_html(&device.device_id);
        let room = escape_html(device.room.as_deref().unwrap_or(""));
        let revoke = if device.status == "revoked" {
            String::new()
        } else {
            format!(
                "<form class=\"inline\" method=\"post\" action=\"/api/devices/{id}/revoke\">\
                {csrf_field}<button class=\"danger\">Revoke</button></form>"
            )
        };
        rows.push_str(&format!(
            "<tr><td>{id}</td><td>{}</td><td>{room}</td><td>{}</td><td>{}</td><td>\
            <form class=\"inline\" method=\"post\" action=\"/api/devices/{id}/assign\">{csrf_field}\
            <input name=\"room\" value=\"{room}\" size=\"6\" required><button>Assign room</button></form>{revoke}</td></tr>",
            if device.offline_since_millis.is_some() {
                status_badge("OFFLINE")
            } else {
                status_badge(&device.status)
            },
            escape_html(&device.firmware),
            device.last_seen_millis,
        ));
    }
    Ok(format!(
        "<section class=\"card\"><h2>Room devices</h2><div class=\"scroll\"><table><thead><tr>\
        <th>Device</th><th>Status</th><th>Room</th><th>Firmware</th><th>Last seen (ms)</th><th></th>\
        </tr></thead><tbody>{rows}</tbody></table></div></section>"
    ))
}

pub(crate) fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::{escape_html, page_shell, status_badge};

    #[test]
    fn page_shell_is_responsive_and_escapes_nothing_twice() {
        let html = page_shell("Desk", "<p>body</p>", true);
        assert!(html
            .contains("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"));
        assert!(html.contains("<meta http-equiv=\"refresh\" content=\"10\">"));
        assert!(html.contains("<title>Desk</title>"));
        assert!(html.contains("<p>body</p>"));
        assert!(html.contains("prefers-color-scheme:dark"));
    }

    #[test]
    fn page_shell_omits_refresh_when_not_live() {
        let html = page_shell("Login", "", false);
        assert!(!html.contains("http-equiv=\"refresh\""));
    }

    #[test]
    fn status_badge_carries_a_status_class_and_escaped_label() {
        assert_eq!(
            status_badge("escalated"),
            "<span class=\"badge escalated\">escalated</span>"
        );
        assert_eq!(
            status_badge("<x>"),
            "<span class=\"badge other\">&lt;x&gt;</span>"
        );
    }

    #[test]
    fn status_badge_marks_offline_devices() {
        assert_eq!(
            status_badge("OFFLINE"),
            "<span class=\"badge offline\">OFFLINE</span>"
        );
    }

    #[test]
    fn escape_html_neutralises_markup() {
        assert_eq!(
            escape_html("<b a=\"1\">'&"),
            "&lt;b a=&quot;1&quot;&gt;&#39;&amp;"
        );
    }
}
