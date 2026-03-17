//! Core skills: umbrella crate that links to skill crates and re-exports their API.
//!
//! Consumers (e.g. desktop-runner) depend only on `core-skills` to use any registered skill.

pub use skill_assistant::{
    AssistantItem, AssistantResult, AssistantSkill, AssistantSkillError, MockAssistantSkill,
};
pub use skill_computer::{ComputerResult, ComputerSkill, ComputerSkillError, MockComputerSkill};
pub use skill_distance::{
    DistanceResult, DistanceSkill, DistanceSkillError, MockDistanceSkill, OpenMeteoDistanceSkill,
};
pub use skill_media::{MacOsMusicSkill, MediaResult, MediaSkill, MediaSkillError, MockMediaSkill};
pub use skill_memory::{
    MemoryFact, MemoryResult, MemorySkill, MemorySkillError, MockMemorySkill, SqliteMemorySkill,
};
pub use skill_smart_home::{
    DeviceState, HueSmartHomeSkill, MockSmartHomeSkill, SmartHomeResult, SmartHomeSkill,
    SmartHomeSkillError,
};
pub use skill_time::{MockTimeSkill, OpenMeteoTimeSkill, TimeResult, TimeSkill, TimeSkillError};
pub use skill_weather::{
    MockWeatherSkill, OpenMeteoWeatherSkill, ResolvedLocation, WeatherResult, WeatherSkill,
    WeatherSkillError,
};
