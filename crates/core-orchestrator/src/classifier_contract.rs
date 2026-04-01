//! Canonical intent-classifier contract shared by backend and desktop runtimes.

const ALL_CLASSIFIER_SKILLS: [&str; 20] = [
    "skill_weather",
    "skill_time",
    "skill_distance",
    "skill_sports_live",
    "skill_holiday_lookup",
    "skill_fuel_price_lookup",
    "skill_horoscope_daily",
    "skill_news_headlines",
    "skill_smart_home",
    "skill_assistant",
    "skill_media",
    "skill_memory",
    "skill_computer",
    "skill_screenshot",
    "skill_app_switcher",
    "skill_reminder",
    "skill_timer",
    "skill_shopping_list",
    "skill_message",
    "skill_volume",
];

fn has_skill(available_skills: &[&str], skill: &str) -> bool {
    available_skills
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case(skill))
}

fn render_decision_order(available_skills: &[&str]) -> String {
    let mut steps = Vec::new();
    if has_skill(available_skills, "skill_message") {
        steps.push("If the user wants to send or contact a person, use \"skill_message\".");
    }
    if has_skill(available_skills, "skill_timer") {
        steps.push("Else if the user wants to create a timer, use \"skill_timer\".");
    }
    if has_skill(available_skills, "skill_reminder") {
        steps.push("Else if the user wants to create a reminder, use \"skill_reminder\".");
    }
    if has_skill(available_skills, "skill_assistant") {
        steps.push("Else if the user wants calendar help, use \"skill_assistant\".");
    }
    if has_skill(available_skills, "skill_media") {
        steps.push(
            "Else if the user wants to control music, audio playback, or media (play, pause, skip track, shuffle), use \"skill_media\".",
        );
    }
    steps.push("Else classify the best matching skill from the valid intents list.");
    steps.push("If no skill clearly matches, use \"chat\".");
    let mut output = String::new();
    for (index, step) in steps.iter().enumerate() {
        output.push_str(&format!("{}. {step}\n", index + 1));
    }
    output
}

