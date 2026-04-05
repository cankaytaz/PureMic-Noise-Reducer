pub mod audio;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
