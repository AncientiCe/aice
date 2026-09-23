use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use property_facilitator::{
    add_user, serve, AlertChannel, AlertChannelKind, AlertRule, AlertSettings, Pack, Role,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::common::{call_tool, login, settings, temp_db, ticket_id, PASSWORD};

/// A real HTTP endpoint that records every JSON body posted to it.
struct Sink {
    url: String,
    received: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Sink {
    fn bodies(&self) -> Vec<Value> {
        self.received
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

async fn sink() -> Sink {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) => panic!("sink bind: {error}"),
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(error) => panic!("sink addr: {error}"),
    };
    let received = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::clone(&received);
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let store = Arc::clone(&store);
                    async move {
                        let body = request
                            .into_body()
                            .collect()
                            .await
                            .map(|collected| collected.to_bytes())
                            .unwrap_or_default();
                        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                            if let Ok(mut guard) = store.lock() {
                                guard.push(value);
                            }
                        }
                        Ok::<_, Infallible>(Response::new(Full::new(bytes::Bytes::from_static(
                            b"ok",
                        ))))
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Sink {
        url: format!("http://{addr}/hook"),
        received,
        task,
    }
}

fn webhook(name: &str, url: &str, tiers: &[&str]) -> AlertChannel {
    AlertChannel {
        name: name.to_string(),
        kind: AlertChannelKind::Webhook {
            url: url.to_string(),
            bearer: None,
        },
        tiers: tiers.iter().map(|tier| tier.to_string()).collect(),
    }
}

fn fast_rules(ack_within: Duration) -> Vec<AlertRule> {
    vec![AlertRule {
        tools: vec!["*".to_string()],
        ack_within,
    }]
}

async fn wait_for(sink: &Sink, count: usize, within: Duration) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let bodies = sink.bodies();
        if bodies.len() >= count || tokio::time::Instant::now() >= deadline {
            return bodies;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn a_fall_alerts_the_staff_tier_straight_away() {
    let staff = sink().await;
    let db = temp_db("alert-new");
    let mut care = settings(Pack::Care, db.clone());
    care.alerts = AlertSettings {
        channels: vec![webhook("pager", &staff.url, &["staff"])],
        rules: fast_rules(Duration::from_secs(60)),
        ..AlertSettings::default_for(Pack::Care)
    };
    let running = match serve(care).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(&running.url, "report_fall", json!({ "room": "12" })).await;
    let bodies = wait_for(&staff, 1, Duration::from_secs(3)).await;
    assert_eq!(bodies.len(), 1, "one alert: {bodies:?}");
    assert_eq!(bodies[0]["ticket_id"], ticket_id(&created).as_str());
    assert_eq!(bodies[0]["tool"], "report_fall");
    assert_eq!(bodies[0]["room"], "12");
    assert_eq!(bodies[0]["tier"], "staff");
    assert_eq!(bodies[0]["reason"], "new");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn an_unacknowledged_ticket_escalates_to_the_next_tier() {
    let staff = sink().await;
    let supervisor = sink().await;
    let db = temp_db("alert-breach");
    let mut hotel = settings(Pack::Hotels, db.clone());
    hotel.alerts = AlertSettings {
        channels: vec![
            webhook("desk", &staff.url, &["staff"]),
            webhook("duty-manager", &supervisor.url, &["supervisor"]),
        ],
        rules: fast_rules(Duration::from_millis(300)),
        ..AlertSettings::default_for(Pack::Hotels)
    };
    let running = match serve(hotel).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(
        &running.url,
        "request_extra_towels",
        json!({ "room": "204" }),
    )
    .await;
    let id = ticket_id(&created);
    let escalated = wait_for(&supervisor, 1, Duration::from_secs(3)).await;
    assert_eq!(escalated.len(), 1, "supervisor paged: {escalated:?}");
    assert_eq!(escalated[0]["reason"], "sla_breach");
    assert_eq!(escalated[0]["tier"], "supervisor");
    assert_eq!(staff.bodies().len(), 1, "staff got only the first alert");

    if let Err(error) = add_user(&db, "lee", PASSWORD, Role::Supervisor) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "lee", PASSWORD).await;
    let tickets = desk.get_json("/api/tickets").await;
    assert_eq!(tickets[0]["status"], "escalated");
    let audit = desk.get_json("/api/audit").await;
    assert!(audit
        .as_array()
        .map(|events| events
            .iter()
            .any(|event| event["actor"] == "sla" && event["ticket_id"] == id.as_str()))
        .unwrap_or(false));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn acknowledging_in_time_stops_the_escalation() {
    let staff = sink().await;
    let supervisor = sink().await;
    let db = temp_db("alert-ack");
    let mut hotel = settings(Pack::Hotels, db.clone());
    hotel.alerts = AlertSettings {
        channels: vec![
            webhook("desk", &staff.url, &["staff"]),
            webhook("duty-manager", &supervisor.url, &["supervisor"]),
        ],
        rules: fast_rules(Duration::from_millis(600)),
        ..AlertSettings::default_for(Pack::Hotels)
    };
    let running = match serve(hotel).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let created = call_tool(&running.url, "request_iron", json!({ "room": "9" })).await;
    if let Err(error) = add_user(&db, "maria", PASSWORD, Role::Staff) {
        panic!("add user: {error}");
    }
    let desk = login(&running.url, "maria", PASSWORD).await;
    let (status, _) = desk
        .api_post(&format!("/api/tickets/{}/acknowledge", ticket_id(&created)))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        supervisor.bodies().is_empty(),
        "no escalation after acknowledge"
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn one_dead_channel_does_not_stop_the_others() {
    let staff = sink().await;
    let db = temp_db("alert-dead");
    let mut care = settings(Pack::Care, db.clone());
    care.alerts = AlertSettings {
        channels: vec![
            webhook("broken", "http://127.0.0.1:1/hook", &["staff"]),
            webhook("pager", &staff.url, &["staff"]),
        ],
        rules: fast_rules(Duration::from_secs(60)),
        ..AlertSettings::default_for(Pack::Care)
    };
    let running = match serve(care).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let _ = call_tool(&running.url, "report_distress", json!({ "room": "3" })).await;
    let bodies = wait_for(&staff, 1, Duration::from_secs(3)).await;
    assert_eq!(bodies.len(), 1);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(staff.bodies().len(), 1, "a delivered alert is not repeated");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn escalation_timers_survive_a_restart() {
    let supervisor = sink().await;
    let db = temp_db("alert-restart");
    let make = |db: std::path::PathBuf, url: &str| {
        let mut hotel = settings(Pack::Hotels, db);
        hotel.alerts = AlertSettings {
            channels: vec![webhook("duty-manager", url, &["supervisor"])],
            rules: fast_rules(Duration::from_millis(700)),
            ..AlertSettings::default_for(Pack::Hotels)
        };
        hotel
    };
    let first = match serve(make(db.clone(), &supervisor.url)).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let _ = call_tool(&first.url, "request_housekeeping", json!({ "room": "31" })).await;
    drop(first);
    let _second = match serve(make(db.clone(), &supervisor.url)).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let bodies = wait_for(&supervisor, 1, Duration::from_secs(3)).await;
    assert_eq!(
        bodies.len(),
        1,
        "the restarted desk still escalated: {bodies:?}"
    );
    assert_eq!(bodies[0]["reason"], "sla_breach");
    let _ = std::fs::remove_file(db);
}

#[test]
fn alert_settings_are_read_from_property_json() {
    let dir = temp_db("alert-config").with_extension("d");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("dir: {error}");
    }
    if let Err(error) = std::fs::write(dir.join("pager.token"), "pager-token-0123456789-0123456789")
    {
        panic!("token: {error}");
    }
    let config = dir.join("property.json");
    let body = json!({
        "pack": "care",
        "alerts": {
            "tiers": ["carer", "nurse", "manager"],
            "channels": [
                { "name": "pager", "kind": "webhook", "url": "https://pager.local/hook",
                  "token_file": "pager.token", "tiers": ["carer", "nurse"] },
                { "name": "nurse-call", "kind": "property_mcp", "tool": "page_staff", "tiers": ["nurse"] }
            ],
            "rules": [
                { "tools": ["report_fall", "report_distress"], "ack_within_secs": 45 },
                { "tools": ["*"], "ack_within_secs": 600 }
            ]
        }
    });
    if let Err(error) = std::fs::write(&config, body.to_string()) {
        panic!("write: {error}");
    }
    let loaded = match property_facilitator::Settings::load_or_default(Pack::Care, &config) {
        Ok(settings) => settings,
        Err(error) => panic!("load: {error}"),
    };
    assert_eq!(loaded.alerts.tiers, vec!["carer", "nurse", "manager"]);
    assert_eq!(loaded.alerts.channels.len(), 2);
    assert!(matches!(
        &loaded.alerts.channels[0].kind,
        AlertChannelKind::Webhook { bearer: Some(token), .. } if token.starts_with("pager-token")
    ));
    assert_eq!(
        loaded.alerts.ack_within("report_fall"),
        Some(Duration::from_secs(45))
    );
    assert_eq!(
        loaded.alerts.ack_within("request_drink"),
        Some(Duration::from_secs(600))
    );
    let defaults = AlertSettings::default_for(Pack::Care);
    assert_eq!(
        defaults.ack_within("report_fall"),
        Some(Duration::from_secs(60))
    );
    let _ = std::fs::remove_dir_all(dir);
}
