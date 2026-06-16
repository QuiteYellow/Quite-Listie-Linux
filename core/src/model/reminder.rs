use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReminderRepeatUnit {
    Day,
    Week,
    Month,
    Year,
    Weekdays,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReminderRepeatMode {
    Fixed,
    AfterComplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderRepeatRule {
    pub unit: ReminderRepeatUnit,
    pub interval: u32,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ReminderRepeatRule {
    pub fn daily() -> Self {
        Self { unit: ReminderRepeatUnit::Day, interval: 1, extra: Default::default() }
    }
    pub fn weekly() -> Self {
        Self { unit: ReminderRepeatUnit::Week, interval: 1, extra: Default::default() }
    }
    pub fn biweekly() -> Self {
        Self { unit: ReminderRepeatUnit::Week, interval: 2, extra: Default::default() }
    }
    pub fn monthly() -> Self {
        Self { unit: ReminderRepeatUnit::Month, interval: 1, extra: Default::default() }
    }
    pub fn yearly() -> Self {
        Self { unit: ReminderRepeatUnit::Year, interval: 1, extra: Default::default() }
    }
    pub fn weekdays() -> Self {
        Self { unit: ReminderRepeatUnit::Weekdays, interval: 1, extra: Default::default() }
    }
}
