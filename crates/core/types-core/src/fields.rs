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

    #[inline(never)]
    fn remove(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name)
    }

    #[inline(never)]
    pub(crate) fn take<T: DeserializeOwned>(&mut self, name: &'static str) -> Result<T, Error> {
        match self.remove(name) {
            Some(value) => T::deserialize(value),
            None => Err(Error::missing_field(name)),
        }
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn rest<T: DeserializeOwned>(self) -> Result<T, Error> {
        T::deserialize(Value::Object(self.0))
    }

    #[cfg(target_family = "wasm")]
    #[inline(never)]
    pub(crate) fn or_default<T: DeserializeOwned + Default>(
        &mut self,
        name: &'static str,
    ) -> Result<T, Error> {
        self.remove(name)
            .map_or_else(|| Ok(T::default()), T::deserialize)
    }

    #[inline(never)]
    pub(crate) fn optional<T: DeserializeOwned>(
        &mut self,
        name: &'static str,
    ) -> Result<Option<T>, Error> {
        self.remove(name).map_or(Ok(None), Option::<T>::deserialize)
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

#[cfg(target_family = "wasm")]
macro_rules! tagged {
    ($name:ident, |$fields:ident, $tag:ident| $body:expr) => {
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
                (|| -> Result<Self, serde_json::Error> {
                    let mut $fields = $crate::fields::Fields::new(
                        value,
                        concat!("internally tagged enum ", stringify!($name)),
                    )?;
                    let $tag = $fields.tag()?;
                    $body
                })()
                .map_err(serde::de::Error::custom)
            }
        }
    };
}

#[cfg(target_family = "wasm")]
pub(crate) fn unknown(tag: &str, expected: &'static [&'static str]) -> serde_json::Error {
    serde::de::Error::unknown_variant(tag, expected)
}
