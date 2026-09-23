//! Server-rendered staff desk pages.

use crate::auth::Role;
use crate::store::{StaffSession, TicketStatus};
use crate::{Facilitator, FacilitatorError};

const STYLE: &str = "body{font-family:system-ui,sans-serif;margin:1.5rem;max-width:72rem}\
table{border-collapse:collapse;width:100%}th,td{border-bottom:1px solid #ccc;padding:.4rem;text-align:left}\
form.inline{display:inline}.escalated{background:#fde2e2}.who{float:right}";

pub(crate) fn login_html(
    state: &Facilitator,
    message: Option<&str>,
) -> Result<String, FacilitatorError> {
    let title = escape_html(state.pack().desk_title());
    let notice = if state.has_users()? {
        String::new()
    } else {
        format!(
            "<p>No staff accounts exist yet. Create one on the server with \
            <code>aice-{} &lt;property.json&gt; user add &lt;name&gt; supervisor</code>.</p>",
            state.pack().as_str()
        )
    };
    let message = message
        .map(|text| format!("<p role=\"alert\">{}</p>", escape_html(text)))
        .unwrap_or_default();
    Ok(format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
        <style>{STYLE}</style></head><body><h1>{title}</h1>{notice}{message}\
        <form method=\"post\" action=\"/login\">\
        <label>Username <input name=\"username\" autocomplete=\"username\" required></label> \
        <label>Password <input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label> \
        <button>Log in</button></form></body></html>"
    ))
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
        let action = |name: &str, label: &str| {
            format!(
                "<form class=\"inline\" method=\"post\" action=\"/api/tickets/{id}/{name}\">\
                {csrf_field}<button>{label}</button></form>"
            )
        };
        let buttons = match TicketStatus::parse(&ticket.status) {
            Some(TicketStatus::Done) if session.role == Role::Supervisor => {
                action("reopen", "Reopen")
            }
            Some(TicketStatus::Done) => String::new(),
            _ => format!(
                "{}{}{}",
                action("acknowledge", "Acknowledge"),
                action("done", "Done"),
                action("escalate", "Escalate")
            ),
        };
        rows.push_str(&format!(
            "<tr class=\"{status}\"><td>{room}</td><td>{tool}</td><td>{status}</td><td>{buttons}</td></tr>"
        ));
    }
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
            "<h2>Audit log</h2><table><tr><th>At (ms)</th><th>Who</th><th>Action</th>\
            <th>Ticket</th><th>Change</th></tr>{audit_rows}</table>"
        )
    } else {
        String::new()
    };
    let stays = stays_html(state, &csrf_field)?;
    let devices = if session.role == Role::Supervisor {
        devices_html(state, &csrf_field)?
    } else {
        String::new()
    };
    let title = escape_html(state.pack().desk_title());
    Ok(format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
        <meta http-equiv=\"refresh\" content=\"10\"><style>{STYLE}</style></head><body>\
        <form class=\"inline who\" method=\"post\" action=\"/logout\">{csrf_field}\
        {} ({}) <button>Log out</button></form><h1>{title}</h1>\
        <table><tr><th>Room</th><th>Request</th><th>Status</th><th></th></tr>{rows}</table>\
        {stays}{devices}{audit}</body></html>",
        escape_html(&session.username),
        session.role.as_str(),
    ))
}

fn stays_html(state: &Facilitator, csrf_field: &str) -> Result<String, FacilitatorError> {
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
            format!(
                "<form class=\"inline\" method=\"post\" action=\"/api/stays/{id}/close\">\
                {csrf_field}<button>Check out</button></form>"
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{id}</td><td>{status}</td><td>{close}</td></tr>",
            escape_html(&stay.room)
        ));
    }
    Ok(format!(
        "<h2>Stays</h2><form method=\"post\" action=\"/api/stays\">{csrf_field}\
        <label>Room <input name=\"room\" size=\"6\" required></label> \
        <label>Continue memory of stay <input name=\"continue_from\" size=\"24\"></label> \
        <button>Check in</button></form>\
        <table><tr><th>Room</th><th>Stay</th><th>Status</th><th></th></tr>{rows}</table>"
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
                {csrf_field}<button>Revoke</button></form>"
            )
        };
        rows.push_str(&format!(
            "<tr><td>{id}</td><td>{}</td><td>{room}</td><td>{}</td><td>{}</td><td>\
            <form class=\"inline\" method=\"post\" action=\"/api/devices/{id}/assign\">{csrf_field}\
            <input name=\"room\" value=\"{room}\" size=\"6\" required><button>Assign room</button></form>{revoke}</td></tr>",
            escape_html(&device.status),
            escape_html(&device.firmware),
            device.last_seen_millis,
        ));
    }
    Ok(format!(
        "<h2>Room devices</h2><table><tr><th>Device</th><th>Status</th><th>Room</th>\
        <th>Firmware</th><th>Last seen (ms)</th><th></th></tr>{rows}</table>"
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
    use super::escape_html;

    #[test]
    fn escape_html_neutralises_markup() {
        assert_eq!(
            escape_html("<b a=\"1\">'&"),
            "&lt;b a=&quot;1&quot;&gt;&#39;&amp;"
        );
    }
}
