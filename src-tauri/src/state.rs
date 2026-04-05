pub struct AppState {
    pub selected_mic_id: Option<String>,
    pub selected_output_id: Option<String>,
    pub is_active: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            selected_mic_id: None,
            selected_output_id: None,
            is_active: false,
        }
    }
}
