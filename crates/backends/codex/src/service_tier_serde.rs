use std::fmt;

use crate::client::pricing::ServiceTier;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, IntoDeserializer, Visitor},
    ser::SerializeMap,
};

pub(crate) fn serialize<S: Serializer>(
    tier: &ServiceTier,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    tier.serialize(serializer)
}

pub(crate) fn serialize_admission<S: Serializer>(
    tier: &ServiceTier,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(1))?;
    match tier {
        ServiceTier::Ultrafast => map.serialize_entry("service_tier", tier)?,
        _ => map.serialize_entry("fast_mode", &!matches!(tier, ServiceTier::Standard))?,
    }
    map.end()
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ServiceTier, D::Error> {
    struct TierVisitor;

    impl Visitor<'_> for TierVisitor {
        type Value = ServiceTier;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("one service_tier or legacy fast_mode field")
        }

        fn visit_bool<E: Error>(self, fast: bool) -> Result<Self::Value, E> {
            Ok(ServiceTier::from_fast_mode(fast))
        }

        fn visit_str<E: Error>(self, tier: &str) -> Result<Self::Value, E> {
            ServiceTier::deserialize(tier.into_deserializer())
        }
    }

    deserializer.deserialize_any(TierVisitor)
}