fn render_per_intent_rules(available_skills: &[&str]) -> String {
    let mut lines = vec!["- chat -> command must be null.".to_string()];
    if has_skill(available_skills, "skill_weather") {
        lines.push("- skill_weather -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_time") {
        lines.push("- skill_time -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_distance") {
        lines.push("- skill_distance -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_sports_live") {
        lines.push("- skill_sports_live -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_holiday_lookup") {
        lines.push("- skill_holiday_lookup -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_fuel_price_lookup") {
        lines.push("- skill_fuel_price_lookup -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_horoscope_daily") {
        lines.push("- skill_horoscope_daily -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_news_headlines") {
        lines.push("- skill_news_headlines -> command=\"get\"".to_string());
    }
    if has_skill(available_skills, "skill_smart_home") {
        lines.push(
            "- skill_smart_home -> command in [\"on\",\"off\",\"toggle\",\"status\",\"set\"]"
                .to_string(),
        );
    }
    if has_skill(available_skills, "skill_assistant") {
        lines.push(
            "- skill_assistant -> command=\"calendar\", assistant_kind=\"calendar\"".to_string(),
        );
    }
    if has_skill(available_skills, "skill_media") {
        lines.push("- skill_media -> command in [\"play\",\"pause\",\"resume\",\"next\",\"previous\",\"shuffle_on\",\"shuffle_off\",\"status\"]".to_string());
        lines.push("  - Use \"resume\" for unpause/continue/resume requests. Use \"play\" only for starting new playback.".to_string());
    }
    if has_skill(available_skills, "skill_memory") {
        lines.push("- skill_memory -> command in [\"store\",\"recall\"]".to_string());
    }
    if has_skill(available_skills, "skill_computer") {
        lines.push(
            "- skill_computer -> command in [\"open\",\"launch\",\"browse\",\"run\"]".to_string(),
        );
    }
    if has_skill(available_skills, "skill_screenshot") {
        lines.push("- skill_screenshot -> command=\"take\"".to_string());
    }
    if has_skill(available_skills, "skill_app_switcher") {
        lines.push("- skill_app_switcher -> command in [\"switch\",\"next\",\"previous\",\"hide\",\"quit\",\"force_quit\"]".to_string());
    }
    if has_skill(available_skills, "skill_reminder") {
        lines.push("- skill_reminder -> command=\"add\", reminder_title required".to_string());
    }
    if has_skill(available_skills, "skill_timer") {
        lines.push("- skill_timer -> command=\"set\", timer_duration required".to_string());
    }
    if has_skill(available_skills, "skill_shopping_list") {
        lines.push(
            "- skill_shopping_list -> command in [\"add\",\"remove\"], shopping_items required"
                .to_string(),
        );
    }
    if has_skill(available_skills, "skill_message") {
        lines.push("- skill_message -> command=\"send\", message_contact required".to_string());
        lines.push("- skill_message message_text:".to_string());
        lines.push("  - if user provided message content, include message_text".to_string());
        lines.push(
            "  - if user did not provide message content, omit message_text (do not invent)"
                .to_string(),
        );
    }
    if has_skill(available_skills, "skill_volume") {
        lines.push(
            "- skill_volume -> command in [\"set\",\"up\",\"down\",\"mute\",\"unmute\",\"get\"]"
                .to_string(),
        );
    }
    format!("{}\n", lines.join("\n"))
}

/// Build the canonical system prompt for LLM intent classification.
pub fn intent_classifier_system_prompt() -> String {
    intent_classifier_system_prompt_for_skills(&ALL_CLASSIFIER_SKILLS)
}

/// Build the canonical system prompt for a set of available skills.
pub fn intent_classifier_system_prompt_for_skills(available_skills: &[&str]) -> String {
    let mut valid_intents = vec!["chat".to_string()];
    for skill in ALL_CLASSIFIER_SKILLS {
        if has_skill(available_skills, skill) {
            valid_intents.push(skill.to_string());
        }
    }
    let valid_intents_json = format!(
        "[{}]",
        valid_intents
            .iter()
            .map(|intent| format!("\"{intent}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let decision_order = render_decision_order(available_skills);
    let per_intent_rules = render_per_intent_rules(available_skills);
    let include_assistant_restriction = has_skill(available_skills, "skill_assistant");
    let include_message_restriction = has_skill(available_skills, "skill_message");
    let include_message_rewrite_block = has_skill(available_skills, "skill_message");
    let mut prompt = r#"You are a strict intent classifier.

Reply with only valid JSON.
Do not output markdown.
Do not output explanations.
Do not output keys not listed below.
Return the smallest valid JSON object for the chosen intent.
Always include "intent".
Include "command" for every non-chat intent.
Do not include null fields.
Think about what the user wants, then pick the most specific skill from the available list.

Valid intents:
"#
    .to_string();
    prompt.push_str(&valid_intents_json);
    prompt.push_str(
        r#"

Decision order:
"#,
    );
    prompt.push_str(&decision_order);
    prompt.push_str(
        r#"

Output schema:
{
  "intent": string,
  "command": string,
  "location": string,
  "origin": string,
  "destination": string,
  "sports_query": string,
  "sports_date": string,
  "holiday_name": string,
  "holiday_date": string,
  "holiday_country_code": string,
  "holiday_region_code": string,
  "holiday_year": number,
  "fuel_country_code": string,
  "fuel_region": string,
  "fuel_type": string,
  "horoscope_sign": string,
  "horoscope_date": string,
  "news_topic": string,
  "news_country_code": string,
  "news_limit": number,
  "smart_home_target": string,
  "smart_home_action": string,
  "media_target": string,
  "media_action": string,
  "memory_query": string,
  "memory_store": boolean,
  "computer_target": string,
  "computer_action": string,
  "screenshot_filename": string,
  "app_switcher_target": string,
  "app_switcher_action": string,
  "reminder_title": string,
  "reminder_when": string,
  "timer_duration": string,
  "timer_name": string,
  "shopping_items": [string],
  "shopping_when": string,
  "message_contact": string,
  "message_text": string,
  "volume_action": string,
  "volume_level": number,
  "assistant_kind": string
}

Per-intent rules:
"#,
    );
    prompt.push_str(&per_intent_rules);
    prompt.push_str(
        r#"

Field restrictions:
- For each intent, include only relevant fields.
"#,
    );
    if include_assistant_restriction {
        prompt.push_str("- Never include reminder/timer/message fields for skill_assistant.\n");
    }
    if include_message_restriction {
        prompt.push_str(
            "- Requests to contact or send a message to a person always use skill_message.\n",
        );
        if include_assistant_restriction {
            prompt.push_str("- Never classify message-sending as skill_assistant.\n");
        }
    }
    let include_media = has_skill(available_skills, "skill_media");
    let include_smart_home = has_skill(available_skills, "skill_smart_home");
    let include_app_switcher = has_skill(available_skills, "skill_app_switcher");
    if include_media && include_smart_home {
        prompt.push_str(
            "- skill_media is for music, audio playback, tracks, queues, and shuffle. skill_smart_home is for physical devices: lights, switches, scenes, brightness, climate. Never route playback or music requests to skill_smart_home.\n",
        );
    }
    if include_media && include_app_switcher {
        prompt.push_str(
            "- skill_media and skill_app_switcher both use \"next\"/\"previous\" commands, but skill_media controls the audio/music player while skill_app_switcher controls the focused application or window.\n",
        );
    }
    if include_message_rewrite_block {
        prompt.push_str(
            r#"

If message_text is used, rewrite the user's requested content into the exact final text that should be sent to the recipient.
Convert indirect or third-person phrasing into direct recipient-facing language.
Prefer natural questions/statements over clause fragments.
Do not output fragments like "how she is" or "if he is free".
If the user only says "send a message to <contact>" without message content, do not invent content.
Examples:
- "how she is" -> "How are you?"
- "if he can call me" -> "Can you call me?"
- "that I am running late" -> "I'm running late."
Output only the final send-ready message in message_text."#,
        );
    }
    prompt
}

/// Canonical few-shot examples for intent classification.
pub fn intent_classifier_few_shots() -> Vec<(String, String)> {
    vec![
        (
            "how far is Paris?".to_string(),
            "{\"intent\":\"skill_distance\",\"command\":\"get\",\"destination\":\"Paris, France\"}"
                .to_string(),
        ),
        (
            "what's the weather in Paris?".to_string(),
            "{\"intent\":\"skill_weather\",\"command\":\"get\",\"location\":\"Paris, France\"}"
                .to_string(),
        ),
        (
            "sports update for lakers vs celtics today".to_string(),
            "{\"intent\":\"skill_sports_live\",\"command\":\"get\",\"sports_query\":\"lakers vs celtics\"}".to_string(),
        ),
        (
            "is there a holiday in Germany on 2026-10-03?".to_string(),
            "{\"intent\":\"skill_holiday_lookup\",\"command\":\"get\",\"holiday_country_code\":\"DE\",\"holiday_date\":\"2026-10-03\"}".to_string(),
        ),
        (
            "fuel price in the uk for diesel".to_string(),
            "{\"intent\":\"skill_fuel_price_lookup\",\"command\":\"get\",\"fuel_country_code\":\"GB\",\"fuel_type\":\"diesel\"}".to_string(),
        ),
        (
            "today horoscope for aries".to_string(),
            "{\"intent\":\"skill_horoscope_daily\",\"command\":\"get\",\"horoscope_sign\":\"aries\"}".to_string(),
        ),
        (
            "top technology headlines in the us".to_string(),
            "{\"intent\":\"skill_news_headlines\",\"command\":\"get\",\"news_topic\":\"technology\",\"news_country_code\":\"US\"}".to_string(),
        ),
        (
            "what's the weather?".to_string(),
            "{\"intent\":\"skill_weather\",\"command\":\"get\"}".to_string(),
        ),
        (
            "time in Tokyo".to_string(),
            "{\"intent\":\"skill_time\",\"command\":\"get\",\"location\":\"Tokyo, Japan\"}"
                .to_string(),
        ),
        (
            "what's the time?".to_string(),
            "{\"intent\":\"skill_time\",\"command\":\"get\"}".to_string(),
        ),
        (
            "what time is it?".to_string(),
            "{\"intent\":\"skill_time\",\"command\":\"get\"}".to_string(),
        ),
        (
            "what time is it in London?".to_string(),
            "{\"intent\":\"skill_time\",\"command\":\"get\",\"location\":\"London, UK\"}"
                .to_string(),
        ),
        (
            "set a timer for 5 minutes".to_string(),
            "{\"intent\":\"skill_timer\",\"command\":\"set\",\"timer_duration\":\"5 minutes\"}"
                .to_string(),
        ),
        (
            "remind me in 5 minutes to ask my wife how she is".to_string(),
            "{\"intent\":\"skill_reminder\",\"command\":\"add\",\"reminder_title\":\"Ask my wife how she is\",\"reminder_when\":\"PT5M\"}".to_string(),
        ),
        (
            "send a message to John saying running late".to_string(),
            "{\"intent\":\"skill_message\",\"command\":\"send\",\"message_contact\":\"John\",\"message_text\":\"running late\"}".to_string(),
        ),
        (
            "send a message to my wife.".to_string(),
            "{\"intent\":\"skill_message\",\"command\":\"send\",\"message_contact\":\"my wife\"}"
                .to_string(),
        ),
        (
            "ask my wife how she is".to_string(),
            "{\"intent\":\"skill_message\",\"command\":\"send\",\"message_contact\":\"my wife\",\"message_text\":\"How are you?\"}".to_string(),
        ),
        (
            "next song".to_string(),
            "{\"intent\":\"skill_media\",\"command\":\"next\"}".to_string(),
        ),
        (
            "pause the music".to_string(),
            "{\"intent\":\"skill_media\",\"command\":\"pause\"}".to_string(),
        ),
        (
            "unpause.".to_string(),
            "{\"intent\":\"skill_media\",\"command\":\"resume\"}".to_string(),
        ),
        (
            "resume the music".to_string(),
            "{\"intent\":\"skill_media\",\"command\":\"resume\"}".to_string(),
        ),
        (
            "turn on the kitchen lights".to_string(),
            "{\"intent\":\"skill_smart_home\",\"command\":\"on\",\"smart_home_target\":\"kitchen lights\"}".to_string(),
        ),
        (
            "switch to the next app".to_string(),
            "{\"intent\":\"skill_app_switcher\",\"command\":\"next\"}".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        intent_classifier_few_shots, intent_classifier_system_prompt,
        intent_classifier_system_prompt_for_skills,
    };

    #[test]
    fn prompt_lists_message_and_assistant_disambiguation() {
        let prompt = intent_classifier_system_prompt();
        assert!(prompt.contains("Valid intents:"));
        assert!(prompt.contains("Decision order:"));
        assert!(prompt.contains("If the user wants to send or contact a person"));
        assert!(prompt.contains("Never classify message-sending as skill_assistant"));
        assert!(prompt.contains("Never include reminder/timer/message fields for skill_assistant"));
        assert!(prompt.contains("Convert indirect or third-person phrasing"));
    }

    #[test]
    fn prompt_enforces_command_field_for_skills() {
        let prompt = intent_classifier_system_prompt();
        assert!(prompt.contains("\"command\": string"));
        assert!(prompt.contains(
            "- skill_volume -> command in [\"set\",\"up\",\"down\",\"mute\",\"unmute\",\"get\"]"
        ));
    }

    #[test]
    fn prompt_can_be_scoped_to_available_skills() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_time", "skill_timer"]);
        assert!(prompt.contains("Valid intents:"));
        assert!(prompt.contains("\"chat\""));
        assert!(prompt.contains("\"skill_time\""));
        assert!(prompt.contains("\"skill_timer\""));
        assert!(!prompt.contains("\"skill_weather\""));
        assert!(prompt.contains("- skill_time -> command=\"get\""));
        assert!(prompt.contains("- skill_timer -> command=\"set\", timer_duration required"));
        assert!(!prompt.contains("- skill_weather -> command=\"get\""));
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
        assert!(prompt.contains("\"skill_sports_live\""));
        assert!(prompt.contains("\"skill_holiday_lookup\""));
        assert!(prompt.contains("\"skill_fuel_price_lookup\""));
        assert!(prompt.contains("\"skill_horoscope_daily\""));
        assert!(prompt.contains("\"skill_news_headlines\""));
        assert!(prompt.contains("- skill_sports_live -> command=\"get\""));
        assert!(prompt.contains("- skill_holiday_lookup -> command=\"get\""));
        assert!(prompt.contains("- skill_fuel_price_lookup -> command=\"get\""));
        assert!(prompt.contains("- skill_horoscope_daily -> command=\"get\""));
        assert!(prompt.contains("- skill_news_headlines -> command=\"get\""));
    }

    #[test]
    fn few_shots_cover_message_send_contract() {
        let few_shots = intent_classifier_few_shots();
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("send a message to John saying running late")
                && a.contains("\"intent\":\"skill_message\"")
                && a.contains("\"command\":\"send\"")
                && a.contains("\"message_contact\":\"John\"")
                && a.contains("\"message_text\":\"running late\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("ask my wife how she is")
                && a.contains("\"intent\":\"skill_message\"")
                && a.contains("\"command\":\"send\"")
                && a.contains("\"message_contact\":\"my wife\"")
                && a.contains("\"message_text\":\"How are you?\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("send a message to my wife.")
                && a.contains("\"intent\":\"skill_message\"")
                && a.contains("\"command\":\"send\"")
                && a.contains("\"message_contact\":\"my wife\"")
                && !a.contains("\"message_text\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("remind me in 5 minutes to ask my wife how she is")
                && a.contains("\"intent\":\"skill_reminder\"")
                && a.contains("\"command\":\"add\"")
                && a.contains("\"reminder_when\":\"PT5M\"")
        }));
    }

    #[test]
    fn few_shots_cover_local_time_queries() {
        let few_shots = intent_classifier_few_shots();
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("what's the time?")
                && a.contains("\"intent\":\"skill_time\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("what time is it?")
                && a.contains("\"intent\":\"skill_time\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("what time is it in London?")
                && a.contains("\"intent\":\"skill_time\"")
                && a.contains("\"command\":\"get\"")
                && a.contains("\"location\":\"London, UK\"")
        }));
    }

    #[test]
    fn prompt_includes_media_vs_smart_home_restriction_when_both_present() {
        let prompt =
            intent_classifier_system_prompt_for_skills(&["skill_media", "skill_smart_home"]);
        assert!(
            prompt.contains("skill_media is for music, audio playback"),
            "expected media/smart-home domain separation in prompt"
        );
        assert!(
            prompt.contains("Never route playback or music requests to skill_smart_home"),
            "expected explicit prohibition on smart-home for playback"
        );
    }

    #[test]
    fn prompt_omits_media_smart_home_restriction_when_only_one_present() {
        let prompt_media_only = intent_classifier_system_prompt_for_skills(&["skill_media"]);
        assert!(!prompt_media_only
            .contains("Never route playback or music requests to skill_smart_home"));

        let prompt_smart_home_only =
            intent_classifier_system_prompt_for_skills(&["skill_smart_home"]);
        assert!(!prompt_smart_home_only.contains("skill_media is for music"));
    }

    #[test]
    fn prompt_includes_media_vs_app_switcher_restriction_when_both_present() {
        let prompt =
            intent_classifier_system_prompt_for_skills(&["skill_media", "skill_app_switcher"]);
        assert!(
            prompt.contains("skill_media controls the audio/music player"),
            "expected media/app-switcher next/previous disambiguation in prompt"
        );
        assert!(
            prompt.contains("skill_app_switcher controls the focused application"),
            "expected app-switcher description in disambiguation rule"
        );
    }

    #[test]
    fn prompt_includes_media_decision_order_step() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_media"]);
        assert!(
            prompt.contains("control music, audio playback, or media"),
            "expected media step in decision order"
        );
    }

    #[test]
    fn few_shots_cover_media_domain_boundary() {
        let few_shots = intent_classifier_few_shots();

        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("next song")
                    && a.contains("\"intent\":\"skill_media\"")
                    && a.contains("\"command\":\"next\"")
            }),
            "expected 'next song' -> skill_media next few-shot"
        );
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("pause the music")
                    && a.contains("\"intent\":\"skill_media\"")
                    && a.contains("\"command\":\"pause\"")
            }),
            "expected 'pause the music' -> skill_media pause few-shot"
        );
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("unpause")
                    && a.contains("\"intent\":\"skill_media\"")
                    && a.contains("\"command\":\"resume\"")
            }),
            "expected 'unpause' -> skill_media resume few-shot"
        );
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("resume the music")
                    && a.contains("\"intent\":\"skill_media\"")
                    && a.contains("\"command\":\"resume\"")
            }),
            "expected 'resume the music' -> skill_media resume few-shot"
        );
    }

    #[test]
    fn prompt_distinguishes_play_from_resume_for_media() {
        let prompt = intent_classifier_system_prompt_for_skills(&["skill_media"]);
        assert!(
            prompt.contains("Use \"resume\" for unpause/continue/resume requests"),
            "expected play-vs-resume disambiguation in media per-intent rules"
        );
    }

    #[test]
    fn few_shots_cover_smart_home_domain_boundary() {
        let few_shots = intent_classifier_few_shots();
        assert!(
            few_shots.iter().any(|(u, a)| {
                u.contains("turn on the kitchen lights")
                    && a.contains("\"intent\":\"skill_smart_home\"")
                    && a.contains("\"command\":\"on\"")
                    && a.contains("\"smart_home_target\":\"kitchen lights\"")
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
                    && a.contains("\"intent\":\"skill_app_switcher\"")
                    && a.contains("\"command\":\"next\"")
            }),
            "expected app-switcher next few-shot"
        );
    }

    #[test]
    fn few_shots_cover_new_core_common_skills() {
        let few_shots = intent_classifier_few_shots();
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("sports")
                && a.contains("\"intent\":\"skill_sports_live\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("holiday")
                && a.contains("\"intent\":\"skill_holiday_lookup\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("fuel")
                && a.contains("\"intent\":\"skill_fuel_price_lookup\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("horoscope")
                && a.contains("\"intent\":\"skill_horoscope_daily\"")
                && a.contains("\"command\":\"get\"")
        }));
        assert!(few_shots.iter().any(|(u, a)| {
            u.contains("headline")
                && a.contains("\"intent\":\"skill_news_headlines\"")
                && a.contains("\"command\":\"get\"")
        }));
    }
}
