use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectedModel {
    pub(crate) id: String,
    pub(crate) context_length: Option<u64>,
}

impl SelectedModel {
    pub(crate) fn new(id: String, context_length: Option<u64>) -> Self {
        Self { id, context_length }
    }
}

impl fmt::Display for SelectedModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.id.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::SelectedModel;

    #[test]
    fn displays_only_the_model_id() {
        let model = SelectedModel::new("model-a".to_string(), Some(32_768));

        assert_eq!(model.to_string(), "model-a");
    }
}
