//! Canonical intent-classifier contract shared by backend and desktop runtimes.

const ALL_CLASSIFIER_SKILLS: [&str; 31] = [
    "skill_weather",
    "skill_time",
    "skill_distance",
    "skill_sports_live",
    "skill_holiday_lookup",
    "skill_fuel_price_lookup",
    "skill_horoscope_daily",
    "skill_news_headlines",
    "skill_calculator",
    "skill_unit_conversion",
    "skill_currency",
    "skill_air_quality",
    "skill_dictionary",
    "skill_translate",
    "skill_calendar",
    "skill_meeting_notes",
    "skill_email",
    "skill_briefing",
    "skill_journal",
    "skill_screen_ocr",
    "skill_smart_home",
    "skill_media",
    "skill_computer",
    "skill_screenshot",
    "skill_app_switcher",
    "skill_reminder",
    "skill_timer",
    "skill_shopping_list",
    "skill_message",
    "skill_volume",
    "skill_hotel",
];

/// Canonical hotel concierge intent kinds (mirrors `skill_hotel::HotelIntentKind`).
const HOTEL_INTENT_KINDS: [&str; 25] = [
    "set_room_temperature",
    "set_lights",
    "set_curtains",
    "set_tv",
    "set_ambient_music",
    "set_do_not_disturb",
    "order_room_service",
    "request_housekeeping",
    "request_extra_towels",
    "request_extra_pillows",
    "request_toiletries",
    "request_laundry_pickup",
    "request_iron",
    "set_wake_up_call",
    "request_late_checkout",
    "book_restaurant",
    "book_spa",
    "book_taxi",
    "concierge_info",
    "request_local_recommendation",
    "report_complaint",
    "report_lost_item",
    "request_billing_summary",
    "request_checkout",
    "language_help",
];

fn has_skill(available_skills: &[&str], skill: &str) -> bool {
    available_skills
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case(skill))
}

/// Short intent name used in compact JSON output to minimise decode tokens.
fn short_intent_name(skill: &str) -> &'static str {
    match skill {
        "skill_weather" => "weather",
        "skill_time" => "time",
        "skill_distance" => "dist",
        "skill_sports_live" => "sports",
        "skill_holiday_lookup" => "holiday",
        "skill_fuel_price_lookup" => "fuel",
        "skill_horoscope_daily" => "horoscope",
        "skill_news_headlines" => "news",
        "skill_calculator" => "calc",
        "skill_unit_conversion" => "unit",
        "skill_currency" => "fx",
        "skill_air_quality" => "air",
        "skill_dictionary" => "dict",
        "skill_translate" => "tx",
        "skill_calendar" => "cal",
        "skill_meeting_notes" => "mtg",
        "skill_email" => "email",
        "skill_briefing" => "brief",
        "skill_journal" => "jrnl",
        "skill_screen_ocr" => "ocr",
        "skill_smart_home" => "shome",
        "skill_media" => "media",
        "skill_computer" => "computer",
        "skill_screenshot" => "screenshot",
        "skill_app_switcher" => "aswitch",
        "skill_reminder" => "reminder",
        "skill_timer" => "timer",
        "skill_shopping_list" => "shop",
        "skill_message" => "msg",
        "skill_volume" => "vol",
        "skill_hotel" => "hotel",
        _ => "chat",
    }
}

