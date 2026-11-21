use serde::{Deserialize, Serialize};

pub type Id = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Collection,
    Folder,
    Request,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Persistence {
    #[default]
    Encrypted,
    SessionOnly,
}

// Runtime data must only be serialized through the encrypted storage layer.
#[derive(Clone, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
pub struct Entry {
    pub id: Id,
    pub parent: Option<Id>,
    pub kind: Kind,
    pub name: String,
    pub method: String,
    pub url: String,
    pub expanded: bool,
    #[serde(default)]
    pub request: rustiq_http::RequestOptions,
    #[serde(default)]
    pub persistence: Persistence,
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub(super) next_id: Id,
}

impl Workspace {
    pub fn entry(&self, id: Id) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn entry_mut(&mut self, id: Id) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.id == id)
    }

    pub fn create(&mut self, kind: Kind, parent: Option<Id>, name: &str) -> Option<Id> {
        let name = name.trim();
        if name.is_empty()
            || match kind {
                Kind::Collection => parent.is_some(),
                _ => !parent
                    .is_some_and(|id| self.entry(id).is_some_and(|e| e.kind != Kind::Request)),
            }
        {
            return None;
        }
        let id = self
            .entries
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap_or(0)
            .max(self.next_id)
            .checked_add(1)?;
        self.next_id = id;
        if let Some(parent) = parent.and_then(|id| self.entry_mut(id)) {
            parent.expanded = true;
        }
        self.entries.push(Entry {
            id,
            parent,
            kind,
            name: name.into(),
            method: "GET".into(),
            url: String::new(),
            expanded: true,
            request: rustiq_http::RequestOptions::default(),
            persistence: Persistence::Encrypted,
        });
        Some(id)
    }

    pub fn rename(&mut self, id: Id, name: &str) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        if let Some(entry) = self.entry_mut(id) {
            entry.name = name.trim().into();
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, id: Id) {
        let mut ids = vec![id];
        let mut cursor = 0;
        while cursor < ids.len() {
            let parent = ids[cursor];
            ids.extend(
                self.entries
                    .iter()
                    .filter(|e| e.parent == Some(parent))
                    .map(|e| e.id),
            );
            cursor += 1;
        }
        self.entries.retain(|e| !ids.contains(&e.id));
    }

    pub fn validate(&self) -> Result<(), String> {
        // Reject invalid references, duplicate IDs and cycles before rendering the tree.
        for entry in &self.entries {
            if self.entries.iter().filter(|e| e.id == entry.id).count() != 1
                || entry.name.trim().is_empty()
                || entry.method.parse::<reqwest::Method>().is_err()
                || (entry.kind == Kind::Collection) != entry.parent.is_none()
            {
                return Err("Invalid workspace entry".into());
            }
            let mut parent = entry.parent;
            let mut ancestors = vec![entry.id];
            while let Some(id) = parent {
                if ancestors.contains(&id) {
                    return Err("Workspace contains a cycle".into());
                }
                ancestors.push(id);
                let node = self.entry(id).ok_or("Workspace parent is missing")?;
                if node.kind == Kind::Request {
                    return Err("Requests cannot contain entries".into());
                }
                parent = node.parent;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("persistence", &self.persistence)
            .finish_non_exhaustive()
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.url.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_and_names() {
        let mut workspace = Workspace::default();
        assert!(workspace.create(Kind::Request, None, "Request").is_none());
        let collection = workspace.create(Kind::Collection, None, " API ").unwrap();
        let folder = workspace
            .create(Kind::Folder, Some(collection), "Users")
            .unwrap();
        let nested = workspace
            .create(Kind::Folder, Some(folder), "Admin")
            .unwrap();
        let request = workspace
            .create(Kind::Request, Some(nested), "List")
            .unwrap();
        assert!(
            workspace
                .create(Kind::Folder, Some(request), "Invalid")
                .is_none()
        );
        assert!(!workspace.rename(request, "  "));
        assert!(workspace.rename(request, " List users "));
        assert_eq!(workspace.entry(request).unwrap().name, "List users");
        workspace.remove(folder);
        assert_eq!(workspace.entries.len(), 1);
        assert_eq!(workspace.entries[0].name, "API");
    }
}
