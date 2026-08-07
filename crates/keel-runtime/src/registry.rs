use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Open-ended descriptor so the registry can grow with the Keel vocabulary
/// without turning every new document kind into a runtime enum change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentDescriptor {
    pub kind: String,
    pub id: String,
    pub version: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ComponentRegistry {
    components: BTreeMap<(String, String), ComponentDescriptor>,
}

impl ComponentRegistry {
    pub fn register(&mut self, descriptor: ComponentDescriptor) {
        self.components
            .insert((descriptor.kind.clone(), descriptor.id.clone()), descriptor);
    }

    pub fn list(&self) -> Vec<ComponentDescriptor> {
        self.components.values().cloned().collect()
    }

    pub fn contains(&self, kind: &str, id: &str) -> bool {
        self.components
            .contains_key(&(kind.to_string(), id.to_string()))
    }
}