/// Compact schema field entries per skill. Uses single-letter generic keys
/// where possible to minimise output tokens.
///
/// Key legend: `l`=location, `o`=origin, `d`=destination, `t`=target,
/// `q`=query, `n`=name/title, `w`=when/date, `p`=params/items,
/// `v`=value(string), `vl`=value(number/level).
fn schema_fields_for_skill(skill: &str) -> &'static [(&'static str, &'static str)] {
    match skill {
        "skill_weather" | "skill_time" => &[("l", "string")],
        "skill_distance" => &[("o", "string"), ("d", "string")],
        "skill_sports_live" => &[("q", "string"), ("w", "string")],
        "skill_holiday_lookup" => &[
            ("n", "string"),
            ("w", "string"),
            ("hcc", "string"),
            ("hrc", "string"),
            ("hy", "number"),
        ],
        "skill_fuel_price_lookup" => &[("fcc", "string"), ("fr", "string"), ("ft", "string")],
        "skill_horoscope_daily" => &[("hs", "string"), ("w", "string")],
        "skill_news_headlines" => &[("q", "string"), ("ncc", "string"), ("nl", "number")],
        "skill_calculator" => &[("q", "string")],
        "skill_unit_conversion" => &[
            ("q", "string"),
            ("uv", "number"),
            ("uf", "string"),
            ("ut", "string"),
        ],
        "skill_currency" => &[("cam", "number"), ("cf", "string"), ("ct", "string")],
        "skill_air_quality" => &[("l", "string")],
        "skill_dictionary" => &[("q", "string")],
        "skill_translate" => &[("q", "string"), ("tsl", "string"), ("ttl", "string")],
        "skill_calendar" => &[
            ("n", "string"),
            ("w", "string"),
            ("cdy", "number"),
            ("l", "string"),
            ("ccn", "string"),
        ],
        "skill_meeting_notes" => &[("q", "string"), ("n", "string"), ("mr", "boolean")],
        "skill_email" => &[("q", "string"), ("el", "number"), ("em", "string")],
        "skill_briefing" => &[("bi", "[string]"), ("bnt", "string"), ("bnc", "string")],
        "skill_journal" => &[
            ("v", "string"),
            ("js", "string"),
            ("jt", "[string]"),
            ("q", "string"),
            ("jl", "number"),
        ],
        "skill_screen_ocr" => &[("q", "string"), ("sf", "string")],
        "skill_smart_home" => &[("t", "string")],
        "skill_media" => &[("t", "string")],
        "skill_computer" => &[("t", "string")],
        "skill_screenshot" => &[("sf", "string")],
        "skill_app_switcher" => &[("t", "string")],
        "skill_reminder" => &[("n", "string"), ("w", "string")],
        "skill_timer" => &[("v", "string"), ("n", "string")],
        "skill_shopping_list" => &[("p", "[string]"), ("w", "string")],
        "skill_message" => &[("t", "string"), ("v", "string")],
        "skill_volume" => &[("vl", "number")],
        "skill_hotel" => &[("hik", "hotel_intent_kind"), ("hsl", "object")],
        _ => &[],
    }
}

fn collect_valid_intents(available_skills: &[&str]) -> Vec<String> {
    let mut intents = vec!["chat".to_string()];
    for skill in ALL_CLASSIFIER_SKILLS {
        if has_skill(available_skills, skill) {
            intents.push(short_intent_name(skill).to_string());
        }
    }
    intents
}

/// Build a JSON Schema that Ollama can use for grammar-constrained generation.
///
/// The `intent` field gets a strict `enum` so the model literally cannot output
/// an invalid intent string.
pub fn intent_classifier_json_schema_for_skills(available_skills: &[&str]) -> serde_json::Value {
    let valid_intents = collect_valid_intents(available_skills);

    let mut properties = serde_json::Map::new();
    properties.insert(
        "i".to_string(),
        serde_json::json!({
            "type": "string",
            "enum": valid_intents,
        }),
    );
    properties.insert("c".to_string(), serde_json::json!({ "type": "string" }));

    let mut seen = std::collections::HashSet::new();
    for skill in ALL_CLASSIFIER_SKILLS {
        if !has_skill(available_skills, skill) {
            continue;
        }
        for &(name, ty) in schema_fields_for_skill(skill) {
            if !seen.insert(name) {
                continue;
            }
            let json_ty = match ty {
                "number" => serde_json::json!({ "type": "number" }),
                "boolean" => serde_json::json!({ "type": "boolean" }),
                "[string]" => serde_json::json!({ "type": "array", "items": { "type": "string" } }),
                "object" => serde_json::json!({ "type": "object", "additionalProperties": true }),
                "hotel_intent_kind" => serde_json::json!({
                    "type": "string",
                    "enum": HOTEL_INTENT_KINDS,
                }),
                _ => serde_json::json!({ "type": "string" }),
            };
            properties.insert(name.to_string(), json_ty);
        }
    }

    serde_json::json!({
        "type": "object",
        "required": ["i"],
        "properties": properties,
        "additionalProperties": false,
    })
}

