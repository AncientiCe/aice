//! LLM-based intent classification: known skills vs chat.

use serde::Deserialize;
use std::fmt;

fn deserialize_optional_string_or_array<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Array(items) => {
            let values: Vec<String> = items
                .into_iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(s) => {
                        let t = s.trim().to_string();
                        if t.is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                    serde_json::Value::Null => None,
                    other => {
                        let t = other.to_string();
                        if t.trim().is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                })
                .collect();
            values.join(", ")
        }
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    if normalized.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn deserialize_optional_string_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let collected: Vec<String> = match value {
        serde_json::Value::String(s) => s
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| match item {
                serde_json::Value::String(s) => {
                    let t = s.trim().to_string();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                }
                serde_json::Value::Null => None,
                other => {
                    let t = other.to_string();
                    if t.trim().is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                }
            })
            .collect(),
        serde_json::Value::Null => Vec::new(),
        other => {
            let t = other.to_string();
            if t.trim().is_empty() {
                Vec::new()
            } else {
                vec![t]
            }
        }
    };
    if collected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(collected))
    }
}

/// Result of classifying user input (from LLM, parsed from JSON).
///
/// Note: `PartialEq` (without `Eq`) because `SkillUnitConversion` and `SkillCurrency`
/// carry `Option<f64>` payloads and `f64` does not implement `Eq` (NaN inequality).
#[derive(Clone, Debug, PartialEq)]
pub enum IntentDecision {
    /// Normal chat / question / unknown; use main chat flow.
    Chat,
    /// Weather skill; optional location override (e.g. "Rome").
    SkillWeather { location: Option<String> },
    /// Time skill; optional location (if absent, use current/default location).
    SkillTime { location: Option<String> },
    /// Distance skill; origin and/or destination. If only one place, it is destination (origin = current).
    SkillDistance {
        origin: Option<String>,
        destination: Option<String>,
    },
    /// Sports live info by matchup/team query and optional date.
    SkillSportsLive {
        query: Option<String>,
        date: Option<String>,
    },
    /// Holiday lookup by optional name/date and country/region/year filters.
    SkillHolidayLookup {
        name: Option<String>,
        date: Option<String>,
        country_code: Option<String>,
        region_code: Option<String>,
        year: Option<i32>,
    },
    /// Fuel price lookup by country, optional region, and optional fuel type.
    SkillFuelPriceLookup {
        country_code: Option<String>,
        region: Option<String>,
        fuel_type: Option<String>,
    },
    /// Daily horoscope by sign and optional date.
    SkillHoroscopeDaily {
        sign: Option<String>,
        date: Option<String>,
    },
    /// News headlines by topic and optional country/limit filters.
    SkillNewsHeadlines {
        topic: Option<String>,
        country_code: Option<String>,
        limit: Option<usize>,
    },
    /// Smart home: lights, climate, scenes, device control.
    SkillSmartHome {
        target: Option<String>,
        action: Option<String>,
    },
    /// Media: playback, multi-room, source selection.
    SkillMedia {
        action: Option<String>,
        target: Option<String>,
    },
    /// Calculator: arithmetic / mathematical expression evaluation.
    SkillCalculator { expression: Option<String> },
    /// Unit conversion: free-form query plus optional structured value/units.
    SkillUnitConversion {
        query: Option<String>,
        value: Option<f64>,
        from_unit: Option<String>,
        to_unit: Option<String>,
    },
    /// Currency conversion: optional amount, source and target ISO 4217 codes.
    SkillCurrency {
        amount: Option<f64>,
        from_currency: Option<String>,
        to_currency: Option<String>,
    },
    /// Air quality lookup at an optional location (defaults to current location).
    SkillAirQuality { location: Option<String> },
    /// Dictionary lookup of an English word.
    SkillDictionary { word: Option<String> },
    /// Translate a piece of text from an optional source language to a required target language.
    SkillTranslate {
        text: Option<String>,
        source_language: Option<String>,
        target_language: Option<String>,
    },
    /// Calendar (Google or Apple): list events or create a new event.
    SkillCalendar {
        action: Option<String>,
        title: Option<String>,
        when: Option<String>,
        days: Option<u32>,
        location: Option<String>,
        calendar_name: Option<String>,
    },
    /// Meeting notes: summarize/extract action items from a transcript.
    SkillMeetingNotes {
        transcript: Option<String>,
        title: Option<String>,
        create_reminders: Option<bool>,
    },
    /// Email (IMAP or Apple Mail): list unread/inbox, search, or triage.
    SkillEmail {
        action: Option<String>,
        query: Option<String>,
        limit: Option<usize>,
        mailbox: Option<String>,
    },
    /// Daily briefing: composed weather + calendar + email + news section opt-ins.
    SkillBriefing {
        include: Option<Vec<String>>,
        news_topic: Option<String>,
        news_country: Option<String>,
    },
    /// Personal journal: add an entry, recall entries by query, or get stats.
    SkillJournal {
        action: Option<String>,
        text: Option<String>,
        sentiment: Option<String>,
        tags: Option<Vec<String>>,
        query: Option<String>,
        limit: Option<usize>,
    },
    /// Screen OCR + LLM Q&A: capture (frontend) → OCR (frontend) → answer (backend).
    SkillScreenOcr {
        question: Option<String>,
        filename: Option<String>,
    },
    /// Computer-use: browser, apps, files.
    SkillComputer {
        action: Option<String>,
        target: Option<String>,
    },
    /// Screenshot: save a local screenshot file.
    SkillScreenshot { filename: Option<String> },
    /// App switcher: switch apps, hide, quit, and force-quit actions.
    SkillAppSwitcher {
        action: Option<String>,
        target: Option<String>,
    },
    /// Reminder: create a macOS Reminders entry; title required, when is optional ISO date-time.
    SkillReminder {
        title: Option<String>,
        when: Option<String>,
    },
    /// Timer: start a macOS Clock timer; duration required, name is optional.
    SkillTimer {
        duration: Option<String>,
        name: Option<String>,
    },
    /// Shopping list: add or remove items in an Apple Notes shopping list for a given date.
    SkillShoppingList {
        action: Option<String>,
        items: Option<String>,
        when: Option<String>,
    },
    /// Message: send an iMessage to a contact.
    SkillMessage {
        command: Option<String>,
        contact: Option<String>,
        message: Option<String>,
    },
    /// Volume: set, adjust, mute/unmute, or query system output volume.
    SkillVolume {
        action: Option<String>,
        level: Option<u8>,
    },
    /// Hotel concierge: structured per-property request executed by a Hotel MCP transport.
    SkillHotel {
        intent_kind: Option<String>,
        slots: Option<serde_json::Value>,
    },
}

