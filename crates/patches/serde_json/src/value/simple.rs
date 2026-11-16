use super::Value;
use crate::error::Error;
use alloc::borrow::ToOwned;
use alloc::string::String;
use serde::ser::{Error as _, Impossible, Serialize};

pub(super) struct Simple;

fn unsupported() -> Error {
    Error::custom("")
}

impl serde::Serializer for Simple {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = Impossible<Value, Error>;
    type SerializeTuple = Impossible<Value, Error>;
    type SerializeTupleStruct = Impossible<Value, Error>;
    type SerializeTupleVariant = Impossible<Value, Error>;
    type SerializeMap = Impossible<Value, Error>;
    type SerializeStruct = Impossible<Value, Error>;
    type SerializeStructVariant = Impossible<Value, Error>;

    fn serialize_bool(self, value: bool) -> Result<Value, Error> {
        Ok(Value::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_i16(self, value: i16) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_i32(self, value: i32) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_i64(self, value: i64) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_u8(self, value: u8) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_u16(self, value: u16) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_u32(self, value: u32) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_u64(self, value: u64) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_f32(self, _value: f32) -> Result<Value, Error> {
        Err(unsupported())
    }

    fn serialize_f64(self, value: f64) -> Result<Value, Error> {
        Ok(Value::from(value))
    }

    fn serialize_char(self, value: char) -> Result<Value, Error> {
        let mut text = String::new();
        text.push(value);
        Ok(Value::String(text))
    }

    fn serialize_str(self, value: &str) -> Result<Value, Error> {
        Ok(Value::String(value.to_owned()))
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Value, Error> {
        Err(unsupported())
    }

    fn serialize_none(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Value, Error> {
        Ok(Value::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Value, Error> {
        Err(unsupported())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(unsupported())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Err(unsupported())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(unsupported())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(unsupported())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(unsupported())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Err(unsupported())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(unsupported())
    }
}
