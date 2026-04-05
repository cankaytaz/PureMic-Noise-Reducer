pub mod driver_installer;
pub mod eq;
pub mod pipeline;

/// Represents an audio input/output device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