/// Parse LLM classifier output into IntentDecision. Expects a single JSON object.
/// Tolerates surrounding whitespace and optional markdown code fence.
pub fn parse_intent(raw: &str) -> Result<IntentDecision, ParseIntentError> {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s)
        .strip_suffix("```")
        .unwrap_or(s)
        .trim();
    #[derive(serde::Deserialize)]
    struct Payload {
        #[serde(alias = "i")]
        intent: String,
        #[serde(default, alias = "c")]
        command: Option<String>,
        #[serde(default, alias = "l")]
        location: Option<String>,
        #[serde(default, alias = "o")]
        origin: Option<String>,
        #[serde(default, alias = "d")]
        destination: Option<String>,
        #[serde(default)]
        sports_query: Option<String>,
        #[serde(default)]
        sports_date: Option<String>,
        #[serde(default)]
        holiday_name: Option<String>,
        #[serde(default)]
        holiday_date: Option<String>,
        #[serde(default)]
        holiday_country_code: Option<String>,
        #[serde(default)]
        holiday_region_code: Option<String>,
        #[serde(default)]
        holiday_year: Option<i32>,
        #[serde(default)]
        fuel_country_code: Option<String>,
        #[serde(default)]
        fuel_region: Option<String>,
        #[serde(default)]
        fuel_type: Option<String>,
        #[serde(default)]
        horoscope_sign: Option<String>,
        #[serde(default)]
        horoscope_date: Option<String>,
        #[serde(default)]
        news_topic: Option<String>,
        #[serde(default)]
        news_country_code: Option<String>,
        #[serde(default)]
        news_limit: Option<usize>,
        #[serde(default)]
        smart_home_target: Option<String>,
        #[serde(default)]
        smart_home_action: Option<String>,
        #[serde(default)]
        media_action: Option<String>,
        #[serde(default)]
        media_target: Option<String>,
        // --- new skill payload long-form fields ---
        #[serde(default)]
        expression: Option<String>,
        #[serde(default)]
        unit_query: Option<String>,
        #[serde(default)]
        unit_value: Option<f64>,
        #[serde(default)]
        unit_from: Option<String>,
        #[serde(default)]
        unit_to: Option<String>,
        #[serde(default)]
        currency_amount: Option<f64>,
        #[serde(default)]
        currency_from: Option<String>,
        #[serde(default)]
        currency_to: Option<String>,
        #[serde(default)]
        air_quality_location: Option<String>,
        #[serde(default)]
        dictionary_word: Option<String>,
        #[serde(default)]
        translate_text: Option<String>,
        #[serde(default)]
        translate_source_language: Option<String>,
        #[serde(default)]
        translate_target_language: Option<String>,
        #[serde(default)]
        calendar_action: Option<String>,
        #[serde(default)]
        calendar_title: Option<String>,
        #[serde(default)]
        calendar_when: Option<String>,
        #[serde(default)]
        calendar_days: Option<u32>,
        #[serde(default)]
        calendar_location: Option<String>,
        #[serde(default)]
        calendar_name: Option<String>,
        #[serde(default)]
        meeting_transcript: Option<String>,
        #[serde(default)]
        meeting_title: Option<String>,
        #[serde(default)]
        meeting_create_reminders: Option<bool>,
        #[serde(default)]
        email_action: Option<String>,
        #[serde(default)]
        email_query: Option<String>,
        #[serde(default)]
        email_limit: Option<usize>,
        #[serde(default)]
        email_mailbox: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_vec")]
        briefing_include: Option<Vec<String>>,
        #[serde(default)]
        briefing_news_topic: Option<String>,
        #[serde(default)]
        briefing_news_country: Option<String>,
        #[serde(default)]
        journal_action: Option<String>,
        #[serde(default)]
        journal_text: Option<String>,
        #[serde(default)]
        journal_sentiment: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_vec")]
        journal_tags: Option<Vec<String>>,
        #[serde(default)]
        journal_query: Option<String>,
        #[serde(default)]
        journal_limit: Option<usize>,
        #[serde(default)]
        ocr_question: Option<String>,
        #[serde(default)]
        ocr_filename: Option<String>,
        #[serde(default)]
        computer_action: Option<String>,
        #[serde(default)]
        computer_target: Option<String>,
        #[serde(default)]
        screenshot_filename: Option<String>,
        #[serde(default)]
        app_switcher_action: Option<String>,
        #[serde(default)]
        app_switcher_target: Option<String>,
        #[serde(default)]
        reminder_title: Option<String>,
        #[serde(default)]
        reminder_when: Option<String>,
        #[serde(default)]
        timer_duration: Option<String>,
        #[serde(default)]
        timer_name: Option<String>,
        #[serde(default)]
        shopping_action: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_or_array")]
        shopping_items: Option<String>,
        #[serde(default)]
        shopping_when: Option<String>,
        #[serde(default)]
        message_contact: Option<String>,
        #[serde(default)]
        message_text: Option<String>,
        #[serde(default)]
        volume_action: Option<String>,
        #[serde(default)]
        volume_level: Option<u8>,
        // Compact generic short-key fields (shared across skills)
        #[serde(default)]
        t: Option<String>,
        #[serde(default)]
        q: Option<String>,
        #[serde(default)]
        n: Option<String>,
        #[serde(default)]
        w: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_or_array")]
        p: Option<String>,
        #[serde(default)]
        v: Option<String>,
        #[serde(default)]
        vl: Option<u8>,
        #[serde(default)]
        sf: Option<String>,
        #[serde(default)]
        hs: Option<String>,
        #[serde(default)]
        hcc: Option<String>,
        #[serde(default)]
        hrc: Option<String>,
        #[serde(default)]
        hy: Option<i32>,
        #[serde(default)]
        fcc: Option<String>,
        #[serde(default)]
        fr: Option<String>,
        #[serde(default)]
        ft: Option<String>,
        #[serde(default)]
        ncc: Option<String>,
        #[serde(default)]
        nl: Option<usize>,
        // --- new skill payload short keys ---
        #[serde(default)]
        uv: Option<f64>,
        #[serde(default)]
        uf: Option<String>,
        #[serde(default)]
        ut: Option<String>,
        #[serde(default)]
        cam: Option<f64>,
        #[serde(default)]
        cf: Option<String>,
        #[serde(default)]
        ct: Option<String>,
        #[serde(default)]
        tsl: Option<String>,
        #[serde(default)]
        ttl: Option<String>,
        #[serde(default)]
        cdy: Option<u32>,
        #[serde(default)]
        ccn: Option<String>,
        #[serde(default)]
        mr: Option<bool>,
        #[serde(default)]
        el: Option<usize>,
        #[serde(default)]
        em: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_vec")]
        bi: Option<Vec<String>>,
        #[serde(default)]
        bnt: Option<String>,
        #[serde(default)]
        bnc: Option<String>,
        #[serde(default)]
        js: Option<String>,
        #[serde(default, deserialize_with = "deserialize_optional_string_vec")]
        jt: Option<Vec<String>>,
        #[serde(default)]
        jl: Option<usize>,
        // --- hotel concierge ---
        #[serde(default)]
        hotel_intent_kind: Option<String>,
        #[serde(default)]
        hotel_slots: Option<serde_json::Value>,
        #[serde(default)]
        hik: Option<String>,
        #[serde(default)]
        hsl: Option<serde_json::Value>,
    }
    let p: Payload = serde_json::from_str(s).map_err(ParseIntentError::Json)?;
    let intent = p.intent.to_lowercase().trim().to_string();
    let opt_str = |o: Option<String>| o.filter(|x| !x.trim().is_empty());
    let or_opt = |a: Option<String>, b: Option<String>| opt_str(a).or_else(|| opt_str(b));
    let command = opt_str(p.command);
    let action_from = |field: Option<String>, fallback: &Option<String>| {
        opt_str(field).or_else(|| fallback.clone())
    };
    let generic_t = opt_str(p.t);
    let generic_q = opt_str(p.q);
    let generic_n = opt_str(p.n);
    let generic_w = opt_str(p.w);
    let generic_v = opt_str(p.v);

    match intent.as_str() {
        "chat" => Ok(IntentDecision::Chat),
        "skill_weather" | "weather" => Ok(IntentDecision::SkillWeather {
            location: or_opt(p.location, generic_t.clone()),
        }),
        "skill_time" | "time" => Ok(IntentDecision::SkillTime {
            location: or_opt(p.location, generic_t.clone()),
        }),
        "skill_distance" | "distance" | "dist" => Ok(IntentDecision::SkillDistance {
            origin: opt_str(p.origin),
            destination: opt_str(p.destination),
        }),
        "skill_sports_live" | "sports_live" | "sports" => Ok(IntentDecision::SkillSportsLive {
            query: or_opt(p.sports_query, generic_q.clone()),
            date: or_opt(p.sports_date, generic_w.clone()),
        }),
        "skill_holiday_lookup" | "holiday_lookup" | "holiday" => {
            Ok(IntentDecision::SkillHolidayLookup {
                name: or_opt(p.holiday_name, generic_n.clone()),
                date: or_opt(p.holiday_date, generic_w.clone()),
                country_code: or_opt(p.holiday_country_code, opt_str(p.hcc)),
                region_code: or_opt(p.holiday_region_code, opt_str(p.hrc)),
                year: p.holiday_year.or(p.hy),
            })
        }
        "skill_fuel_price_lookup" | "fuel_price_lookup" | "fuel" => {
            Ok(IntentDecision::SkillFuelPriceLookup {
                country_code: or_opt(p.fuel_country_code, opt_str(p.fcc)),
                region: or_opt(p.fuel_region, opt_str(p.fr)),
                fuel_type: or_opt(p.fuel_type, opt_str(p.ft)),
            })
        }
        "skill_horoscope_daily" | "horoscope_daily" | "horoscope" => {
            Ok(IntentDecision::SkillHoroscopeDaily {
                sign: or_opt(p.horoscope_sign, opt_str(p.hs)),
                date: or_opt(p.horoscope_date, generic_w.clone()),
            })
        }
        "skill_news_headlines" | "news_headlines" | "news" => {
            Ok(IntentDecision::SkillNewsHeadlines {
                topic: or_opt(p.news_topic, generic_q.clone()),
                country_code: or_opt(p.news_country_code, opt_str(p.ncc)),
                limit: p.news_limit.or(p.nl),
            })
        }
        "skill_smart_home" | "smart_home" | "shome" => Ok(IntentDecision::SkillSmartHome {
            target: or_opt(p.smart_home_target, generic_t.clone()),
            action: action_from(p.smart_home_action, &command),
        }),
        "skill_media" | "media" => Ok(IntentDecision::SkillMedia {
            action: action_from(p.media_action, &command),
            target: or_opt(p.media_target, generic_t.clone()),
        }),
        "skill_calculator" | "calculator" | "calc" => Ok(IntentDecision::SkillCalculator {
            expression: or_opt(p.expression, generic_q.clone()),
        }),
        "skill_unit_conversion" | "unit_conversion" | "unit" => {
            Ok(IntentDecision::SkillUnitConversion {
                query: or_opt(p.unit_query, generic_q.clone()),
                value: p.unit_value.or(p.uv),
                from_unit: or_opt(p.unit_from, opt_str(p.uf)),
                to_unit: or_opt(p.unit_to, opt_str(p.ut)),
            })
        }
        "skill_currency" | "currency" | "fx" => Ok(IntentDecision::SkillCurrency {
            amount: p.currency_amount.or(p.cam),
            from_currency: or_opt(p.currency_from, opt_str(p.cf)),
            to_currency: or_opt(p.currency_to, opt_str(p.ct)),
        }),
        "skill_air_quality" | "air_quality" | "air" => Ok(IntentDecision::SkillAirQuality {
            location: or_opt(
                p.air_quality_location,
                or_opt(p.location, generic_t.clone()),
            ),
        }),
        "skill_dictionary" | "dictionary" | "dict" => Ok(IntentDecision::SkillDictionary {
            word: or_opt(p.dictionary_word, generic_q.clone()),
        }),
        "skill_translate" | "translate" | "tx" => Ok(IntentDecision::SkillTranslate {
            text: or_opt(p.translate_text, generic_q.clone()),
            source_language: or_opt(p.translate_source_language, opt_str(p.tsl)),
            target_language: or_opt(p.translate_target_language, opt_str(p.ttl)),
        }),
        "skill_calendar" | "calendar" | "cal" => Ok(IntentDecision::SkillCalendar {
            action: action_from(p.calendar_action, &command),
            title: or_opt(p.calendar_title, generic_n.clone()),
            when: or_opt(p.calendar_when, generic_w.clone()),
            days: p.calendar_days.or(p.cdy),
            location: or_opt(p.calendar_location.clone(), p.location.clone()),
            calendar_name: or_opt(p.calendar_name, opt_str(p.ccn)),
        }),
        "skill_meeting_notes" | "meeting_notes" | "mtg" => Ok(IntentDecision::SkillMeetingNotes {
            transcript: or_opt(p.meeting_transcript, generic_q.clone()),
            title: or_opt(p.meeting_title, generic_n.clone()),
            create_reminders: p.meeting_create_reminders.or(p.mr),
        }),
        "skill_email" | "email" | "mail" => Ok(IntentDecision::SkillEmail {
            action: action_from(p.email_action, &command),
            query: or_opt(p.email_query, generic_q.clone()),
            limit: p.email_limit.or(p.el),
            mailbox: or_opt(p.email_mailbox, opt_str(p.em)),
        }),
        "skill_briefing" | "briefing" | "brief" => Ok(IntentDecision::SkillBriefing {
            include: p.briefing_include.or(p.bi),
            news_topic: or_opt(p.briefing_news_topic, opt_str(p.bnt)),
            news_country: or_opt(p.briefing_news_country, opt_str(p.bnc)),
        }),
        "skill_journal" | "journal" | "jrnl" => Ok(IntentDecision::SkillJournal {
            action: action_from(p.journal_action, &command),
            text: or_opt(p.journal_text, generic_v.clone()),
            sentiment: or_opt(p.journal_sentiment, opt_str(p.js)),
            tags: p.journal_tags.or(p.jt),
            query: or_opt(p.journal_query, generic_q.clone()),
            limit: p.journal_limit.or(p.jl),
        }),
        "skill_screen_ocr" | "screen_ocr" | "ocr" => Ok(IntentDecision::SkillScreenOcr {
            question: or_opt(p.ocr_question, generic_q.clone()),
            filename: or_opt(p.ocr_filename, opt_str(p.sf)),
        }),
        "skill_computer" | "computer" => Ok(IntentDecision::SkillComputer {
            action: action_from(p.computer_action, &command),
            target: or_opt(p.computer_target, generic_t.clone()),
        }),
        "skill_screenshot" | "screenshot" => Ok(IntentDecision::SkillScreenshot {
            filename: or_opt(p.screenshot_filename, opt_str(p.sf)),
        }),
        "skill_app_switcher" | "app_switcher" | "aswitch" => Ok(IntentDecision::SkillAppSwitcher {
            action: action_from(p.app_switcher_action, &command),
            target: or_opt(p.app_switcher_target, generic_t.clone()),
        }),
        "skill_reminder" | "reminder" => Ok(IntentDecision::SkillReminder {
            title: or_opt(p.reminder_title, generic_n.clone()),
            when: or_opt(p.reminder_when, generic_w.clone()),
        }),
        "skill_timer" | "timer" => Ok(IntentDecision::SkillTimer {
            duration: or_opt(p.timer_duration, generic_v.clone()),
            name: or_opt(p.timer_name, generic_n.clone()),
        }),
        "skill_shopping_list" | "shopping_list" | "shop" => Ok(IntentDecision::SkillShoppingList {
            action: action_from(p.shopping_action, &command),
            items: or_opt(p.shopping_items, opt_str(p.p)),
            when: or_opt(p.shopping_when, generic_w.clone()),
        }),
        "skill_message" | "message" | "msg" => Ok(IntentDecision::SkillMessage {
            command: command.clone(),
            contact: or_opt(p.message_contact, generic_t.clone()),
            message: or_opt(p.message_text, generic_v.clone()),
        }),
        "skill_volume" | "volume" | "vol" => Ok(IntentDecision::SkillVolume {
            action: action_from(p.volume_action, &command),
            level: p.volume_level.or(p.vl),
        }),
        "skill_hotel" | "hotel" => Ok(IntentDecision::SkillHotel {
            intent_kind: or_opt(p.hotel_intent_kind, opt_str(p.hik)),
            slots: p.hotel_slots.or(p.hsl),
        }),
        _ => Ok(IntentDecision::Chat),
    }
}

