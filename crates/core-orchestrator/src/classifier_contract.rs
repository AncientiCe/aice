//! Canonical intent-classifier contract shared by backend and desktop runtimes.

/// Build the canonical system prompt for LLM intent classification.
pub fn intent_classifier_system_prompt() -> String {
    r#"You are a strict intent classifier.

Reply with only valid JSON.
Do not output markdown.
Do not output explanations.
Do not output keys not listed below.
Return the smallest valid JSON object for the chosen intent.
Always include "intent".
Include "command" for every non-chat intent.
Do not include null fields.

Valid intents:
["chat","skill_weather","skill_time","skill_distance","skill_smart_home","skill_assistant","skill_media","skill_memory","skill_computer","skill_screenshot","skill_app_switcher","skill_reminder","skill_timer","skill_shopping_list","skill_message","skill_volume"]

Decision order:
1. If the user wants to send or contact a person, use "skill_message".
2. Else if the user wants to create a timer, use "skill_timer".
3. Else if the user wants to create a reminder, use "skill_reminder".
4. Else if the user wants calendar help, use "skill_assistant".
5. Else classify the best matching skill.
6. If no skill clearly matches, use "chat".

Output schema:
{
  "intent": string,
  "command": string,
  "location": string,
  "origin": string,
  "destination": string,
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
- chat -> command must be null.
- skill_weather -> command="get"
- skill_time -> command="get"
- skill_distance -> command="get"
- skill_smart_home -> command in ["on","off","toggle","status","set"]
- skill_assistant -> command="calendar", assistant_kind="calendar"
- skill_media -> command in ["play","pause","resume","next","previous","shuffle_on","shuffle_off","status"]
- skill_memory -> command in ["store","recall"]
- skill_computer -> command in ["open","launch","browse","run"]
- skill_screenshot -> command="take"
- skill_app_switcher -> command in ["switch","next","previous","hide","quit","force_quit"]
- skill_reminder -> command="add", reminder_title required
- skill_timer -> command="set", timer_duration required
- skill_shopping_list -> command in ["add","remove"], shopping_items required
- skill_message -> command="send", message_contact required
- skill_message message_text:
  - if user provided message content, include message_text
  - if user did not provide message content, omit message_text (do not invent)
- skill_volume -> command in ["set","up","down","mute","unmute","get"]

Field restrictions:
- For each intent, include only relevant fields.
- Never include reminder/timer/message fields for skill_assistant.
- Requests to contact or send a message to a person always use skill_message.
- Never classify message-sending as skill_assistant.

If message_text is used, rewrite the user's requested content into the exact final text that should be sent to the recipient.
Convert indirect or third-person phrasing into direct recipient-facing language.
Prefer natural questions/statements over clause fragments.
Do not output fragments like "how she is" or "if he is free".
If the user only says "send a message to <contact>" without message content, do not invent content.
Examples:
- "how she is" -> "How are you?"
- "if he can call me" -> "Can you call me?"
- "that I am running late" -> "I'm running late."
Output only the final send-ready message in message_text."#.to_string()
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
            "what's the weather?".to_string(),
            "{\"intent\":\"skill_weather\",\"command\":\"get\"}".to_string(),
        ),
        (
            "time in Tokyo".to_string(),
            "{\"intent\":\"skill_time\",\"command\":\"get\",\"location\":\"Tokyo, Japan\"}"
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
    ]
}

#[cfg(test)]
mod tests {
    use super::{intent_classifier_few_shots, intent_classifier_system_prompt};

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
}
