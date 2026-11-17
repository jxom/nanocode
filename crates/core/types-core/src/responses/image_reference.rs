use serde::{Deserialize, Serialize};

#[must_use]
pub fn valid_image_file_id(file_id: &str) -> bool {
    !file_id.is_empty()
        && file_id.len() <= 512
        && file_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[doc(hidden)]
pub fn serialize_file_id<S: serde::Serializer>(
    file_id: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if !valid_image_file_id(file_id) {
        return Err(serde::ser::Error::custom("invalid image file_id"));
    }
    serializer.serialize_str(file_id)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ImageReference {
    Inline {
        image_url: String,
    },

    File {
        #[serde(serialize_with = "serialize_file_id")]
        file_id: String,
    },
}
#[cfg(target_family = "wasm")]
impl<'de> Deserialize<'de> for ImageReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            image_url: Option<String>,
            file_id: Option<String>,
        }
        match Wire::deserialize(deserializer)? {
            Wire {
                image_url: Some(image_url),
                file_id: None,
            } => Ok(Self::Inline { image_url }),
            Wire {
                image_url: None,
                file_id: Some(file_id),
            } if valid_image_file_id(&file_id) => Ok(Self::File { file_id }),
            Wire {
                image_url: None,
                file_id: Some(_),
            } => Err(serde::de::Error::custom("invalid image file_id")),
            Wire { .. } => Err(serde::de::Error::custom(
                "data did not match any variant of untagged enum Wire",
            )),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
impl<'de> Deserialize<'de> for ImageReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Inline(Inline),
            File(File),
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Inline {
            image_url: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct File {
            file_id: String,
        }
        match Wire::deserialize(deserializer)? {
            Wire::Inline(Inline { image_url }) => Ok(Self::Inline { image_url }),
            Wire::File(File { file_id }) if valid_image_file_id(&file_id) => {
                Ok(Self::File { file_id })
            }
            Wire::File(_) => Err(serde::de::Error::custom("invalid image file_id")),
        }
    }
}
