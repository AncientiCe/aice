use crate::FacilitatorError;

/// Which property product this process is running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pack {
    Hotels,
    Care,
    Ward,
}

impl Pack {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotels => "hotels",
            Self::Care => "care",
            Self::Ward => "ward",
        }
    }

    pub fn parse(value: &str) -> Result<Self, FacilitatorError> {
        match value.trim() {
            "hotels" => Ok(Self::Hotels),
            "care" => Ok(Self::Care),
            "ward" => Ok(Self::Ward),
            other => Err(FacilitatorError::UnknownPack(other.to_string())),
        }
    }

    pub fn desk_title(self) -> &'static str {
        match self {
            Self::Hotels => "aice-hotels desk",
            Self::Care => "aice-care desk",
            Self::Ward => "aice-ward desk",
        }
    }

    pub fn default_bind(self) -> &'static str {
        match self {
            Self::Hotels => "127.0.0.1:8791",
            Self::Care => "127.0.0.1:8792",
            Self::Ward => "127.0.0.1:8793",
        }
    }

    pub fn tools(self) -> &'static [&'static str] {
        match self {
            Self::Hotels => HOTEL_TOOLS,
            Self::Care => CARE_TOOLS,
            Self::Ward => crate::WARD_TOOLS,
        }
    }

    pub fn always_escalates(self, tool: &str) -> bool {
        matches!(self, Self::Care) && CARE_ALWAYS_ESCALATE.contains(&tool)
    }
}

/// Hotel and serviced-apartment requests. The first 25 match the voice classifier.
const HOTEL_TOOLS: &[&str] = &[
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
    "request_access_code",
    "request_maintenance",
];

const CARE_TOOLS: &[&str] = &[
    "request_bathroom_help",
    "request_drink",
    "report_pain",
    "request_visitor",
    "set_lights",
    "report_distress",
    "report_fall",
];

const CARE_ALWAYS_ESCALATE: &[&str] = &["report_distress", "report_fall"];
