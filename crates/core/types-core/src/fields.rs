use serde::{
    Deserialize,
    de::{DeserializeOwned, Error as _, Unexpected},
};
use serde_json::{Error, Map, Value};

pub(crate) struct Fields(Map<String, Value>);

impl Fields {
    pub(crate) fn new(value: Value, expected: &'static str) -> Result<Self, Error> {
        match value {
            Value::Object(object) => Ok(Self(object)),
            other => Err(Error::invalid_type(unexpected(&other), &expected)),
        }
    }

    pub(crate) fn tag(&mut self) -> Result<String, Error> {
        match self.0.remove("type") {
            Some(Value::String(tag)) => Ok(tag),
            Some(other) => Err(Error::invalid_type(
                unexpected(&other),
                &"variant identifier",
            )),
            None => Err(Error::missing_field("type")),
        }
    }

    pub(crate) fn deny(&self, expected: &'static [&'static str]) -> Result<(), Error> {
        match self.0.keys().find(|key| !expected.contains(&key.as_str())) {
            Some(key) => Err(Error::unknown_field(key, expected)),
            None => Ok(()),
        }
    }

    pub(crate) fn take<T: DeserializeOwned>(&mut self, name: &'static str) -> Result<T, Error> {
        T::deserialize(
            self.0
                .remove(name)
                .ok_or_else(|| Error::missing_field(name))?,
        )
    }

    pub(crate) fn optional<T: DeserializeOwned>(
        &mut self,
        name: &'static str,
    ) -> Result<Option<T>, Error> {
        self.0
            .remove(name)
            .map_or(Ok(None), Option::<T>::deserialize)
    }
}

fn unexpected(value: &Value) -> Unexpected<'_> {
    match value {
        Value::Null => Unexpected::Unit,
        Value::Bool(value) => Unexpected::Bool(*value),
        Value::Number(number) => match (number.as_u64(), number.as_i64()) {
            (Some(value), _) => Unexpected::Unsigned(value),
            (None, Some(value)) => Unexpected::Signed(value),
            (None, None) => Unexpected::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(value) => Unexpected::Str(value),
        Value::Array(_) => Unexpected::Seq,
        Value::Object(_) => Unexpected::Map,
    }
}
