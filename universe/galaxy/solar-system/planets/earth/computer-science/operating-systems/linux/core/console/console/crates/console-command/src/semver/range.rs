use console_protocol::{ShellError, Span, Value};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemverRangeValue {
    pub requirement: semver::VersionReq,
}

#[typetag::serde]
impl console_protocol::CustomValue for SemverRangeValue {
    fn clone_value(&self, span: Span) -> Value {
        Value::custom(Box::new(self.clone()), span)
    }

    fn type_name(&self) -> String {
        "semver-range".to_string()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        Ok(Value::string(self.requirement.to_string(), span))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }

    /// Requirements have no order, so only equal requirements compare, as `Equal`.
    fn partial_cmp(&self, other: &Value) -> Option<Ordering> {
        let other = other
            .as_custom_value()
            .ok()?
            .as_any()
            .downcast_ref::<Self>()?;
        (self.requirement == other.requirement).then_some(Ordering::Equal)
    }

    fn hash_value(&self, mut state: &mut dyn Hasher) {
        self.requirement.hash(&mut state);
    }
}

impl SemverRangeValue {
    pub fn new(requirement: semver::VersionReq) -> Self {
        Self { requirement }
    }
}

