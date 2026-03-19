//! LLM-based intent classifier: calls LLM with classification prompt and parses JSON.

use core_orchestrator::{parse_intent, IntentClassifier, IntentDecision, LlmStream};
use futures::StreamExt;

const CLASSIFICATION_SYSTEM_PROMPT: &str = "You are an intent classifier. Reply with only a JSON object, no other text. \
Use \"intent\": one of \"chat\", \"skill_weather\", \"skill_time\", \"skill_distance\", \"skill_smart_home\", \"skill_assistant\", \"skill_media\", \"skill_memory\", \"skill_computer\", \"skill_reminder\", \"skill_timer\", \"skill_shopping_list\", \"skill_message\". \
When the user names a place, output an EXACT location string (city + country), e.g. \"Rome, Italy\". \
Weather: ANY weather question is skill_weather. Set \"location\" only when they name a place. \
Time: \"what time is it\", \"time in X\". Omit \"location\" unless they name a place. \
Distance: \"origin\" and/or \"destination\" as full location strings. One place = \"destination\" only. \
Smart home: lights, thermostat, locks, scenes, devices. Use \"skill_smart_home\"; optional \"smart_home_target\" (e.g. \"living room\"), \"smart_home_action\" (e.g. \"turn off\"). \
Assistant: calendar and assistant-only actions. Use \"skill_assistant\"; optional \"assistant_kind\": \"calendar\". \
Media: play, pause, volume, multi-room, \"play X in Y\". Use \"skill_media\"; optional \"media_action\", \"media_target\". \
Memory: remember/recall facts, \"remember that\", \"what did I say about X\". Use \"skill_memory\"; optional \"memory_query\", \"memory_store\": true when storing. \
Computer: open app, browser, run script, file operations. Use \"skill_computer\"; optional \"computer_action\", \"computer_target\". \
Reminder: add/create a reminder. Use \"skill_reminder\"; \"reminder_title\" = what to be reminded about; optional \"reminder_when\" = date-time in ISO 8601 (e.g. \"2026-03-20T17:00\") when the user specifies a time. Omit \"reminder_when\" if no time given. \
Timer: set/start a timer. Use \"skill_timer\"; \"timer_duration\" = duration string (e.g. \"5 minutes\", \"1 hour 30 minutes\"); optional \"timer_name\" = label the user gives the timer. \
Shopping list: add/remove items from a shopping list. Use \"skill_shopping_list\"; \"shopping_action\": \"add\" or \"remove\"; \"shopping_items\" = comma-separated items to add/remove; optional \"shopping_when\" = target date (e.g. \"today\", \"tomorrow\", \"saturday\", \"2026-03-20\"). Default when not specified: \"today\". \
Message: send a message to a contact via iMessage. Use \"skill_message\"; \"message_contact\" = target contact phrase (e.g. \"my wife\", \"John Doe\"); \"message_text\" = what to send (e.g. \"How are you?\"). Never classify message-sending as \"skill_assistant\". \
Do NOT use \"skill_assistant\" for sending iMessages. \
Examples: {\"intent\": \"skill_reminder\", \"reminder_title\": \"Call mom\", \"reminder_when\": \"2026-03-20T17:00\"}; {\"intent\": \"skill_timer\", \"timer_duration\": \"5 minutes\"}; {\"intent\": \"skill_shopping_list\", \"shopping_action\": \"add\", \"shopping_items\": \"strawberries, salami, celery\"}; {\"intent\": \"skill_message\", \"message_contact\": \"my wife\", \"message_text\": \"How are you?\"}; {\"intent\": \"skill_message\", \"message_contact\": \"my wife\", \"message_text\": \"how she's doing\"}.";

/// Intent classifier that uses an LLM to classify user text; parses JSON response.
pub struct LlmIntentClassifier<'a, L> {
    pub llm: &'a L,
    pub system_prompt: String,
}

impl<'a, L> LlmIntentClassifier<'a, L> {
    pub fn new(llm: &'a L) -> Self {
        Self {
            llm,
            system_prompt: CLASSIFICATION_SYSTEM_PROMPT.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl<L> IntentClassifier for LlmIntentClassifier<'_, L>
where
    L: LlmStream + Send + Sync,
{
    async fn classify(
        &self,
        user_text: &str,
    ) -> Result<IntentDecision, Box<dyn std::error::Error + Send + Sync>> {
        let user_message = format!(
            "Classify this user request. Reply with only the JSON object.\nUser request: \"{}\"",
            user_text.trim()
        );
        let mut stream = self
            .llm
            .chat_stream(&user_message, &[], Some(self.system_prompt.as_str()))
            .await?;
        let mut raw = String::new();
        while let Some(token) = stream.next().await {
            raw.push_str(&token);
        }
        let decision = parse_intent(raw.trim())?;
        Ok(decision)
    }
}
