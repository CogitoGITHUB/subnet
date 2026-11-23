use crate::module::ALL_MODULES;
use serde::de::{
    self, Deserializer, Error, IntoDeserializer, Visitor,
    value::{Error as ValueError, MapDeserializer, SeqDeserializer},
};
use std::{cmp::Ordering, fmt};
use toml::Value;

/// A `toml::Value` that borrows its contents instead of owning them.
#[derive(Debug, Clone, Copy)]
pub enum ValueRef<'a> {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(&'a str),
    Datetime(&'a toml::value::Datetime),
    Array(&'a [Value]),
    Table(&'a toml::map::Map<String, Value>),
}

impl<'de> From<&'de Value> for ValueRef<'de> {
    fn from(value: &'de Value) -> Self {
        match value {
            Value::Boolean(b) => Self::Boolean(*b),
            Value::Integer(i) => Self::Integer(*i),
            Value::Float(f) => Self::Float(*f),
            Value::String(s) => Self::String(s),
            Value::Array(a) => Self::Array(a),
            Value::Table(t) => Self::Table(t),
            Value::Datetime(d) => Self::Datetime(d),
        }
    }
}

impl<'de> From<&'de toml::Table> for ValueRef<'de> {
    fn from(value: &'de toml::Table) -> Self {
        Self::Table(value)
    }
}

/// A helper struct for deserializing a TOML value references with serde.
/// This also prints a warning and suggestions if a key is unknown.
#[derive(Debug)]
pub struct ValueDeserializer<'de> {
    value: ValueRef<'de>,
    info: Option<StructInfo>,
    current_key: Option<&'de str>,
    error_on_ignored: bool,
}

/// When deserializing a struct, this struct stores information about the struct.
#[derive(Debug, Clone, Copy)]
struct StructInfo {
    fields: &'static [&'static str],
    name: &'static str,
}

impl<'de> ValueDeserializer<'de> {
    pub fn new<T: Into<ValueRef<'de>>>(value: T) -> Self {
        Self {
            value: value.into(),
            info: None,
            current_key: None,
            error_on_ignored: true,
        }
    }

    fn with_info(
        value: ValueRef<'de>,
        info: Option<StructInfo>,
        current_key: &'de str,
        ignored: bool,
    ) -> Self {
        Self {
            value,
            info,
            current_key: Some(current_key),
            error_on_ignored: ignored,
        }
    }

    pub fn with_allow_unknown_keys(self) -> Self {
        Self {
            error_on_ignored: false,
            ..self
        }
    }
}

impl ValueDeserializer<'_> {
    /// Prettify an error message by adding the current key and struct name to it.
    fn error<T: fmt::Display>(&self, msg: T) -> ValueError {
        match (self.current_key, self.info) {
            (Some(key), Some(StructInfo { name, .. })) => {
                // Prettify name of struct
                let display_name = name.strip_suffix("Config").unwrap_or(name);
                ValueError::custom(format!("Error in '{display_name}' at '{key}': {msg}"))
            }
            // Handling other cases leads to duplicates in the error message.
            _ => ValueError::custom(msg),
        }
    }

    /// Return the fitting `de::Unexpected` type description for the given value.
    /// For use with `Error::invalid_type`.
    fn serde_unexpected(&self) -> de::Unexpected<'_> {
        match self.value {
            ValueRef::Boolean(b) => de::Unexpected::Bool(b),
            ValueRef::Integer(i) => de::Unexpected::Signed(i),
            ValueRef::Float(f) => de::Unexpected::Float(f),
            ValueRef::String(s) => de::Unexpected::Str(s),
            ValueRef::Array(_v) => de::Unexpected::Other("array"),
            ValueRef::Table(_v) => de::Unexpected::Other("table"),
            ValueRef::Datetime(_v) => de::Unexpected::Other("datetime"),
        }
    }
}

impl<'de> IntoDeserializer<'de> for ValueDeserializer<'de> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = ValueError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            ValueRef::Boolean(b) => visitor.visit_bool(b),
            ValueRef::Integer(i) => visitor.visit_i64(i),
            ValueRef::Float(f) => visitor.visit_f64(f),
            ValueRef::String(s) => visitor.visit_borrowed_str(s),
            ValueRef::Array(a) => {
                let seq = SeqDeserializer::new(a.iter().map(ValueDeserializer::new));
                seq.deserialize_seq(visitor)
            }
            ValueRef::Table(t) => {
                let map = MapDeserializer::new(t.iter().map(|(k, v)| {
                    (
                        k.as_str(),
                        ValueDeserializer::with_info(
                            v.into(),
                            self.info,
                            k.as_str(),
                            self.error_on_ignored,
                        ),
                    )
                }));
                map.deserialize_map(visitor)
            }
            ValueRef::Datetime(d) => visitor.visit_string(d.to_string()),
        }
        .map_err(|e| self.error(e))
    }

    // Save a reference to the struct fields and name for later use in error messages.
    fn deserialize_struct<V>(
        mut self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.info = Some(StructInfo { fields, name });
        self.deserialize_any(visitor)
    }

    // Always `Some` because TOML doesn't have a null type.
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    // Handle ignored Values. (Values at unknown keys in TOML)
    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self
            .info
            .filter(|StructInfo { name, .. }| name == &"StarshipRootConfig")
            .and(self.current_key)
            .is_some_and(|key| ALL_MODULES.contains(&key) || key == "custom" || key == "env_var")
        {
            return visitor.visit_none();
        }

        if !self.error_on_ignored {
            return visitor.visit_none();
        }

        let did_you_mean = match (self.current_key, self.info) {
            (Some(key), Some(StructInfo { fields, .. })) => fields
                .iter()
                .filter_map(|field| {
                    let score = strsim::jaro_winkler(key, field);
                    (score > 0.8).then_some((score, field))
                })
                .max_by(|(score_a, _field_a), (score_b, _field_b)| {
                    score_a.partial_cmp(score_b).unwrap_or(Ordering::Equal)
                }),
            _ => None,
        };
        let did_you_mean = did_you_mean
            .map(|(_score, field)| format!(" (Did you mean '{field}'?)"))
            .unwrap_or_default();

        Err(self.error(format!("Unknown key{did_you_mean}")))
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            // de::Value::StrDeserializer implements de::EnumAccess, so we can just use it.
            ValueRef::String(s) => visitor.visit_enum(s.into_deserializer()),
            _ => Err(Self::Error::invalid_type(
                self.serde_unexpected(),
                &"string",
            )),
        }
    }

    // Handle most deserialization cases by deferring to `deserialize_any`.
    serde::forward_to_deserialize_any! {
        bool u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 char str string unit seq
        bytes byte_buf map unit_struct tuple_struct tuple identifier
    }
}