/// Compact per-skill rule line using short intent names and short field keys.
/// `!` = required, `?` = optional.
fn render_compact_rules(available_skills: &[&str]) -> String {
    let mut lines = vec!["chat: no c".to_string()];
    let rules: &[(&str, &str)] = &[
        ("skill_weather", "c=get l?"),
        ("skill_time", "c=get l?"),
        ("skill_distance", "c=get o? d?"),
        ("skill_sports_live", "c=get q? w?"),
        ("skill_holiday_lookup", "c=get n? w? hcc? hrc? hy?"),
        ("skill_fuel_price_lookup", "c=get fcc? fr? ft?"),
        ("skill_horoscope_daily", "c=get hs? w?"),
        ("skill_news_headlines", "c=get q? ncc? nl?"),
        ("skill_calculator", "c=eval q!"),
        ("skill_unit_conversion", "c=convert q? uv? uf? ut?"),
        ("skill_currency", "c=convert cam? cf? ct?"),
        ("skill_air_quality", "c=get l?"),
        ("skill_dictionary", "c=define q!"),
        ("skill_translate", "c=translate q! tsl? ttl!"),
        (
            "skill_calendar",
            "c=list_today|list_tomorrow|list_upcoming|create n? w? cdy? l? ccn?",
        ),
        ("skill_meeting_notes", "c=summarize q! n? mr?"),
        (
            "skill_email",
            "c=list_unread|list_inbox|search|triage q? el? em?",
        ),
        ("skill_briefing", "c=get bi? bnt? bnc?"),
        ("skill_journal", "c=add|recall|stats v? js? jt? q? jl?"),
        ("skill_screen_ocr", "c=ask q? sf?"),
        ("skill_smart_home", "c=on|off|toggle|status|set t?"),
        (
            "skill_media",
            "c=play|pause|resume|next|previous|shuffle_on|shuffle_off|status t?",
        ),
        ("skill_computer", "c=open|launch|browse|run t?"),
        ("skill_screenshot", "c=take sf?"),
        (
            "skill_app_switcher",
            "c=switch|next|previous|hide|quit|force_quit t?",
        ),
        ("skill_reminder", "c=add n! w?"),
        ("skill_timer", "c=set v! n?"),
        ("skill_shopping_list", "c=add|remove p! w?"),
        ("skill_message", "c=send t! v?"),
        ("skill_volume", "c=set|up|down|mute|unmute|get vl?"),
        (
            "skill_hotel",
            "c=request hik=set_room_temperature|set_lights|set_curtains|set_tv|set_ambient_music|set_do_not_disturb|order_room_service|request_housekeeping|request_extra_towels|request_extra_pillows|request_toiletries|request_laundry_pickup|request_iron|set_wake_up_call|request_late_checkout|book_restaurant|book_spa|book_taxi|concierge_info|request_local_recommendation|report_complaint|report_lost_item|request_billing_summary|request_checkout|language_help hsl?",
        ),
    ];
    for &(skill, rule) in rules {
        if has_skill(available_skills, skill) {
            let short = short_intent_name(skill);
            lines.push(format!("{short}: {rule}"));
        }
    }
    lines.join("\n")
}

fn render_disambiguation(available_skills: &[&str]) -> String {
    let mut lines = Vec::new();
    if has_skill(available_skills, "skill_media") {
        lines.push("resume=unpause/continue. play=new playback only.");
    }
    if has_skill(available_skills, "skill_message") {
        lines.push(
            "msg v: rewrite indirect phrasing to direct recipient text. \"how she is\"->\"How are you?\". Omit v if user gave no content.",
        );
    }
    if has_skill(available_skills, "skill_media") && has_skill(available_skills, "skill_smart_home")
    {
        lines.push("media=music/audio. shome=lights/devices/climate.");
    }
    if has_skill(available_skills, "skill_media")
        && has_skill(available_skills, "skill_app_switcher")
    {
        lines.push("media next/prev=audio track. aswitch next/prev=app/window.");
    }
    let has_calc = has_skill(available_skills, "skill_calculator");
    let has_unit = has_skill(available_skills, "skill_unit_conversion");
    let has_fx = has_skill(available_skills, "skill_currency");
    if (has_calc as u8 + has_unit as u8 + has_fx as u8) >= 2 {
        lines.push("calc=arithmetic. unit=X to Y units. fx=X to Y currencies.");
    }
    if has_skill(available_skills, "skill_translate") {
        lines.push("tx requires explicit ttl (target language). Otherwise chat.");
    }
    if has_skill(available_skills, "skill_meeting_notes") {
        lines.push("mtg requires q (transcript text). Without it use chat.");
    }
    if has_skill(available_skills, "skill_journal") && has_skill(available_skills, "skill_reminder")
    {
        lines.push("jrnl=personal log entry. reminder=task with due time.");
    }
    if has_skill(available_skills, "skill_email") && has_skill(available_skills, "skill_message") {
        lines.push("email=mailbox/inbox/unread. msg=iMessage/SMS to a contact.");
    }
    if has_skill(available_skills, "skill_calendar") {
        lines
            .push("cal action: today/tomorrow/upcoming list events; create needs n (title) and w.");
    }
    if has_skill(available_skills, "skill_hotel") {
        lines.push(
            "hotel: in-room concierge requests. Always set hik. Put structured details in hsl (object).",
        );
    }
    lines.join("\n")
}