#[derive(Debug)]
pub enum ParseIntentError {
    Json(serde_json::Error),
}

impl fmt::Display for ParseIntentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseIntentError::Json(e) => write!(f, "intent JSON parse error: {}", e),
        }
    }
}

impl std::error::Error for ParseIntentError {}

/// Async classifier: user text -> intent decision (implementations call LLM and parse).
#[async_trait::async_trait]
pub trait IntentClassifier: Send + Sync {
    async fn classify(
        &self,
        user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>>;
}

const HOTEL_INTENT_KINDS: &[&str] = &[
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

const SMART_HOME_ACTIONS: &[&str] = &["on", "off", "toggle", "status", "set"];
const MEDIA_ACTIONS: &[&str] = &[
    "play",
    "pause",
    "resume",
    "next",
    "previous",
    "shuffle_on",
    "shuffle_off",
    "status",
];
const APP_SWITCHER_ACTIONS: &[&str] = &["switch", "next", "previous", "hide", "quit", "force_quit"];
const COMPUTER_ACTIONS: &[&str] = &["open", "launch", "browse", "run"];
const VOLUME_ACTIONS: &[&str] = &["set", "up", "down", "mute", "unmute", "get"];
const SHOPPING_LIST_ACTIONS: &[&str] = &["add", "remove"];
const MESSAGE_COMMANDS: &[&str] = &["send"];
const CALENDAR_ACTIONS: &[&str] = &["list_today", "list_tomorrow", "list_upcoming", "create"];
const EMAIL_ACTIONS: &[&str] = &["list_unread", "list_inbox", "search", "triage"];
const JOURNAL_ACTIONS: &[&str] = &["add", "recall", "stats"];

fn action_allowed(action: &Option<String>, allowlist: &[&str]) -> bool {
    match action.as_deref() {
        None => true,
        Some(a) => allowlist.contains(&a),
    }
}

/// Validate a parsed `IntentDecision` against the published per-skill command allowlists.
///
/// If the model returned a `command` / `action` value that is not in the allowlist for the
/// chosen skill, the model produced internally inconsistent output relative to its own system
/// prompt.  In that case this function logs a warning and returns `IntentDecision::Chat` so
/// the turn falls back gracefully instead of reaching skill execution with an invalid action.
pub fn validate_intent_decision(decision: IntentDecision) -> IntentDecision {
    let invalid = match &decision {
        IntentDecision::SkillSmartHome { action, .. } => {
            !action_allowed(action, SMART_HOME_ACTIONS)
        }
        IntentDecision::SkillMedia { action, .. } => !action_allowed(action, MEDIA_ACTIONS),
        IntentDecision::SkillAppSwitcher { action, .. } => {
            !action_allowed(action, APP_SWITCHER_ACTIONS)
        }
        IntentDecision::SkillComputer { action, .. } => !action_allowed(action, COMPUTER_ACTIONS),
        IntentDecision::SkillVolume { action, .. } => !action_allowed(action, VOLUME_ACTIONS),
        IntentDecision::SkillShoppingList { action, .. } => {
            !action_allowed(action, SHOPPING_LIST_ACTIONS)
        }
        IntentDecision::SkillMessage { command, .. } => !action_allowed(command, MESSAGE_COMMANDS),
        IntentDecision::SkillCalendar { action, .. } => !action_allowed(action, CALENDAR_ACTIONS),
        IntentDecision::SkillEmail { action, .. } => !action_allowed(action, EMAIL_ACTIONS),
        IntentDecision::SkillJournal { action, .. } => !action_allowed(action, JOURNAL_ACTIONS),
        IntentDecision::SkillHotel { intent_kind, .. } => match intent_kind.as_deref() {
            None => true,
            Some(kind) => !HOTEL_INTENT_KINDS.contains(&kind),
        },
        _ => false,
    };
    if invalid {
        let skill = match &decision {
            IntentDecision::SkillSmartHome { action, .. } => {
                format!("skill_smart_home action={:?}", action)
            }
            IntentDecision::SkillMedia { action, .. } => format!("skill_media action={:?}", action),
            IntentDecision::SkillAppSwitcher { action, .. } => {
                format!("skill_app_switcher action={:?}", action)
            }
            IntentDecision::SkillComputer { action, .. } => {
                format!("skill_computer action={:?}", action)
            }
            IntentDecision::SkillVolume { action, .. } => {
                format!("skill_volume action={:?}", action)
            }
            IntentDecision::SkillShoppingList { action, .. } => {
                format!("skill_shopping_list action={:?}", action)
            }
            IntentDecision::SkillMessage { command, .. } => {
                format!("skill_message command={:?}", command)
            }
            IntentDecision::SkillCalendar { action, .. } => {
                format!("skill_calendar action={:?}", action)
            }
            IntentDecision::SkillEmail { action, .. } => {
                format!("skill_email action={:?}", action)
            }
            IntentDecision::SkillJournal { action, .. } => {
                format!("skill_journal action={:?}", action)
            }
            IntentDecision::SkillHotel { intent_kind, .. } => {
                format!("skill_hotel intent_kind={:?}", intent_kind)
            }
            other => format!("{:?}", other),
        };
        tracing::warn!(
            invalid_decision = %skill,
            "intent decision failed contract validation; falling back to chat"
        );
        core_observability::record_intent_validation_rejected(&skill);
        IntentDecision::Chat
    } else {
        decision
    }
}

#[cfg(test)]
mod tests {
    pub trait TestResultExt<T, E> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(value) => value,
                Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
            }
        }
    }

    pub trait TestOptionExt<T> {
        fn must_some(self, message: &str) -> T;
    }

    impl<T> TestOptionExt<T> for Option<T> {
        fn must_some(self, message: &str) -> T {
            match self {
                Some(value) => value,
                None => panic!("expected Some(..) in test: {message}"),
            }
        }
    }
    use super::{parse_intent, validate_intent_decision, IntentDecision};
    #[allow(unused_imports)]
    use TestOptionExt as _;

    #[test]
    fn parse_intent_smart_home() {
        let raw = r#"{"intent": "skill_smart_home", "smart_home_target": "living room", "smart_home_action": "turn off"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillSmartHome { target, action } => {
                assert_eq!(target.as_deref(), Some("living room"));
                assert_eq!(action.as_deref(), Some("turn off"));
            }
            _ => panic!("expected SkillSmartHome"),
        }
    }

    #[test]
    fn parse_intent_media() {
        let raw = r#"{"intent": "skill_media", "media_action": "play", "media_target": "kitchen"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMedia { action, target } => {
                assert_eq!(action.as_deref(), Some("play"));
                assert_eq!(target.as_deref(), Some("kitchen"));
            }
            _ => panic!("expected SkillMedia"),
        }
    }

    #[test]
    fn parse_intent_computer() {
        let raw = r#"{"intent": "skill_computer", "computer_action": "open", "computer_target": "browser"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillComputer { action, target } => {
                assert_eq!(action.as_deref(), Some("open"));
                assert_eq!(target.as_deref(), Some("browser"));
            }
            _ => panic!("expected SkillComputer"),
        }
    }

    #[test]
    fn parse_intent_screenshot() {
        let raw = r#"{"intent":"skill_screenshot","screenshot_filename":"desk.png"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillScreenshot { filename } => {
                assert_eq!(filename.as_deref(), Some("desk.png"));
            }
            _ => panic!("expected SkillScreenshot"),
        }
    }

    #[test]
    fn parse_intent_reminder_with_when() {
        let raw = r#"{"intent": "skill_reminder", "reminder_title": "Call mom", "reminder_when": "2026-03-20T17:00"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillReminder { title, when } => {
                assert_eq!(title.as_deref(), Some("Call mom"));
                assert_eq!(when.as_deref(), Some("2026-03-20T17:00"));
            }
            _ => panic!("expected SkillReminder"),
        }
    }

    #[test]
    fn parse_intent_reminder_without_when() {
        let raw = r#"{"intent": "skill_reminder", "reminder_title": "Buy groceries"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillReminder { title, when } => {
                assert_eq!(title.as_deref(), Some("Buy groceries"));
                assert!(when.is_none());
            }
            _ => panic!("expected SkillReminder"),
        }
    }

    #[test]
    fn parse_intent_timer_with_name() {
        let raw = r#"{"intent": "skill_timer", "timer_duration": "5 minutes", "timer_name": "pasta timer"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTimer { duration, name } => {
                assert_eq!(duration.as_deref(), Some("5 minutes"));
                assert_eq!(name.as_deref(), Some("pasta timer"));
            }
            _ => panic!("expected SkillTimer"),
        }
    }

    #[test]
    fn parse_intent_timer_without_name() {
        let raw = r#"{"intent": "skill_timer", "timer_duration": "30 minutes"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTimer { duration, name } => {
                assert_eq!(duration.as_deref(), Some("30 minutes"));
                assert!(name.is_none());
            }
            _ => panic!("expected SkillTimer"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_add() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": "strawberries, salami", "shopping_when": "today"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("strawberries, salami"));
                assert_eq!(when.as_deref(), Some("today"));
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_no_when_defaults_to_none() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": "milk"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("milk"));
                assert!(when.is_none());
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_shopping_list_items_array_is_normalized() {
        let raw = r#"{"intent": "skill_shopping_list", "shopping_action": "add", "shopping_items": ["strawberries", "salami"], "shopping_when": "today"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("strawberries, salami"));
                assert_eq!(when.as_deref(), Some("today"));
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_intent_message() {
        let raw = r#"{"intent":"skill_message","command":"send","message_contact":"my wife","message_text":"How are you?"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMessage {
                command,
                contact,
                message,
            } => {
                assert_eq!(command.as_deref(), Some("send"));
                assert_eq!(contact.as_deref(), Some("my wife"));
                assert_eq!(message.as_deref(), Some("How are you?"));
            }
            _ => panic!("expected SkillMessage"),
        }
    }

    #[test]
    fn parse_intent_uses_global_command_for_action_skills() {
        let raw = r#"{"intent":"skill_volume","command":"mute"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("mute"));
                assert_eq!(*level, None);
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_volume_set_level() {
        let raw = r#"{"intent":"skill_volume","volume_action":"set","volume_level":40}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("set"));
                assert_eq!(*level, Some(40));
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_volume_without_level() {
        let raw = r#"{"intent":"volume","volume_action":"mute"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("mute"));
                assert_eq!(*level, None);
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_intent_app_switcher_with_target() {
        let raw = r#"{"intent":"skill_app_switcher","app_switcher_action":"switch","app_switcher_target":"Safari"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAppSwitcher { action, target } => {
                assert_eq!(action.as_deref(), Some("switch"));
                assert_eq!(target.as_deref(), Some("Safari"));
            }
            _ => panic!("expected SkillAppSwitcher"),
        }
    }

    #[test]
    fn parse_intent_app_switcher_without_target() {
        let raw = r#"{"intent":"app_switcher","app_switcher_action":"next"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAppSwitcher { action, target } => {
                assert_eq!(action.as_deref(), Some("next"));
                assert!(target.is_none());
            }
            _ => panic!("expected SkillAppSwitcher"),
        }
    }

    #[test]
    fn parse_intent_sports_live() {
        let raw = r#"{"intent":"skill_sports_live","command":"get","sports_query":"lakers vs celtics","sports_date":"2026-04-01"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillSportsLive { query, date } => {
                assert_eq!(query.as_deref(), Some("lakers vs celtics"));
                assert_eq!(date.as_deref(), Some("2026-04-01"));
            }
            _ => panic!("expected SkillSportsLive"),
        }
    }

    #[test]
    fn parse_intent_holiday_lookup() {
        let raw = r#"{"intent":"skill_holiday_lookup","command":"get","holiday_name":"easter","holiday_country_code":"DE","holiday_year":2026}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHolidayLookup {
                name,
                date,
                country_code,
                region_code,
                year,
            } => {
                assert_eq!(name.as_deref(), Some("easter"));
                assert!(date.is_none());
                assert_eq!(country_code.as_deref(), Some("DE"));
                assert!(region_code.is_none());
                assert_eq!(*year, Some(2026));
            }
            _ => panic!("expected SkillHolidayLookup"),
        }
    }

    #[test]
    fn parse_intent_fuel_price_lookup() {
        let raw = r#"{"intent":"skill_fuel_price_lookup","command":"get","fuel_country_code":"GB","fuel_region":"london","fuel_type":"diesel"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillFuelPriceLookup {
                country_code,
                region,
                fuel_type,
            } => {
                assert_eq!(country_code.as_deref(), Some("GB"));
                assert_eq!(region.as_deref(), Some("london"));
                assert_eq!(fuel_type.as_deref(), Some("diesel"));
            }
            _ => panic!("expected SkillFuelPriceLookup"),
        }
    }

    #[test]
    fn parse_intent_horoscope_daily() {
        let raw = r#"{"intent":"skill_horoscope_daily","command":"get","horoscope_sign":"aries","horoscope_date":"2026-04-01"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHoroscopeDaily { sign, date } => {
                assert_eq!(sign.as_deref(), Some("aries"));
                assert_eq!(date.as_deref(), Some("2026-04-01"));
            }
            _ => panic!("expected SkillHoroscopeDaily"),
        }
    }

    #[test]
    fn parse_intent_news_headlines() {
        let raw = r#"{"intent":"skill_news_headlines","command":"get","news_topic":"technology","news_country_code":"US","news_limit":5}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillNewsHeadlines {
                topic,
                country_code,
                limit,
            } => {
                assert_eq!(topic.as_deref(), Some("technology"));
                assert_eq!(country_code.as_deref(), Some("US"));
                assert_eq!(*limit, Some(5));
            }
            _ => panic!("expected SkillNewsHeadlines"),
        }
    }

    // --- new skill parse tests ---

    #[test]
    fn parse_intent_calculator() {
        let raw = r#"{"intent":"skill_calculator","expression":"2 + 2 * 5"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCalculator { expression } => {
                assert_eq!(expression.as_deref(), Some("2 + 2 * 5"));
            }
            _ => panic!("expected SkillCalculator"),
        }
    }

    #[test]
    fn parse_intent_calculator_compact_uses_q() {
        let raw = r#"{"i":"calc","q":"3 * (4 + 1)"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCalculator { expression } => {
                assert_eq!(expression.as_deref(), Some("3 * (4 + 1)"));
            }
            _ => panic!("expected SkillCalculator"),
        }
    }

    #[test]
    fn parse_intent_unit_conversion() {
        let raw =
            r#"{"intent":"skill_unit_conversion","unit_value":10,"unit_from":"kg","unit_to":"lb"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillUnitConversion {
                query,
                value,
                from_unit,
                to_unit,
            } => {
                assert!(query.is_none());
                assert_eq!(*value, Some(10.0));
                assert_eq!(from_unit.as_deref(), Some("kg"));
                assert_eq!(to_unit.as_deref(), Some("lb"));
            }
            _ => panic!("expected SkillUnitConversion"),
        }
    }

    #[test]
    fn parse_intent_unit_conversion_compact() {
        let raw = r#"{"i":"unit","uv":3.5,"uf":"miles","ut":"km"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillUnitConversion {
                value,
                from_unit,
                to_unit,
                ..
            } => {
                assert_eq!(*value, Some(3.5));
                assert_eq!(from_unit.as_deref(), Some("miles"));
                assert_eq!(to_unit.as_deref(), Some("km"));
            }
            _ => panic!("expected SkillUnitConversion"),
        }
    }

    #[test]
    fn parse_intent_currency_with_amount() {
        let raw = r#"{"intent":"skill_currency","currency_amount":100,"currency_from":"USD","currency_to":"EUR"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCurrency {
                amount,
                from_currency,
                to_currency,
            } => {
                assert_eq!(*amount, Some(100.0));
                assert_eq!(from_currency.as_deref(), Some("USD"));
                assert_eq!(to_currency.as_deref(), Some("EUR"));
            }
            _ => panic!("expected SkillCurrency"),
        }
    }

    #[test]
    fn parse_intent_currency_compact() {
        let raw = r#"{"i":"fx","cam":42.5,"cf":"GBP","ct":"JPY"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCurrency {
                amount,
                from_currency,
                to_currency,
            } => {
                assert_eq!(*amount, Some(42.5));
                assert_eq!(from_currency.as_deref(), Some("GBP"));
                assert_eq!(to_currency.as_deref(), Some("JPY"));
            }
            _ => panic!("expected SkillCurrency"),
        }
    }

    #[test]
    fn parse_intent_air_quality_with_location() {
        let raw = r#"{"intent":"skill_air_quality","air_quality_location":"Milan"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAirQuality { location } => {
                assert_eq!(location.as_deref(), Some("Milan"));
            }
            _ => panic!("expected SkillAirQuality"),
        }
    }

    #[test]
    fn parse_intent_air_quality_falls_back_to_generic_l() {
        let raw = r#"{"i":"air","l":"Berlin"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAirQuality { location } => {
                assert_eq!(location.as_deref(), Some("Berlin"));
            }
            _ => panic!("expected SkillAirQuality"),
        }
    }

    #[test]
    fn parse_intent_dictionary() {
        let raw = r#"{"intent":"skill_dictionary","dictionary_word":"serendipity"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillDictionary { word } => {
                assert_eq!(word.as_deref(), Some("serendipity"));
            }
            _ => panic!("expected SkillDictionary"),
        }
    }

    #[test]
    fn parse_intent_translate_short_keys() {
        let raw = r#"{"i":"tx","q":"good morning","tsl":"en","ttl":"it"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTranslate {
                text,
                source_language,
                target_language,
            } => {
                assert_eq!(text.as_deref(), Some("good morning"));
                assert_eq!(source_language.as_deref(), Some("en"));
                assert_eq!(target_language.as_deref(), Some("it"));
            }
            _ => panic!("expected SkillTranslate"),
        }
    }

    #[test]
    fn parse_intent_calendar_list_today() {
        let raw = r#"{"intent":"skill_calendar","calendar_action":"list_today"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCalendar { action, .. } => {
                assert_eq!(action.as_deref(), Some("list_today"));
            }
            _ => panic!("expected SkillCalendar"),
        }
    }

    #[test]
    fn parse_intent_calendar_create_event() {
        let raw = r#"{"intent":"skill_calendar","calendar_action":"create","calendar_title":"Standup","calendar_when":"2026-04-20T10:00","calendar_location":"Office","calendar_name":"Work"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCalendar {
                action,
                title,
                when,
                location,
                calendar_name,
                ..
            } => {
                assert_eq!(action.as_deref(), Some("create"));
                assert_eq!(title.as_deref(), Some("Standup"));
                assert_eq!(when.as_deref(), Some("2026-04-20T10:00"));
                assert_eq!(location.as_deref(), Some("Office"));
                assert_eq!(calendar_name.as_deref(), Some("Work"));
            }
            _ => panic!("expected SkillCalendar"),
        }
    }

    #[test]
    fn parse_intent_calendar_compact_with_days() {
        let raw = r#"{"i":"cal","c":"list_upcoming","cdy":7}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillCalendar { action, days, .. } => {
                assert_eq!(action.as_deref(), Some("list_upcoming"));
                assert_eq!(*days, Some(7));
            }
            _ => panic!("expected SkillCalendar"),
        }
    }

    #[test]
    fn parse_intent_meeting_notes_with_transcript() {
        let raw = r#"{"intent":"skill_meeting_notes","meeting_transcript":"Alice: ship by Friday. Bob: ok.","meeting_title":"sync"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMeetingNotes {
                transcript,
                title,
                create_reminders,
            } => {
                assert_eq!(
                    transcript.as_deref(),
                    Some("Alice: ship by Friday. Bob: ok.")
                );
                assert_eq!(title.as_deref(), Some("sync"));
                assert!(create_reminders.is_none());
            }
            _ => panic!("expected SkillMeetingNotes"),
        }
    }

    #[test]
    fn parse_intent_email_list_unread() {
        let raw = r#"{"intent":"skill_email","email_action":"list_unread","email_limit":5}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillEmail {
                action,
                limit,
                query,
                mailbox,
            } => {
                assert_eq!(action.as_deref(), Some("list_unread"));
                assert_eq!(*limit, Some(5));
                assert!(query.is_none());
                assert!(mailbox.is_none());
            }
            _ => panic!("expected SkillEmail"),
        }
    }

    #[test]
    fn parse_intent_email_search_short_keys() {
        let raw = r#"{"i":"email","c":"search","q":"invoice","el":3,"em":"INBOX"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillEmail {
                action,
                query,
                limit,
                mailbox,
            } => {
                assert_eq!(action.as_deref(), Some("search"));
                assert_eq!(query.as_deref(), Some("invoice"));
                assert_eq!(*limit, Some(3));
                assert_eq!(mailbox.as_deref(), Some("INBOX"));
            }
            _ => panic!("expected SkillEmail"),
        }
    }

    #[test]
    fn parse_intent_briefing_with_include_array() {
        let raw = r#"{"intent":"skill_briefing","briefing_include":["weather","calendar","news"],"briefing_news_topic":"technology"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillBriefing {
                include,
                news_topic,
                ..
            } => {
                let inc = include.as_deref().must_some("include present");
                assert_eq!(inc, &["weather", "calendar", "news"]);
                assert_eq!(news_topic.as_deref(), Some("technology"));
            }
            _ => panic!("expected SkillBriefing"),
        }
    }

    #[test]
    fn parse_intent_briefing_compact_with_csv_string() {
        let raw = r#"{"i":"brief","bi":"weather, email"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillBriefing { include, .. } => {
                let inc = include.as_deref().must_some("include present");
                assert_eq!(inc, &["weather", "email"]);
            }
            _ => panic!("expected SkillBriefing"),
        }
    }

    #[test]
    fn parse_intent_journal_add_with_tags() {
        let raw = r#"{"intent":"skill_journal","journal_action":"add","journal_text":"Great run today","journal_sentiment":"positive","journal_tags":["fitness","morning"]}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillJournal {
                action,
                text,
                sentiment,
                tags,
                ..
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(text.as_deref(), Some("Great run today"));
                assert_eq!(sentiment.as_deref(), Some("positive"));
                let tg = tags.as_deref().must_some("tags present");
                assert_eq!(tg, &["fitness", "morning"]);
            }
            _ => panic!("expected SkillJournal"),
        }
    }

    #[test]
    fn parse_intent_journal_recall_compact() {
        let raw = r#"{"i":"jrnl","c":"recall","q":"vacation","jl":3}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillJournal {
                action,
                query,
                limit,
                ..
            } => {
                assert_eq!(action.as_deref(), Some("recall"));
                assert_eq!(query.as_deref(), Some("vacation"));
                assert_eq!(*limit, Some(3));
            }
            _ => panic!("expected SkillJournal"),
        }
    }

    #[test]
    fn parse_intent_screen_ocr_with_question() {
        let raw = r#"{"intent":"skill_screen_ocr","ocr_question":"What does the modal say?","ocr_filename":"shot.png"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillScreenOcr { question, filename } => {
                assert_eq!(question.as_deref(), Some("What does the modal say?"));
                assert_eq!(filename.as_deref(), Some("shot.png"));
            }
            _ => panic!("expected SkillScreenOcr"),
        }
    }

    #[test]
    fn parse_intent_screen_ocr_compact() {
        let raw = r#"{"i":"ocr","q":"summarize","sf":"capture.png"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillScreenOcr { question, filename } => {
                assert_eq!(question.as_deref(), Some("summarize"));
                assert_eq!(filename.as_deref(), Some("capture.png"));
            }
            _ => panic!("expected SkillScreenOcr"),
        }
    }

    // --- validate_intent_decision tests ---

    #[test]
    fn validation_passes_valid_smart_home_action() {
        for action in ["on", "off", "toggle", "status", "set"] {
            let d = IntentDecision::SkillSmartHome {
                target: None,
                action: Some(action.to_string()),
            };
            assert_eq!(
                validate_intent_decision(d.clone()),
                d,
                "expected valid smart_home action '{action}' to pass"
            );
        }
    }

    #[test]
    fn validation_rejects_invalid_smart_home_action() {
        // The exact failure observed in production logs: command:"next" on skill_smart_home
        let d = IntentDecision::SkillSmartHome {
            target: None,
            action: Some("next".to_string()),
        };
        assert_eq!(
            validate_intent_decision(d),
            IntentDecision::Chat,
            "expected skill_smart_home with action='next' to be rejected"
        );
    }

    #[test]
    fn validation_rejects_invented_smart_home_command() {
        let d = IntentDecision::SkillSmartHome {
            target: None,
            action: Some("play_next_song".to_string()),
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_passes_none_action_for_smart_home() {
        let d = IntentDecision::SkillSmartHome {
            target: None,
            action: None,
        };
        assert_eq!(validate_intent_decision(d.clone()), d);
    }

    #[test]
    fn validation_passes_valid_media_actions() {
        for action in [
            "play",
            "pause",
            "resume",
            "next",
            "previous",
            "shuffle_on",
            "shuffle_off",
            "status",
        ] {
            let d = IntentDecision::SkillMedia {
                action: Some(action.to_string()),
                target: None,
            };
            assert_eq!(
                validate_intent_decision(d.clone()),
                d,
                "expected valid media action '{action}' to pass"
            );
        }
    }

    #[test]
    fn validation_rejects_invalid_media_action() {
        let d = IntentDecision::SkillMedia {
            action: Some("turn_on".to_string()),
            target: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_passes_valid_volume_actions() {
        for action in ["set", "up", "down", "mute", "unmute", "get"] {
            let d = IntentDecision::SkillVolume {
                action: Some(action.to_string()),
                level: None,
            };
            assert_eq!(validate_intent_decision(d.clone()), d);
        }
    }

    #[test]
    fn validation_rejects_invalid_volume_action() {
        let d = IntentDecision::SkillVolume {
            action: Some("louder".to_string()),
            level: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_passes_chat_and_non_action_skills_unchanged() {
        let cases = [
            IntentDecision::Chat,
            IntentDecision::SkillWeather { location: None },
            IntentDecision::SkillTime { location: None },
            IntentDecision::SkillScreenshot { filename: None },
            IntentDecision::SkillCalculator { expression: None },
            IntentDecision::SkillDictionary { word: None },
            IntentDecision::SkillScreenOcr {
                question: None,
                filename: None,
            },
        ];
        for d in cases {
            assert_eq!(validate_intent_decision(d.clone()), d);
        }
    }

    #[test]
    fn validation_passes_valid_calendar_actions() {
        for action in ["list_today", "list_tomorrow", "list_upcoming", "create"] {
            let d = IntentDecision::SkillCalendar {
                action: Some(action.to_string()),
                title: None,
                when: None,
                days: None,
                location: None,
                calendar_name: None,
            };
            assert_eq!(
                validate_intent_decision(d.clone()),
                d,
                "expected valid calendar action '{action}' to pass"
            );
        }
    }

    #[test]
    fn validation_rejects_invalid_calendar_action() {
        let d = IntentDecision::SkillCalendar {
            action: Some("delete_event".to_string()),
            title: None,
            when: None,
            days: None,
            location: None,
            calendar_name: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_passes_valid_email_actions() {
        for action in ["list_unread", "list_inbox", "search", "triage"] {
            let d = IntentDecision::SkillEmail {
                action: Some(action.to_string()),
                query: None,
                limit: None,
                mailbox: None,
            };
            assert_eq!(validate_intent_decision(d.clone()), d);
        }
    }

    #[test]
    fn validation_rejects_invalid_email_action() {
        let d = IntentDecision::SkillEmail {
            action: Some("send".to_string()),
            query: None,
            limit: None,
            mailbox: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_passes_valid_journal_actions() {
        for action in ["add", "recall", "stats"] {
            let d = IntentDecision::SkillJournal {
                action: Some(action.to_string()),
                text: None,
                sentiment: None,
                tags: None,
                query: None,
                limit: None,
            };
            assert_eq!(validate_intent_decision(d.clone()), d);
        }
    }

    #[test]
    fn validation_rejects_invalid_journal_action() {
        let d = IntentDecision::SkillJournal {
            action: Some("delete".to_string()),
            text: None,
            sentiment: None,
            tags: None,
            query: None,
            limit: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    // --- compact format tests (short keys + short intent names) ---

    #[test]
    fn parse_compact_weather() {
        let raw = r#"{"i":"weather","c":"get","l":"Paris, France"}"#;
        let d = parse_intent(raw).must();
        assert_eq!(
            d,
            IntentDecision::SkillWeather {
                location: Some("Paris, France".to_string())
            }
        );
    }

    #[test]
    fn parse_compact_distance() {
        let raw = r#"{"i":"dist","c":"get","d":"Paris, France"}"#;
        let d = parse_intent(raw).must();
        assert_eq!(
            d,
            IntentDecision::SkillDistance {
                origin: None,
                destination: Some("Paris, France".to_string())
            }
        );
    }

    #[test]
    fn parse_compact_shopping_list() {
        let raw = r#"{"i":"shop","c":"add","p":["strawberries","salami"]}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillShoppingList {
                action,
                items,
                when,
            } => {
                assert_eq!(action.as_deref(), Some("add"));
                assert_eq!(items.as_deref(), Some("strawberries, salami"));
                assert!(when.is_none());
            }
            _ => panic!("expected SkillShoppingList"),
        }
    }

    #[test]
    fn parse_compact_message() {
        let raw = r#"{"i":"msg","c":"send","t":"my wife","v":"How are you?"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMessage {
                command,
                contact,
                message,
            } => {
                assert_eq!(command.as_deref(), Some("send"));
                assert_eq!(contact.as_deref(), Some("my wife"));
                assert_eq!(message.as_deref(), Some("How are you?"));
            }
            _ => panic!("expected SkillMessage"),
        }
    }

    #[test]
    fn parse_compact_timer() {
        let raw = r#"{"i":"timer","c":"set","v":"5 minutes","n":"pasta"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillTimer { duration, name } => {
                assert_eq!(duration.as_deref(), Some("5 minutes"));
                assert_eq!(name.as_deref(), Some("pasta"));
            }
            _ => panic!("expected SkillTimer"),
        }
    }

    #[test]
    fn parse_compact_reminder() {
        let raw = r#"{"i":"reminder","c":"add","n":"Call mom","w":"PT5M"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillReminder { title, when } => {
                assert_eq!(title.as_deref(), Some("Call mom"));
                assert_eq!(when.as_deref(), Some("PT5M"));
            }
            _ => panic!("expected SkillReminder"),
        }
    }

    #[test]
    fn parse_compact_smart_home() {
        let raw = r#"{"i":"shome","c":"on","t":"kitchen lights"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillSmartHome { target, action } => {
                assert_eq!(target.as_deref(), Some("kitchen lights"));
                assert_eq!(action.as_deref(), Some("on"));
            }
            _ => panic!("expected SkillSmartHome"),
        }
    }

    #[test]
    fn parse_compact_media() {
        let raw = r#"{"i":"media","c":"resume"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillMedia { action, target } => {
                assert_eq!(action.as_deref(), Some("resume"));
                assert!(target.is_none());
            }
            _ => panic!("expected SkillMedia"),
        }
    }

    #[test]
    fn parse_compact_volume() {
        let raw = r#"{"i":"vol","c":"set","vl":40}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillVolume { action, level } => {
                assert_eq!(action.as_deref(), Some("set"));
                assert_eq!(*level, Some(40));
            }
            _ => panic!("expected SkillVolume"),
        }
    }

    #[test]
    fn parse_compact_app_switcher() {
        let raw = r#"{"i":"aswitch","c":"next"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillAppSwitcher { action, target } => {
                assert_eq!(action.as_deref(), Some("next"));
                assert!(target.is_none());
            }
            _ => panic!("expected SkillAppSwitcher"),
        }
    }

    #[test]
    fn parse_compact_fuel() {
        let raw = r#"{"i":"fuel","c":"get","fcc":"GB","fr":"london","ft":"diesel"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillFuelPriceLookup {
                country_code,
                region,
                fuel_type,
            } => {
                assert_eq!(country_code.as_deref(), Some("GB"));
                assert_eq!(region.as_deref(), Some("london"));
                assert_eq!(fuel_type.as_deref(), Some("diesel"));
            }
            _ => panic!("expected SkillFuelPriceLookup"),
        }
    }

    #[test]
    fn parse_intent_hotel_long_form() {
        let raw = r#"{"intent":"skill_hotel","hotel_intent_kind":"set_room_temperature","hotel_slots":{"celsius":22}}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHotel { intent_kind, slots } => {
                assert_eq!(intent_kind.as_deref(), Some("set_room_temperature"));
                let s = slots.as_ref().must_some("slots present");
                assert_eq!(s["celsius"], serde_json::json!(22));
            }
            _ => panic!("expected SkillHotel"),
        }
    }

    #[test]
    fn parse_intent_hotel_compact() {
        let raw = r#"{"i":"hotel","c":"request","hik":"book_taxi","hsl":{"destination":"airport","time":"16:00"}}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHotel { intent_kind, slots } => {
                assert_eq!(intent_kind.as_deref(), Some("book_taxi"));
                let s = slots.as_ref().must_some("slots present");
                assert_eq!(s["destination"], serde_json::json!("airport"));
                assert_eq!(s["time"], serde_json::json!("16:00"));
            }
            _ => panic!("expected SkillHotel"),
        }
    }

    #[test]
    fn parse_intent_hotel_without_slots() {
        let raw = r#"{"i":"hotel","c":"request","hik":"request_late_checkout"}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHotel { intent_kind, slots } => {
                assert_eq!(intent_kind.as_deref(), Some("request_late_checkout"));
                assert!(slots.is_none());
            }
            _ => panic!("expected SkillHotel"),
        }
    }

    #[test]
    fn validation_passes_canonical_hotel_intent_kinds() {
        for kind in [
            "set_room_temperature",
            "order_room_service",
            "set_wake_up_call",
            "book_taxi",
            "language_help",
        ] {
            let d = IntentDecision::SkillHotel {
                intent_kind: Some(kind.to_string()),
                slots: None,
            };
            assert_eq!(
                validate_intent_decision(d.clone()),
                d,
                "expected hotel intent_kind '{kind}' to pass validation"
            );
        }
    }

    #[test]
    fn validation_rejects_unknown_hotel_intent_kind() {
        let d = IntentDecision::SkillHotel {
            intent_kind: Some("teleport_user".to_string()),
            slots: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn validation_rejects_hotel_without_intent_kind() {
        let d = IntentDecision::SkillHotel {
            intent_kind: None,
            slots: None,
        };
        assert_eq!(validate_intent_decision(d), IntentDecision::Chat);
    }

    #[test]
    fn parse_compact_holiday() {
        let raw = r#"{"i":"holiday","c":"get","n":"easter","hcc":"DE","hy":2026}"#;
        let d = parse_intent(raw).must();
        match &d {
            IntentDecision::SkillHolidayLookup {
                name,
                country_code,
                year,
                ..
            } => {
                assert_eq!(name.as_deref(), Some("easter"));
                assert_eq!(country_code.as_deref(), Some("DE"));
                assert_eq!(*year, Some(2026));
            }
            _ => panic!("expected SkillHolidayLookup"),
        }
    }
}
