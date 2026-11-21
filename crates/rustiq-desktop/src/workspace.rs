use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

pub type Id = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Collection,
    Folder,
    Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: Id,
    pub parent: Option<Id>,
    pub kind: Kind,
    pub name: String,
    pub method: String,
    pub url: String,
    pub expanded: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub entries: Vec<Entry>,
    #[serde(default)]
    next_id: Id,
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

    pub fn load(path: &Path) -> Result<Self, String> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.to_string()),
        };
        let workspace: Self = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        // Reject invalid references, duplicate IDs and cycles before rendering the tree.
        for entry in &workspace.entries {
            if workspace
                .entries
                .iter()
                .filter(|e| e.id == entry.id)
                .count()
                != 1
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
                let node = workspace.entry(id).ok_or("Workspace parent is missing")?;
                if node.kind == Kind::Request {
                    return Err("Requests cannot contain entries".into());
                }
                parent = node.parent;
            }
        }
        Ok(workspace)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, data).map_err(|e| e.to_string())?;
        fs::rename(temporary, path).map_err(|e| e.to_string())
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

    #[test]
    fn persistence_round_trip() {
        let path = std::env::temp_dir().join(format!("rustiq-test-{}.json", std::process::id()));
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let request = workspace
            .create(Kind::Request, Some(collection), "Users")
            .unwrap();
        workspace.entry_mut(request).unwrap().url = "https://example.com/users".into();
        workspace.save(&path).unwrap();
        let loaded = Workspace::load(&path).unwrap();
        assert_eq!(
            loaded.entry(request).unwrap().url,
            "https://example.com/users"
        );
        fs::write(&path, b"invalid json").unwrap();
        assert!(Workspace::load(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