/// Build the canonical system prompt for LLM intent classification.
pub fn intent_classifier_system_prompt() -> String {
    intent_classifier_system_prompt_for_skills(&ALL_CLASSIFIER_SKILLS)
}

/// Build the compact system prompt for a set of available skills.
///
/// Optimized for minimal token count while preserving classification accuracy.
/// Grammar-constrained decoding (JSON schema) handles structural validation, so
/// the prompt focuses on skill semantics and disambiguation only.
pub fn intent_classifier_system_prompt_for_skills(available_skills: &[&str]) -> String {
    let valid_intents = collect_valid_intents(available_skills);
    let valid_intents_json = format!(
        "[{}]",
        valid_intents
            .iter()
            .map(|intent| format!("\"{intent}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let compact_rules = render_compact_rules(available_skills);
    let disambiguation = render_disambiguation(available_skills);

    let mut prompt = String::from(
        "Strict intent classifier. JSON only. Most specific skill. Required: \"i\". Non-chat: add \"c\". Omit nulls.\n\nIntents: ",
    );
    prompt.push_str(&valid_intents_json);
    prompt.push_str("\n\n");
    prompt.push_str(&compact_rules);
    if !disambiguation.is_empty() {
        prompt.push('\n');
        prompt.push_str(&disambiguation);
    }
    prompt
}

/// Canonical few-shot examples for intent classification.
///
/// Kept minimal: one example per disambiguation-critical boundary. Simple
/// `command:"get"` skills (sports, holiday, fuel, horoscope, news) rely on
/// per-intent rules and don't need few-shots.
pub fn intent_classifier_few_shots() -> Vec<(String, String)> {
    vec![
        ("tell me a joke".to_string(), "{\"i\":\"chat\"}".to_string()),
        (
            "what's the weather in Paris?".to_string(),
            "{\"i\":\"weather\",\"c\":\"get\",\"l\":\"Paris, France\"}".to_string(),
        ),
        (
            "time in Tokyo".to_string(),
            "{\"i\":\"time\",\"c\":\"get\",\"l\":\"Tokyo, Japan\"}".to_string(),
        ),
        (
            "how far is Paris?".to_string(),
            "{\"i\":\"dist\",\"c\":\"get\",\"d\":\"Paris, France\"}".to_string(),
        ),
        (
            "set a timer for 5 minutes".to_string(),
            "{\"i\":\"timer\",\"c\":\"set\",\"v\":\"5 minutes\"}".to_string(),
        ),
        (
            "remind me in 5 minutes to ask my wife how she is".to_string(),
            "{\"i\":\"reminder\",\"c\":\"add\",\"n\":\"Ask my wife how she is\",\"w\":\"PT5M\"}"
                .to_string(),
        ),
        (
            "ask my wife how she is".to_string(),
            "{\"i\":\"msg\",\"c\":\"send\",\"t\":\"my wife\",\"v\":\"How are you?\"}".to_string(),
        ),
        (
            "send a message to my wife.".to_string(),
            "{\"i\":\"msg\",\"c\":\"send\",\"t\":\"my wife\"}".to_string(),
        ),
        (
            "unpause.".to_string(),
            "{\"i\":\"media\",\"c\":\"resume\"}".to_string(),
        ),
        (
            "turn on the kitchen lights".to_string(),
            "{\"i\":\"shome\",\"c\":\"on\",\"t\":\"kitchen lights\"}".to_string(),
        ),
        (
            "switch to the next app".to_string(),
            "{\"i\":\"aswitch\",\"c\":\"next\"}".to_string(),
        ),
        (
            "what's 12 times 7".to_string(),
            "{\"i\":\"calc\",\"c\":\"eval\",\"q\":\"12 * 7\"}".to_string(),
        ),
        (
            "convert 10 pounds to kilograms".to_string(),
            "{\"i\":\"unit\",\"c\":\"convert\",\"uv\":10,\"uf\":\"lb\",\"ut\":\"kg\"}".to_string(),
        ),
        (
            "how much is 100 USD in EUR".to_string(),
            "{\"i\":\"fx\",\"c\":\"convert\",\"cam\":100,\"cf\":\"USD\",\"ct\":\"EUR\"}"
                .to_string(),
        ),
        (
            "is the air quality bad in Milan".to_string(),
            "{\"i\":\"air\",\"c\":\"get\",\"l\":\"Milan, Italy\"}".to_string(),
        ),
        (
            "what does serendipity mean".to_string(),
            "{\"i\":\"dict\",\"c\":\"define\",\"q\":\"serendipity\"}".to_string(),
        ),
        (
            "translate 'good morning' to italian".to_string(),
            "{\"i\":\"tx\",\"c\":\"translate\",\"q\":\"good morning\",\"ttl\":\"it\"}".to_string(),
        ),
        (
            "what's on my calendar today".to_string(),
            "{\"i\":\"cal\",\"c\":\"list_today\"}".to_string(),
        ),
        (
            "summarize this meeting transcript: Alice ship Friday. Bob ok.".to_string(),
            "{\"i\":\"mtg\",\"c\":\"summarize\",\"q\":\"Alice ship Friday. Bob ok.\"}".to_string(),
        ),
        (
            "any unread emails".to_string(),
            "{\"i\":\"email\",\"c\":\"list_unread\"}".to_string(),
        ),
        (
            "give me my morning briefing".to_string(),
            "{\"i\":\"brief\",\"c\":\"get\"}".to_string(),
        ),
        (
            "journal today: had a great run".to_string(),
            "{\"i\":\"jrnl\",\"c\":\"add\",\"v\":\"had a great run\"}".to_string(),
        ),
        (
            "what does this screenshot say".to_string(),
            "{\"i\":\"ocr\",\"c\":\"ask\",\"q\":\"what does this screenshot say\"}".to_string(),
        ),
        (
            "set the room to 22 degrees".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"set_room_temperature\",\"hsl\":{\"celsius\":22}}".to_string(),
        ),
        (
            "send up extra towels".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"request_extra_towels\",\"hsl\":{\"count\":2}}".to_string(),
        ),
        (
            "wake me at 7am".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"set_wake_up_call\",\"hsl\":{\"time\":\"07:00\"}}".to_string(),
        ),
        (
            "book a taxi to the airport at 4pm".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"book_taxi\",\"hsl\":{\"destination\":\"airport\",\"time\":\"16:00\"}}".to_string(),
        ),
        (
            "order a club sandwich and still water".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"order_room_service\",\"hsl\":{\"items\":[\"club sandwich\",\"still water\"]}}".to_string(),
        ),
        (
            "I locked myself out".to_string(),
            "{\"i\":\"hotel\",\"c\":\"request\",\"hik\":\"report_complaint\",\"hsl\":{\"category\":\"lockout\",\"description\":\"locked out of room\"}}".to_string(),
        ),
    ]
}

/// Canonical few-shot examples filtered to available skills.
///
/// Always returns chat-unrelated examples only for enabled skills. If filtering
/// yields no entries, this falls back to the full canonical few-shot set.
pub fn intent_classifier_few_shots_for_skills(available_skills: &[&str]) -> Vec<(String, String)> {
    let all = intent_classifier_few_shots();
    let short_names: Vec<&str> = available_skills
        .iter()
        .filter(|s| has_skill(&ALL_CLASSIFIER_SKILLS, s))
        .map(|s| short_intent_name(s))
        .collect();
    let mut filtered = all
        .iter()
        .filter(|(_, assistant)| {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(assistant.as_str()) else {
                return false;
            };
            let Some(intent) = value.get("i").and_then(serde_json::Value::as_str) else {
                return false;
            };
            short_names.contains(&intent)
        })
        .cloned()
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        return all;
    }
    filtered.shrink_to_fit();
    filtered
}

#[cfg(test)]
mod tests {
    use super::{
        intent_classifier_few_shots, intent_classifier_few_shots_for_skills,
        intent_classifier_json_schema_for_skills, intent_classifier_system_prompt,
        intent_classifier_system_prompt_for_skills,
    };

    #[test]
    fn prompt_lists_message_disambiguation() {
        let prompt = intent_classifier_system_prompt();
        assert!(prompt.contains("Intents: "));
        assert!(prompt.contains("msg: c=send t! v?"));
        assert!(prompt.contains("rewrite indirect phrasing"));
    }

    #[test]
    fn prompt_does_not_mention_deprecated_assistant_or_memory_skills() {
        let prompt = intent_classifier_system_prompt();
        assert!(
            !prompt.contains("\"assist\""),
            "assistant short name must not appear"
        );
        assert!(
            !prompt.contains("\"mem\""),
            "memory short name must not appear"
        );
        assert!(!prompt.contains("skill_assistant"));
        assert!(!prompt.contains("skill_memory"));
    }

    #[test]
    fn prompt_lists_new_skill_short_names() {
        let prompt = intent_classifier_system_prompt();
        for short in [
            "calc", "unit", "fx", "air", "dict", "tx", "cal", "mtg", "email", "brief", "jrnl",
            "ocr",
        ] {
            assert!(
                prompt.contains(&format!("\"{short}\"")),
                "expected short name '{short}' in intents enum"
            );
        }
    }

    #[test]
    fn prompt_includes_new_skill_rule_lines() {
        let prompt = intent_classifier_system_prompt();
        assert!(prompt.contains("calc: c=eval q!"));
        assert!(prompt.contains("unit: c=convert"));
        assert!(prompt.contains("fx: c=convert"));
        assert!(prompt.contains("air: c=get l?"));
        assert!(prompt.contains("dict: c=define q!"));
        assert!(prompt.contains("tx: c=translate q! tsl? ttl!"));
        assert!(prompt.contains("cal: c=list_today|list_tomorrow|list_upcoming|create"));
        assert!(prompt.contains("mtg: c=summarize q!"));
        assert!(prompt.contains("email: c=list_unread|list_inbox|search|triage"));
        assert!(prompt.contains("brief: c=get bi?"));
        assert!(prompt.contains("jrnl: c=add|recall|stats"));
        assert!(prompt.contains("ocr: c=ask q? sf?"));
    }

    #[test]
    fn prompt_includes_translate_target_language_disambiguation() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_translate"]);
        assert!(prompt.contains("tx requires explicit ttl"));
    }

    #[test]
    fn prompt_includes_calc_unit_fx_disambiguation_when_multiple_present() {
        let prompt = intent_classifier_system_prompt_for_skills(&[
            "skill_calculator",
            "skill_unit_conversion",
            "skill_currency",
        ]);
        assert!(prompt.contains("calc=arithmetic"));
        assert!(prompt.contains("unit=X to Y units"));
        assert!(prompt.contains("fx=X to Y currencies"));
    }

    #[test]
    fn prompt_includes_meeting_notes_transcript_requirement() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_meeting_notes"]);
        assert!(prompt.contains("mtg requires q (transcript text)"));
    }

    #[test]
    fn prompt_includes_command_rules_for_skills() {
        let prompt = intent_classifier_system_prompt();
        assert!(
            prompt.contains("vol: c=set|up|down|mute|unmute|get"),
            "expected volume command rules"
        );
        assert!(
            prompt.contains("Non-chat: add \"c\""),
            "expected command instruction"
        );
    }

    #[test]
    fn prompt_can_be_scoped_to_available_skills() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_time", "skill_timer"]);
        assert!(prompt.contains("Intents: "));
        assert!(prompt.contains("\"chat\""));
        assert!(prompt.contains("\"time\""));
        assert!(prompt.contains("\"timer\""));
        assert!(!prompt.contains("\"weather\""));
        assert!(prompt.contains("time: c=get"));
        assert!(prompt.contains("timer: c=set v!"));
        assert!(!prompt.contains("weather:"));
    }

    #[test]
    fn prompt_rules_only_include_enabled_skills() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_time", "skill_timer"]);
        assert!(prompt.contains("l?"), "expected time location field");
        assert!(prompt.contains("v!"), "expected timer value field");
        assert!(!prompt.contains(" q?"), "sports query should be absent");
        assert!(!prompt.contains(" t?"), "target field should be absent");
        assert!(!prompt.contains("vl?"), "volume field should be absent");
    }

    #[test]
    fn prompt_can_include_new_core_common_skills() {
        let prompt = intent_classifier_system_prompt_for_skills(&[
            "skill_sports_live",
            "skill_holiday_lookup",
            "skill_fuel_price_lookup",
            "skill_horoscope_daily",
            "skill_news_headlines",
        ]);
        assert!(prompt.contains("\"sports\""));
        assert!(prompt.contains("\"holiday\""));
        assert!(prompt.contains("\"fuel\""));
        assert!(prompt.contains("\"horoscope\""));
        assert!(prompt.contains("\"news\""));
        assert!(prompt.contains("sports: c=get"));
        assert!(prompt.contains("holiday: c=get"));
        assert!(prompt.contains("fuel: c=get"));
        assert!(prompt.contains("horoscope: c=get"));
        assert!(prompt.contains("news: c=get"));
    }

    #[test]
    fn few_shots_cover_message_send_contract() {
        let few_shots = intent_classifier_few_shots();
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("ask my wife how she is")
                && a.contains("\"i\":\"msg\"")
                && a.contains("\"c\":\"send\"")
                && a.contains("\"t\":\"my wife\"")
                && a.contains("\"v\":\"How are you?\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("send a message to my wife.")
                && a.contains("\"i\":\"msg\"")
                && a.contains("\"c\":\"send\"")
                && a.contains("\"t\":\"my wife\"")
                && !a.contains("\"v\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("remind me in 5 minutes to ask my wife how she is")
                && a.contains("\"i\":\"reminder\"")
                && a.contains("\"c\":\"add\"")
                && a.contains("\"w\":\"PT5M\"")
        }));
    }

    #[test]
    fn few_shots_include_chat_negative_example() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.iter().any(|(_, a)| a.contains("\"i\":\"chat\"")),
            "expected a chat negative example in few-shots"
        );
    }

    #[test]
    fn prompt_includes_media_vs_smart_home_restriction_when_both_present() {
        let prompt =
            intent_classifier_system_prompt_for_skills(&["skill_media", "skill_smart_home"]);
        assert!(
            prompt.contains("media=music/audio"),
            "expected media/smart-home domain separation in prompt"
        );
        assert!(
            prompt.contains("shome=lights/devices"),
            "expected smart-home domain description"
        );
    }

    #[test]
    fn prompt_omits_media_smart_home_restriction_when_only_one_present() {
        let prompt_media_only = intent_classifier_system_prompt_for_skills(&["skill_media"]);
        assert!(!prompt_media_only.contains("shome=lights"));

        let prompt_smart_home_only =
            intent_classifier_system_prompt_for_skills(&["skill_smart_home"]);
        assert!(!prompt_smart_home_only.contains("media=music"));
    }

    #[test]
    fn prompt_includes_media_vs_app_switcher_restriction_when_both_present() {
        let prompt =
            intent_classifier_system_prompt_for_skills(&["skill_media", "skill_app_switcher"]);
        assert!(
            prompt.contains("media next/prev=audio track"),
            "expected media/app-switcher next/previous disambiguation in prompt"
        );
        assert!(
            prompt.contains("aswitch next/prev=app/window"),
            "expected app-switcher description in disambiguation rule"
        );
    }

    #[test]
    fn prompt_includes_resume_vs_play_disambiguation() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_media"]);
        assert!(
            prompt.contains("resume=unpause/continue"),
            "expected play-vs-resume disambiguation in media rules"
        );
    }

    #[test]
    fn few_shots_cover_media_resume_boundary() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("unpause")
                    && a.contains("\"i\":\"media\"")
                    && a.contains("\"c\":\"resume\"")
            }),
            "expected 'unpause' -> media resume few-shot"
        );
    }

    #[test]
    fn few_shots_cover_smart_home_domain_boundary() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("turn on the kitchen lights")
                    && a.contains("\"i\":\"shome\"")
                    && a.contains("\"c\":\"on\"")
                    && a.contains("\"t\":\"kitchen lights\"")
            }),
            "expected smart-home lights few-shot"
        );
    }

    #[test]
    fn few_shots_cover_app_switcher_domain_boundary() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("switch to the next app")
                    && a.contains("\"i\":\"aswitch\"")
                    && a.contains("\"c\":\"next\"")
            }),
            "expected app-switcher next few-shot"
        );
    }

    #[test]
    fn few_shots_count_is_compact() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.len() <= 30,
            "few-shot set should stay compact; got {} examples",
            few_shots.len()
        );
    }

    #[test]
    fn few_shots_cover_new_skill_boundaries() {
        let few_shots = intent_classifier_few_shots();
        for short in [
            "calc", "unit", "fx", "air", "dict", "tx", "cal", "mtg", "email", "brief", "jrnl",
            "ocr",
        ] {
            let needle = format!("\"i\":\"{short}\"");
            assert!(
                few_shots.iter().any(|(_, a)| a.contains(&needle)),
                "expected at least one few-shot for short intent '{short}'"
            );
        }
    }

    #[test]
    fn few_shots_can_be_filtered_to_available_skills() {
        let filtered = intent_classifier_few_shots_for_skills(&["skill_time", "skill_timer"]);
        assert!(!filtered.is_empty());
        assert!(filtered.iter().all(|(_, assistant)| {
            assistant.contains("\"i\":\"time\"") || assistant.contains("\"i\":\"timer\"")
        }));
    }

    #[test]
    fn few_shot_filter_falls_back_to_full_when_no_match() {
        let filtered = intent_classifier_few_shots_for_skills(&["skill_not_real"]);
        let full = intent_classifier_few_shots();
        assert_eq!(filtered, full);
    }

    #[test]
    fn json_schema_constrains_intent_enum_to_enabled_skills() {
        let schema = intent_classifier_json_schema_for_skills(&["skill_time", "skill_timer"]);
        let intent_prop = &schema["properties"]["i"];
        let allowed: Vec<&str> = intent_prop["enum"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(allowed.contains(&"chat"), "chat must always be in enum");
        assert!(allowed.contains(&"time"));
        assert!(allowed.contains(&"timer"));
        assert!(
            !allowed.contains(&"weather"),
            "disabled skill must be absent"
        );
    }

    #[test]
    fn json_schema_only_includes_fields_for_enabled_skills() {
        let schema = intent_classifier_json_schema_for_skills(&["skill_time"]);
        let props = schema["properties"]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(props.contains(&"i".to_string()));
        assert!(props.contains(&"c".to_string()));
        assert!(props.contains(&"l".to_string()));
        assert!(!props.contains(&"q".to_string()));
        assert!(!props.contains(&"t".to_string()));
    }

    #[test]
    fn json_schema_marks_additional_properties_false() {
        let schema = intent_classifier_json_schema_for_skills(&["skill_time"]);
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn json_schema_requires_intent_field() {
        let schema = intent_classifier_json_schema_for_skills(&["skill_time"]);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(required.contains(&"i"));
    }

    #[test]
    fn system_prompt_is_deterministic_for_kv_cache_prefix_reuse() {
        let skills = &["skill_weather", "skill_time", "skill_media", "skill_timer"];
        let a = intent_classifier_system_prompt_for_skills(skills);
        let b = intent_classifier_system_prompt_for_skills(skills);
        assert_eq!(
            a, b,
            "prompt must be byte-identical across calls for KV cache prefix reuse"
        );

        let schema_a = intent_classifier_json_schema_for_skills(skills);
        let schema_b = intent_classifier_json_schema_for_skills(skills);
        assert_eq!(
            schema_a, schema_b,
            "JSON schema must be identical across calls"
        );

        let fs_a = intent_classifier_few_shots_for_skills(skills);
        let fs_b = intent_classifier_few_shots_for_skills(skills);
        assert_eq!(fs_a, fs_b, "few-shots must be identical across calls");
    }

    #[test]
    fn prompt_lists_hotel_skill_when_enabled() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_hotel"]);
        assert!(prompt.contains("\"hotel\""));
        assert!(prompt.contains("hotel: c=request hik="));
        assert!(prompt.contains("set_room_temperature"));
        assert!(prompt.contains("language_help"));
        assert!(prompt.contains("hotel: in-room concierge"));
    }

    #[test]
    fn json_schema_constrains_hotel_intent_kind_enum() {
        let schema = intent_classifier_json_schema_for_skills(&["skill_hotel"]);
        let hik = &schema["properties"]["hik"];
        assert_eq!(hik["type"], serde_json::json!("string"));
        let kinds: Vec<&str> = hik["enum"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(kinds.len(), 25);
        assert!(kinds.contains(&"set_room_temperature"));
        assert!(kinds.contains(&"language_help"));
        let hsl = &schema["properties"]["hsl"];
        assert_eq!(hsl["type"], serde_json::json!("object"));
        assert_eq!(hsl["additionalProperties"], serde_json::json!(true));
    }

    #[test]
    fn few_shots_cover_canonical_hotel_utterances() {
        let few_shots = intent_classifier_few_shots();
        for needle in [
            "set_room_temperature",
            "request_extra_towels",
            "set_wake_up_call",
            "book_taxi",
            "order_room_service",
        ] {
            assert!(
                few_shots
                    .iter()
                    .any(|(_, a)| a.contains(needle) && a.contains("\"i\":\"hotel\"")),
                "expected hotel few-shot for '{needle}'"
            );
        }
    }

    #[test]
    fn compact_prompt_is_significantly_smaller_than_full_skill_set() {
        let prompt = intent_classifier_system_prompt();
        assert!(
            prompt.len() < 3500,
            "compact prompt should stay under 3500 chars; got {} chars",
            prompt.len()
        );
    }
}
